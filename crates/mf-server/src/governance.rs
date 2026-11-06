//! Validation decisions, run jobs and append-only audit log.
//! Two implementations: SQLite (local file) and PostgreSQL (shared across instances).

use crate::Role;
use mf_runtime::pg::{PgConn, PgPool};
use mf_runtime::report::Decision;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::path::Path;

/// Inventory record of a model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ModelRecord {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub purpose: String,
    /// Risk tier: 1 (high) to 3 (low).
    pub tier: i64,
    /// `development`, `in_validation`, `approved`, `in_use`, `retired`.
    pub status: String,
    /// Next review date (YYYY-MM-DD).
    pub next_review: Option<String>,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub updated_by: String,
}

pub const MODEL_STATUSES: &[&str] = &["development", "in_validation", "approved", "in_use", "retired"];

/// A validation finding: a weakness of a model raised by a validator, remediated by the model
/// owner and closed by a validator.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Finding {
    pub id: i64,
    pub model: String,
    /// The run the finding was raised on, if any.
    pub pipeline: Option<String>,
    pub run_id: Option<i64>,
    pub step: Option<String>,
    pub title: String,
    pub description: String,
    /// `critical`, `high`, `medium` or `low`.
    pub severity: String,
    /// `open`, `remediated`, `closed` or `risk_accepted`.
    pub status: String,
    /// Responsible for the remediation (by default the model owner).
    pub owner: String,
    pub raised_by: String,
    /// Remediation deadline (YYYY-MM-DD).
    pub due_date: String,
    pub remediation: String,
    /// Run that evidences the remediation (`pipeline#id`), replayed identically when submitted.
    pub evidence: Option<String>,
    pub remediated_by: Option<String>,
    pub closed_by: Option<String>,
    pub closed_at: Option<i64>,
    /// When the overdue finding was escalated (once).
    pub escalated_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub const SEVERITIES: &[&str] = &["critical", "high", "medium", "low"];
pub const FINDING_STATUSES: &[&str] = &["open", "remediated", "closed", "risk_accepted"];

const FINDING_COLUMNS: &str =
    "id, model, pipeline, run_id, step, title, description, severity, status, owner, raised_by,
    due_date, remediation, evidence, remediated_by, closed_by, closed_at, escalated_at, created_at, updated_at";
const FINDING_TYPES: &[C] = &[
    C::I,
    C::S,
    C::OS,
    C::OI,
    C::OS,
    C::S,
    C::S,
    C::S,
    C::S,
    C::S,
    C::S,
    C::S,
    C::S,
    C::OS,
    C::OS,
    C::OS,
    C::OI,
    C::OI,
    C::I,
    C::I,
];

fn finding_from(r: Vec<Value>) -> Finding {
    let s = |i: usize| r[i].as_str().unwrap_or_default().to_string();
    let os = |i: usize| r[i].as_str().map(String::from);
    Finding {
        id: r[0].as_i64().unwrap_or_default(),
        model: s(1),
        pipeline: os(2),
        run_id: r[3].as_i64(),
        step: os(4),
        title: s(5),
        description: s(6),
        severity: s(7),
        status: s(8),
        owner: s(9),
        raised_by: s(10),
        due_date: s(11),
        remediation: s(12),
        evidence: os(13),
        remediated_by: os(14),
        closed_by: os(15),
        closed_at: r[16].as_i64(),
        escalated_at: r[17].as_i64(),
        created_at: r[18].as_i64().unwrap_or_default(),
        updated_at: r[19].as_i64().unwrap_or_default(),
    }
}

pub enum Gov {
    Sqlite(Connection),
    Postgres(Box<RefCell<PgConn>>),
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Instance identifier (host name, i.e. the pod on Kubernetes).
pub fn instance_id() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "local".into())
}

