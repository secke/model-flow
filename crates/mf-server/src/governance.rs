//! Décisions de validation et journal d'audit (SQLite, en ajout seul).

use crate::Role;
use mf_runtime::report::Decision;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;

pub struct Gov(Connection);

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
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
        Ok(Self(c))
    }

    pub fn create_job(&self, pipeline: &str, user: &str) -> Result<i64, String> {
        self.0
            .execute(
                "INSERT INTO jobs (pipeline, user, status, created_at) VALUES (?1, ?2, 'queued', ?3)",
                params![pipeline, user, now()],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.0.last_insert_rowid())
    }

    pub fn job_started(&self, id: i64) -> Result<(), String> {
        self.0
            .execute(
                "UPDATE jobs SET status = 'running', started_at = ?2 WHERE id = ?1",
                params![id, now()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn job_finished(&self, id: i64, run_id: Option<i64>, error: Option<&str>) -> Result<(), String> {
        let status = if error.is_none() { "succeeded" } else { "failed" };
        self.0
            .execute(
                "UPDATE jobs SET status = ?2, run_id = ?3, error = ?4, finished_at = ?5 WHERE id = ?1",
                params![id, status, run_id, error, now()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn fail_interrupted_jobs(&self) -> Result<(), String> {
        self.0
            .execute(
                "UPDATE jobs SET status = 'failed', error = 'interrompu par un arrêt du serveur', finished_at = ?1
                 WHERE status IN ('queued', 'running')",
                params![now()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn job(&self, id: i64) -> Result<Option<Value>, String> {
        use rusqlite::OptionalExtension;
        self.0
            .query_row(
                "SELECT id, pipeline, user, status, run_id, error, created_at, started_at, finished_at FROM jobs WHERE id = ?1",
                params![id],
                |r| {
                    Ok(json!({"id": r.get::<_, i64>(0)?, "pipeline": r.get::<_, String>(1)?, "user": r.get::<_, String>(2)?,
                              "status": r.get::<_, String>(3)?, "run_id": r.get::<_, Option<i64>>(4)?,
                              "error": r.get::<_, Option<String>>(5)?, "created_at": r.get::<_, i64>(6)?,
                              "started_at": r.get::<_, Option<i64>>(7)?, "finished_at": r.get::<_, Option<i64>>(8)?}))
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn audit(&self, user: &str, role: Role, action: &str, target: &str, outcome: &str) -> Result<(), String> {
        let role = serde_json::to_value(role)
            .unwrap()
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.0
            .execute(
                "INSERT INTO audit (ts, user, role, action, target, outcome) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![now(), user, role, action, target, outcome],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn decide(
        &self,
        pipeline: &str,
        run_id: i64,
        decision: &str,
        comment: &str,
        user: &str,
    ) -> Result<i64, String> {
        self.0
            .execute(
                "INSERT INTO decisions (pipeline, run_id, decision, comment, user, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![pipeline, run_id, decision, comment, user, now()],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.0.last_insert_rowid())
    }

    pub fn decisions(&self, pipeline: &str, run_id: i64) -> Result<Vec<Decision>, String> {
        let mut st = self
            .0
            .prepare("SELECT user, decision, comment, created_at FROM decisions WHERE pipeline = ?1 AND run_id = ?2 ORDER BY id")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map(params![pipeline, run_id], |r| {
                Ok(Decision {
                    user: r.get(0)?,
                    decision: r.get(1)?,
                    comment: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn audit_log(&self, limit: u32) -> Result<Vec<Value>, String> {
        let mut st = self
            .0
            .prepare("SELECT id, ts, user, role, action, target, outcome FROM audit ORDER BY id DESC LIMIT ?1")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map(params![limit], |r| {
                Ok(
                    json!({"id": r.get::<_, i64>(0)?, "ts": r.get::<_, i64>(1)?, "user": r.get::<_, String>(2)?,
                          "role": r.get::<_, String>(3)?, "action": r.get::<_, String>(4)?,
                          "target": r.get::<_, String>(5)?, "outcome": r.get::<_, String>(6)?}),
                )
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
}
