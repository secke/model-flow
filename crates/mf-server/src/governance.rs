//! Décisions de validation, travaux d'exécution et journal d'audit en ajout seul.
//! Deux implémentations : SQLite (fichier local) et PostgreSQL (partagée entre instances).

use crate::Role;
use mf_runtime::pg::{PgConn, PgPool};
use mf_runtime::report::Decision;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::path::Path;

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

/// Identifiant de l'instance (nom d'hôte, c'est-à-dire le pod sous Kubernetes).
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

/// Schéma PostgreSQL de la gouvernance (idempotent). Le journal d'audit refuse UPDATE, DELETE et TRUNCATE.
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
CREATE OR REPLACE FUNCTION mf_audit_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'journal d''audit en ajout seul'; END $$;
CREATE OR REPLACE TRIGGER audit_no_change BEFORE UPDATE OR DELETE ON audit
    FOR EACH ROW EXECUTE FUNCTION mf_audit_append_only();
CREATE OR REPLACE TRIGGER audit_no_truncate BEFORE TRUNCATE ON audit
    FOR EACH STATEMENT EXECUTE FUNCTION mf_audit_append_only();";

/// Paramètre de requête commun aux deux moteurs.
enum P<'a> {
    I(i64),
    S(&'a str),
    OI(Option<i64>),
    OS(Option<&'a str>),
}

/// Type attendu d'une colonne lue.
#[derive(Clone, Copy)]
enum C {
    I,
    S,
    OI,
    OS,
}

fn pg_err(e: postgres::Error) -> String {
    e.as_db_error()
        .map(|d| format!("PostgreSQL : {}", d.message()))
        .unwrap_or_else(|| format!("PostgreSQL : {e}"))
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
             CREATE TRIGGER IF NOT EXISTS audit_no_update BEFORE UPDATE ON audit
                BEGIN SELECT RAISE(ABORT, 'journal d''audit en ajout seul'); END;
             CREATE TRIGGER IF NOT EXISTS audit_no_delete BEFORE DELETE ON audit
                BEGIN SELECT RAISE(ABORT, 'journal d''audit en ajout seul'); END;",
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
            pool.get().map_err(|e| format!("PostgreSQL : {e}"))?,
        ))))
    }

    /// Adapte une requête écrite avec `$n` et `{user}` au moteur.
    fn sql(&self, q: &str) -> String {
        match self {
            Gov::Sqlite(_) => {
                let mut s = q.replace("{user}", "user");
                for i in (1..=9).rev() {
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

    /// Marque en échec les travaux restés en cours lors d'un arrêt : tous en SQLite (instance unique),
    /// ceux de cette instance en PostgreSQL (les autres instances continuent les leurs).
    pub fn fail_interrupted_jobs(&self) -> Result<u64, String> {
        let base = "UPDATE jobs SET status = 'failed', error = 'interrompu par un arrêt du serveur', finished_at = $1
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