/// PostgreSQL governance schema (idempotent). The audit log rejects UPDATE, DELETE and TRUNCATE.
pub const PG_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS decisions (
    id BIGSERIAL PRIMARY KEY, pipeline TEXT NOT NULL, run_id BIGINT NOT NULL,
    decision TEXT NOT NULL, comment TEXT NOT NULL, username TEXT NOT NULL, created_at BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS audit (
    id BIGSERIAL PRIMARY KEY, ts BIGINT NOT NULL, username TEXT NOT NULL, role TEXT NOT NULL,
    action TEXT NOT NULL, target TEXT NOT NULL, outcome TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS jobs (
    id BIGSERIAL PRIMARY KEY, pipeline TEXT NOT NULL, username TEXT NOT NULL, status TEXT NOT NULL,
    run_id BIGINT, error TEXT, created_at BIGINT NOT NULL, started_at BIGINT, finished_at BIGINT,
    instance TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS models (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, owner TEXT NOT NULL, purpose TEXT NOT NULL,
    tier BIGINT NOT NULL, status TEXT NOT NULL, next_review TEXT,
    created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL, updated_by TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS schedules (pipeline TEXT PRIMARY KEY, last_slot BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS findings (
    id BIGSERIAL PRIMARY KEY, model TEXT NOT NULL, pipeline TEXT, run_id BIGINT, step TEXT,
    title TEXT NOT NULL, description TEXT NOT NULL, severity TEXT NOT NULL, status TEXT NOT NULL,
    owner TEXT NOT NULL, raised_by TEXT NOT NULL, due_date TEXT NOT NULL, remediation TEXT NOT NULL,
    evidence TEXT, remediated_by TEXT, closed_by TEXT, closed_at BIGINT, escalated_at BIGINT,
    created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS finding_events (
    id BIGSERIAL PRIMARY KEY, finding_id BIGINT NOT NULL, ts BIGINT NOT NULL, username TEXT NOT NULL,
    action TEXT NOT NULL, status TEXT NOT NULL, comment TEXT NOT NULL);
CREATE OR REPLACE FUNCTION mf_audit_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'append-only audit log'; END $$;
CREATE OR REPLACE TRIGGER audit_no_change BEFORE UPDATE OR DELETE ON audit
    FOR EACH ROW EXECUTE FUNCTION mf_audit_append_only();
CREATE OR REPLACE TRIGGER audit_no_truncate BEFORE TRUNCATE ON audit
    FOR EACH STATEMENT EXECUTE FUNCTION mf_audit_append_only();";

/// Query parameter shared by both backends.
enum P<'a> {
    I(i64),
    S(&'a str),
    OI(Option<i64>),
    OS(Option<&'a str>),
}

/// Expected type of a column being read.
#[derive(Clone, Copy)]
enum C {
    I,
    S,
    OI,
    OS,
}

fn pg_err(e: postgres::Error) -> String {
    e.as_db_error()
        .map(|d| format!("PostgreSQL: {}", d.message()))
        .unwrap_or_else(|| format!("PostgreSQL: {e}"))
}

impl Gov {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let c = Connection::open(path).map_err(|e| e.to_string())?;
        c.busy_timeout(std::time::Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS decisions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pipeline TEXT NOT NULL, run_id INTEGER NOT NULL,
                decision TEXT NOT NULL, comment TEXT NOT NULL,
                user TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL,
                user TEXT NOT NULL, role TEXT NOT NULL, action TEXT NOT NULL,
                target TEXT NOT NULL, outcome TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pipeline TEXT NOT NULL, user TEXT NOT NULL,
                status TEXT NOT NULL, run_id INTEGER, error TEXT,
                created_at INTEGER NOT NULL, started_at INTEGER, finished_at INTEGER);
             CREATE TABLE IF NOT EXISTS models (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, owner TEXT NOT NULL, purpose TEXT NOT NULL,
                tier INTEGER NOT NULL, status TEXT NOT NULL, next_review TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, updated_by TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS schedules (pipeline TEXT PRIMARY KEY, last_slot INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS findings (
                id INTEGER PRIMARY KEY AUTOINCREMENT, model TEXT NOT NULL, pipeline TEXT, run_id INTEGER, step TEXT,
                title TEXT NOT NULL, description TEXT NOT NULL, severity TEXT NOT NULL, status TEXT NOT NULL,
                owner TEXT NOT NULL, raised_by TEXT NOT NULL, due_date TEXT NOT NULL, remediation TEXT NOT NULL,
                evidence TEXT, remediated_by TEXT, closed_by TEXT, closed_at INTEGER, escalated_at INTEGER,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS finding_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT, finding_id INTEGER NOT NULL, ts INTEGER NOT NULL,
                user TEXT NOT NULL, action TEXT NOT NULL, status TEXT NOT NULL, comment TEXT NOT NULL);
             CREATE TRIGGER IF NOT EXISTS audit_no_update BEFORE UPDATE ON audit
                BEGIN SELECT RAISE(ABORT, 'append-only audit log'); END;
             CREATE TRIGGER IF NOT EXISTS audit_no_delete BEFORE DELETE ON audit
                BEGIN SELECT RAISE(ABORT, 'append-only audit log'); END;",
        )
        .map_err(|e| e.to_string())?;
        let has_instance: bool = c
            .prepare("SELECT 1 FROM pragma_table_info('jobs') WHERE name = 'instance'")
            .and_then(|mut s| s.exists([]))
            .map_err(|e| e.to_string())?;
        if !has_instance {
            c.execute_batch("ALTER TABLE jobs ADD COLUMN instance TEXT NOT NULL DEFAULT ''")
                .map_err(|e| e.to_string())?;
        }
        Ok(Gov::Sqlite(c))
    }

    pub fn postgres(pool: &PgPool) -> Result<Self, String> {
        Ok(Gov::Postgres(Box::new(RefCell::new(
            pool.get().map_err(|e| format!("PostgreSQL: {e}"))?,
        ))))
    }

    /// Adapts a query written with `$n` and `{user}` to the backend.
    fn sql(&self, q: &str) -> String {
        match self {
            Gov::Sqlite(_) => {
                let mut s = q.replace("{user}", "user");
                for i in (1..=20).rev() {
                    s = s.replace(&format!("${i}"), &format!("?{i}"));
                }
                s
            }
            Gov::Postgres(_) => q.replace("{user}", "username"),
        }
    }

    fn exec(&self, q: &str, ps: &[P]) -> Result<u64, String> {
        let q = self.sql(q);
        match self {
            Gov::Sqlite(c) => {
                let v: Vec<Box<dyn rusqlite::ToSql>> = ps.iter().map(to_sqlite).collect();
                c.execute(&q, rusqlite::params_from_iter(v.iter()))
                    .map(|n| n as u64)
                    .map_err(|e| e.to_string())
            }
            Gov::Postgres(c) => {
                let v: Vec<Box<dyn postgres::types::ToSql + Sync>> = ps.iter().map(to_pg).collect();
                let r: Vec<&(dyn postgres::types::ToSql + Sync)> = v.iter().map(|b| b.as_ref()).collect();
                c.borrow_mut().execute(&q, &r).map_err(pg_err)
            }
        }
    }

    fn insert(&self, q: &str, ps: &[P]) -> Result<i64, String> {
        match self {
            Gov::Sqlite(c) => {
                self.exec(q, ps)?;
                Ok(c.last_insert_rowid())
            }
            Gov::Postgres(c) => {
                let q = self.sql(&format!("{q} RETURNING id"));
                let v: Vec<Box<dyn postgres::types::ToSql + Sync>> = ps.iter().map(to_pg).collect();
                let r: Vec<&(dyn postgres::types::ToSql + Sync)> = v.iter().map(|b| b.as_ref()).collect();
                Ok(c.borrow_mut().query_one(&q, &r).map_err(pg_err)?.get(0))
            }
        }
    }

    fn query(&self, q: &str, ps: &[P], cols: &[C]) -> Result<Vec<Vec<Value>>, String> {
        let q = self.sql(q);
        match self {
            Gov::Sqlite(c) => {
                let v: Vec<Box<dyn rusqlite::ToSql>> = ps.iter().map(to_sqlite).collect();
                let mut st = c.prepare(&q).map_err(|e| e.to_string())?;
                let rows = st
                    .query_map(rusqlite::params_from_iter(v.iter()), |r| {
                        cols.iter()
                            .enumerate()
                            .map(|(i, k)| {
                                Ok(match k {
                                    C::I => json!(r.get::<_, i64>(i)?),
                                    C::S => json!(r.get::<_, String>(i)?),
                                    C::OI => json!(r.get::<_, Option<i64>>(i)?),
                                    C::OS => json!(r.get::<_, Option<String>>(i)?),
                                })
                            })
                            .collect::<rusqlite::Result<Vec<Value>>>()
                    })
                    .map_err(|e| e.to_string())?;
                rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
            }
            Gov::Postgres(c) => {
                let v: Vec<Box<dyn postgres::types::ToSql + Sync>> = ps.iter().map(to_pg).collect();
                let r: Vec<&(dyn postgres::types::ToSql + Sync)> = v.iter().map(|b| b.as_ref()).collect();
                let rows = c.borrow_mut().query(&q, &r).map_err(pg_err)?;
                Ok(rows
                    .iter()
                    .map(|row| {
                        cols.iter()
                            .enumerate()
                            .map(|(i, k)| match k {
                                C::I => json!(row.get::<_, i64>(i)),
                                C::S => json!(row.get::<_, String>(i)),
                                C::OI => json!(row.get::<_, Option<i64>>(i)),
                                C::OS => json!(row.get::<_, Option<String>>(i)),
                            })
                            .collect()
                    })
                    .collect())
            }
        }
    }

    pub fn create_job(&self, pipeline: &str, user: &str) -> Result<i64, String> {
        let inst = instance_id();
        self.insert(
            "INSERT INTO jobs (pipeline, {user}, status, created_at, instance) VALUES ($1, $2, 'queued', $3, $4)",
            &[P::S(pipeline), P::S(user), P::I(now()), P::S(&inst)],
        )
    }

    pub fn job_started(&self, id: i64) -> Result<(), String> {
        self.exec(
            "UPDATE jobs SET status = 'running', started_at = $2 WHERE id = $1",
            &[P::I(id), P::I(now())],
        )
        .map(|_| ())
    }

    pub fn job_finished(&self, id: i64, run_id: Option<i64>, error: Option<&str>) -> Result<(), String> {
        let status = if error.is_none() { "succeeded" } else { "failed" };
        self.exec(
            "UPDATE jobs SET status = $2, run_id = $3, error = $4, finished_at = $5 WHERE id = $1",
            &[P::I(id), P::S(status), P::OI(run_id), P::OS(error), P::I(now())],
        )
        .map(|_| ())
    }

    /// Marks jobs left running at shutdown as failed: all of them on SQLite (single instance),
    /// only this instance's on PostgreSQL (other instances keep theirs running).
    pub fn fail_interrupted_jobs(&self) -> Result<u64, String> {
        let base = "UPDATE jobs SET status = 'failed', error = 'interrupted by a server shutdown', finished_at = $1
                    WHERE status IN ('queued', 'running')";
        match self {
            Gov::Sqlite(_) => self.exec(base, &[P::I(now())]),
            Gov::Postgres(_) => {
                let inst = instance_id();
                self.exec(&format!("{base} AND instance = $2"), &[P::I(now()), P::S(&inst)])
            }
        }
    }

    pub fn job(&self, id: i64) -> Result<Option<Value>, String> {
        let rows = self.query(
            "SELECT id, pipeline, {user}, status, run_id, error, created_at, started_at, finished_at, instance FROM jobs WHERE id = $1",
            &[P::I(id)],
            &[C::I, C::S, C::S, C::S, C::OI, C::OS, C::I, C::OI, C::OI, C::S],
        )?;
        let keys = [
            "id",
            "pipeline",
            "user",
            "status",
            "run_id",
            "error",
            "created_at",
            "started_at",
            "finished_at",
            "instance",
        ];
        Ok(rows
            .into_iter()
            .next()
            .map(|r| Value::Object(keys.iter().map(|k| k.to_string()).zip(r).collect())))
    }

    pub fn create_model(&self, m: &ModelRecord, user: &str) -> Result<bool, String> {
        let t = now();
        let n = self.exec(
            "INSERT INTO models (id, name, owner, purpose, tier, status, next_review, created_at, updated_at, updated_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, $9) ON CONFLICT (id) DO NOTHING",
            &[
                P::S(&m.id),
                P::S(&m.name),
                P::S(&m.owner),
                P::S(&m.purpose),
                P::I(m.tier),
                P::S(&m.status),
                P::OS(m.next_review.as_deref()),
                P::I(t),
                P::S(user),
            ],
        )?;
        Ok(n == 1)
    }

    pub fn update_model(&self, m: &ModelRecord, user: &str) -> Result<bool, String> {
        let n = self.exec(
            "UPDATE models SET name = $2, owner = $3, purpose = $4, tier = $5, status = $6, next_review = $7,
             updated_at = $8, updated_by = $9 WHERE id = $1",
            &[
                P::S(&m.id),
                P::S(&m.name),
                P::S(&m.owner),
                P::S(&m.purpose),
                P::I(m.tier),
                P::S(&m.status),
                P::OS(m.next_review.as_deref()),
                P::I(now()),
                P::S(user),
            ],
        )?;
        Ok(n == 1)
    }

    pub fn models(&self) -> Result<Vec<ModelRecord>, String> {
        let rows = self.query(
            "SELECT id, name, owner, purpose, tier, status, next_review, updated_at, updated_by FROM models ORDER BY id",
            &[],
            &[C::S, C::S, C::S, C::S, C::I, C::S, C::OS, C::I, C::S],
        )?;
        Ok(rows
            .into_iter()
            .map(|r| ModelRecord {
                id: r[0].as_str().unwrap_or_default().into(),
                name: r[1].as_str().unwrap_or_default().into(),
                owner: r[2].as_str().unwrap_or_default().into(),
                purpose: r[3].as_str().unwrap_or_default().into(),
                tier: r[4].as_i64().unwrap_or(3),
                status: r[5].as_str().unwrap_or_default().into(),
                next_review: r[6].as_str().map(String::from),
                updated_at: r[7].as_i64().unwrap_or_default(),
                updated_by: r[8].as_str().unwrap_or_default().into(),
            })
            .collect())
    }

    /// Latest scheduled slot processed for a pipeline.
    pub fn schedule_last(&self, pipeline: &str) -> Result<Option<i64>, String> {
        let rows = self.query(
            "SELECT last_slot FROM schedules WHERE pipeline = $1",
            &[P::S(pipeline)],
            &[C::I],
        )?;
        Ok(rows.first().and_then(|r| r[0].as_i64()))
    }

    /// Claims a slot (compare-and-swap): only one instance gets it.
    pub fn schedule_claim(&self, pipeline: &str, previous: Option<i64>, slot: i64) -> Result<bool, String> {
        let n = match previous {
            None => self.exec(
                "INSERT INTO schedules (pipeline, last_slot) VALUES ($1, $2) ON CONFLICT (pipeline) DO NOTHING",
                &[P::S(pipeline), P::I(slot)],
            )?,
            Some(old) => self.exec(
                "UPDATE schedules SET last_slot = $3 WHERE pipeline = $1 AND last_slot = $2",
                &[P::S(pipeline), P::I(old), P::I(slot)],
            )?,
        };
        Ok(n == 1)
    }

    pub fn audit(&self, user: &str, role: Role, action: &str, target: &str, outcome: &str) -> Result<(), String> {
        let role = serde_json::to_value(role)
            .unwrap()
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.exec(
            "INSERT INTO audit (ts, {user}, role, action, target, outcome) VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                P::I(now()),
                P::S(user),
                P::S(&role),
                P::S(action),
                P::S(target),
                P::S(outcome),
            ],
        )
        .map(|_| ())
    }

    pub fn decide(
        &self,
        pipeline: &str,
        run_id: i64,
        decision: &str,
        comment: &str,
        user: &str,
    ) -> Result<i64, String> {
        self.insert(
            "INSERT INTO decisions (pipeline, run_id, decision, comment, {user}, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
            &[P::S(pipeline), P::I(run_id), P::S(decision), P::S(comment), P::S(user), P::I(now())],
        )
    }

    pub fn decisions(&self, pipeline: &str, run_id: i64) -> Result<Vec<Decision>, String> {
        let rows = self.query(
            "SELECT {user}, decision, comment, created_at FROM decisions WHERE pipeline = $1 AND run_id = $2 ORDER BY id",
            &[P::S(pipeline), P::I(run_id)],
            &[C::S, C::S, C::S, C::I],
        )?;
        Ok(rows
            .into_iter()
            .map(|r| Decision {
                user: r[0].as_str().unwrap_or_default().into(),
                decision: r[1].as_str().unwrap_or_default().into(),
                comment: r[2].as_str().unwrap_or_default().into(),
                created_at: r[3].as_i64().unwrap_or_default(),
            })
            .collect())
    }

    /// Records a new open finding and its first event; returns its id.
    pub fn create_finding(&self, f: &Finding, user: &str) -> Result<i64, String> {
        let t = now();
        let id = self.insert(
            "INSERT INTO findings (model, pipeline, run_id, step, title, description, severity, status, owner,
             raised_by, due_date, remediation, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'open', $8, $9, $10, '', $11, $11)",
            &[
                P::S(&f.model),
                P::OS(f.pipeline.as_deref()),
                P::OI(f.run_id),
                P::OS(f.step.as_deref()),
                P::S(&f.title),
                P::S(&f.description),
                P::S(&f.severity),
                P::S(&f.owner),
                P::S(user),
                P::S(&f.due_date),
                P::I(t),
            ],
        )?;
        self.finding_event(id, user, "raise", "open", &f.description)?;
        Ok(id)
    }

    pub fn finding(&self, id: i64) -> Result<Option<Finding>, String> {
        let rows = self.query(
            &format!("SELECT {FINDING_COLUMNS} FROM findings WHERE id = $1"),
            &[P::I(id)],
            FINDING_TYPES,
        )?;
        Ok(rows.into_iter().next().map(finding_from))
    }

    pub fn findings(&self) -> Result<Vec<Finding>, String> {
        let rows = self.query(
            &format!("SELECT {FINDING_COLUMNS} FROM findings ORDER BY id"),
            &[],
            FINDING_TYPES,
        )?;
        Ok(rows.into_iter().map(finding_from).collect())
    }

    /// Saves the mutable fields of a finding.
    pub fn update_finding(&self, f: &Finding) -> Result<(), String> {
        self.exec(
            "UPDATE findings SET status = $2, remediation = $3, due_date = $4, evidence = $5, remediated_by = $6,
             closed_by = $7, closed_at = $8, escalated_at = $9, updated_at = $10 WHERE id = $1",
            &[
                P::I(f.id),
                P::S(&f.status),
                P::S(&f.remediation),
                P::S(&f.due_date),
                P::OS(f.evidence.as_deref()),
                P::OS(f.remediated_by.as_deref()),
                P::OS(f.closed_by.as_deref()),
                P::OI(f.closed_at),
                P::OI(f.escalated_at),
                P::I(now()),
            ],
        )
        .map(|_| ())
    }

    pub fn finding_event(&self, id: i64, user: &str, action: &str, status: &str, comment: &str) -> Result<(), String> {
        self.exec(
            "INSERT INTO finding_events (finding_id, ts, {user}, action, status, comment) VALUES ($1, $2, $3, $4, $5, $6)",
            &[P::I(id), P::I(now()), P::S(user), P::S(action), P::S(status), P::S(comment)],
        )
        .map(|_| ())
    }

    pub fn finding_events(&self, id: i64) -> Result<Vec<Value>, String> {
        let rows = self.query(
            "SELECT ts, {user}, action, status, comment FROM finding_events WHERE finding_id = $1 ORDER BY id",
            &[P::I(id)],
            &[C::I, C::S, C::S, C::S, C::S],
        )?;
        let keys = ["ts", "user", "action", "status", "comment"];
        Ok(rows
            .into_iter()
            .map(|r| Value::Object(keys.iter().map(|k| k.to_string()).zip(r).collect()))
            .collect())
    }

    pub fn audit_log(&self, limit: u32) -> Result<Vec<Value>, String> {
        let rows = self.query(
            "SELECT id, ts, {user}, role, action, target, outcome FROM audit ORDER BY id DESC LIMIT $1",
            &[P::I(limit as i64)],
            &[C::I, C::I, C::S, C::S, C::S, C::S, C::S],
        )?;
        let keys = ["id", "ts", "user", "role", "action", "target", "outcome"];
        Ok(rows
            .into_iter()
            .map(|r| Value::Object(keys.iter().map(|k| k.to_string()).zip(r).collect()))
            .collect())
    }
}

fn to_sqlite(p: &P) -> Box<dyn rusqlite::ToSql> {
    match p {
        P::I(v) => Box::new(*v),
        P::S(v) => Box::new(v.to_string()),
        P::OI(v) => Box::new(*v),
        P::OS(v) => Box::new(v.map(str::to_string)),
    }
}

fn to_pg(p: &P) -> Box<dyn postgres::types::ToSql + Sync> {
    match p {
        P::I(v) => Box::new(*v),
        P::S(v) => Box::new(v.to_string()),
        P::OI(v) => Box::new(*v),
        P::OS(v) => Box::new(v.map(str::to_string)),
    }
}
