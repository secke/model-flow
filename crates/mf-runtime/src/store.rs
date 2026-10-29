//! Run history and content-addressed output store.
//! Two implementations: SQLite (local file, one database per pipeline) and PostgreSQL (shared).

use crate::pg::{PgConn, PgPool};
use mf_core::{hash_value, Manifest};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::cell::RefCell;
use std::path::Path;

pub enum Store {
    Sqlite(Connection),
    Postgres(Box<RefCell<PgConn>>),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunRow {
    pub id: i64,
    pub pipeline: String,
    pub run_hash: String,
    pub created_at: i64,
    pub triggered_by: String,
    /// Worst status of the check steps (`pass`, `warn`, `fail`), absent if there are none.
    pub status: Option<String>,
}

/// Worst status (`fail` > `warn` > `pass`) among the step outputs.
pub fn overall_status(m: &Manifest) -> Option<String> {
    let rank = |s: &str| match s {
        "fail" => 3,
        "warn" => 2,
        "pass" => 1,
        _ => 0,
    };
    m.steps
        .iter()
        .filter_map(|s| s.output.get("status").and_then(Value::as_str))
        .max_by_key(|s| rank(s))
        .filter(|s| rank(s) > 0)
        .map(str::to_string)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn pg_err(e: postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!("PostgreSQL : {}", db.message()),
        None => format!("PostgreSQL : {e}"),
    }
}

/// PostgreSQL schema for runs (idempotent).
pub const PG_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS runs (
    id BIGSERIAL PRIMARY KEY,
    pipeline TEXT NOT NULL,
    run_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    manifest TEXT NOT NULL,
    triggered_by TEXT NOT NULL DEFAULT 'cli');
CREATE INDEX IF NOT EXISTS runs_pipeline ON runs(pipeline);
ALTER TABLE runs ADD COLUMN IF NOT EXISTS status TEXT;
CREATE TABLE IF NOT EXISTS blobs (
    hash TEXT PRIMARY KEY,
    size BIGINT NOT NULL,
    content TEXT NOT NULL);";

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let c = Connection::open(path).map_err(|e| e.to_string())?;
        c.busy_timeout(std::time::Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pipeline TEXT NOT NULL,
                run_hash TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                manifest TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS runs_pipeline ON runs(pipeline);
             CREATE TABLE IF NOT EXISTS blobs (
                hash TEXT PRIMARY KEY,
                size INTEGER NOT NULL,
                content TEXT NOT NULL);",
        )
        .map_err(|e| e.to_string())?;
        // v0.5 migration: run author.
        let has_col: bool = c
            .prepare("SELECT 1 FROM pragma_table_info('runs') WHERE name = 'triggered_by'")
            .and_then(|mut s| s.exists([]))
            .map_err(|e| e.to_string())?;
        if !has_col {
            c.execute_batch("ALTER TABLE runs ADD COLUMN triggered_by TEXT NOT NULL DEFAULT 'cli'")
                .map_err(|e| e.to_string())?;
        }
        let has_status: bool = c
            .prepare("SELECT 1 FROM pragma_table_info('runs') WHERE name = 'status'")
            .and_then(|mut s| s.exists([]))
            .map_err(|e| e.to_string())?;
        if !has_status {
            c.execute_batch("ALTER TABLE runs ADD COLUMN status TEXT")
                .map_err(|e| e.to_string())?;
        }
        Ok(Store::Sqlite(c))
    }

    /// Connection taken from the PostgreSQL pool (schema created by [`crate::pg::connect`]).
    pub fn postgres(pool: &PgPool) -> Result<Self, String> {
        Ok(Store::Postgres(Box::new(RefCell::new(
            pool.get().map_err(|e| format!("PostgreSQL : {e}"))?,
        ))))
    }

    /// Records a run. Outputs are stored once, addressed by their
    /// fingerprint (deduplicated across runs); the manifest only keeps the fingerprints.
    pub fn save(&self, pipeline: &str, m: &Manifest, triggered_by: &str) -> Result<i64, String> {
        let mut slim = m.clone();
        let blobs: Vec<(String, String)> = m
            .steps
            .iter()
            .map(|s| (s.output_hash.clone(), serde_json::to_string(&s.output).unwrap()))
            .collect();
        slim.steps.iter_mut().for_each(|s| s.output = Value::Null);
        let manifest = serde_json::to_string(&slim).unwrap();
        let status = overall_status(m);
        match self {
            Store::Sqlite(c) => {
                let tx = c.unchecked_transaction().map_err(|e| e.to_string())?;
                for (h, content) in &blobs {
                    tx.execute(
                        "INSERT OR IGNORE INTO blobs (hash, size, content) VALUES (?1, ?2, ?3)",
                        params![h, content.len() as i64, content],
                    )
                    .map_err(|e| e.to_string())?;
                }
                tx.execute(
                    "INSERT INTO runs (pipeline, run_hash, created_at, manifest, triggered_by, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![pipeline, m.run_hash, now(), manifest, triggered_by, status],
                )
                .map_err(|e| e.to_string())?;
                let id = tx.last_insert_rowid();
                tx.commit().map_err(|e| e.to_string())?;
                Ok(id)
            }
            Store::Postgres(c) => {
                let mut c = c.borrow_mut();
                let mut tx = c.transaction().map_err(pg_err)?;
                for (h, content) in &blobs {
                    tx.execute(
                        "INSERT INTO blobs (hash, size, content) VALUES ($1, $2, $3) ON CONFLICT (hash) DO NOTHING",
                        &[h, &(content.len() as i64), content],
                    )
                    .map_err(pg_err)?;
                }
                let id: i64 = tx
                    .query_one(
                        "INSERT INTO runs (pipeline, run_hash, created_at, manifest, triggered_by, status)
                         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
                        &[&pipeline, &m.run_hash, &now(), &manifest, &triggered_by, &status],
                    )
                    .map_err(pg_err)?
                    .get(0);
                tx.commit().map_err(pg_err)?;
                Ok(id)
            }
        }
    }

    fn blob(&self, hash: &str) -> Result<Option<String>, String> {
        match self {
            Store::Sqlite(c) => c
                .query_row("SELECT content FROM blobs WHERE hash = ?1", params![hash], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string()),
            Store::Postgres(c) => Ok(c
                .borrow_mut()
                .query_opt("SELECT content FROM blobs WHERE hash = $1", &[&hash])
                .map_err(pg_err)?
                .map(|r| r.get(0))),
        }
    }

    /// Reloads the outputs from the store and checks their integrity.
    fn hydrate(&self, m: &mut Manifest) -> Result<(), String> {
        for s in &mut m.steps {
            if !s.output.is_null() {
                continue; // manifest older than v0.6: inline output
            }
            let content = self
                .blob(&s.output_hash)?
                .ok_or_else(|| format!("output of step {} not found in the store", s.name))?;
            let v: Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
            if hash_value(&v) != s.output_hash {
                return Err(format!(
                    "integrity compromised: the stored output of step {} has been modified",
                    s.name
                ));
            }
            s.output = v;
        }
        Ok(())
    }

    /// Store size: (number of distinct outputs, bytes).
    pub fn blob_stats(&self) -> Result<(i64, i64), String> {
        match self {
            Store::Sqlite(c) => c
                .query_row("SELECT COUNT(*), COALESCE(SUM(size), 0) FROM blobs", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .map_err(|e| e.to_string()),
            Store::Postgres(c) => {
                let r = c
                    .borrow_mut()
                    .query_one(
                        "SELECT COUNT(*)::BIGINT, COALESCE(SUM(size), 0)::BIGINT FROM blobs",
                        &[],
                    )
                    .map_err(pg_err)?;
                Ok((r.get(0), r.get(1)))
            }
        }
    }

    /// Run `id`, or the pipeline's latest run if `id` is absent.
    pub fn get(&self, pipeline: &str, id: Option<i64>) -> Result<(i64, Manifest), String> {
        let found: Option<(i64, String)> = match self {
            Store::Sqlite(c) => match id {
                Some(i) => c.query_row(
                    "SELECT id, manifest FROM runs WHERE id = ?1 AND pipeline = ?2",
                    params![i, pipeline],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                ),
                None => c.query_row(
                    "SELECT id, manifest FROM runs WHERE pipeline = ?1 ORDER BY id DESC LIMIT 1",
                    params![pipeline],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                ),
            }
            .optional()
            .map_err(|e| e.to_string())?,
            Store::Postgres(c) => {
                let mut c = c.borrow_mut();
                match id {
                    Some(i) => c.query_opt(
                        "SELECT id, manifest FROM runs WHERE id = $1 AND pipeline = $2",
                        &[&i, &pipeline],
                    ),
                    None => c.query_opt(
                        "SELECT id, manifest FROM runs WHERE pipeline = $1 ORDER BY id DESC LIMIT 1",
                        &[&pipeline],
                    ),
                }
                .map_err(pg_err)?
                .map(|r| (r.get(0), r.get(1)))
            }
        };
        let (rid, text) = found.ok_or_else(|| "no run found".to_string())?;
        let mut m: Manifest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        self.hydrate(&mut m)?;
        Ok((rid, m))
    }

    pub fn info(&self, pipeline: &str, id: i64) -> Result<Option<RunRow>, String> {
        let q =
            "SELECT id, pipeline, run_hash, created_at, triggered_by, status FROM runs WHERE id = %1 AND pipeline = %2";
        match self {
            Store::Sqlite(c) => c
                .query_row(&q.replace("%1", "?1").replace("%2", "?2"), params![id, pipeline], row)
                .optional()
                .map_err(|e| e.to_string()),
            Store::Postgres(c) => Ok(c
                .borrow_mut()
                .query_opt(&q.replace("%1", "$1").replace("%2", "$2"), &[&id, &pipeline])
                .map_err(pg_err)?
                .map(|r| pg_row(&r))),
        }
    }

    pub fn list(&self, pipeline: &str) -> Result<Vec<RunRow>, String> {
        let q =
            "SELECT id, pipeline, run_hash, created_at, triggered_by, status FROM runs WHERE pipeline = %1 ORDER BY id";
        match self {
            Store::Sqlite(c) => {
                let mut st = c.prepare(&q.replace("%1", "?1")).map_err(|e| e.to_string())?;
                let rows = st.query_map(params![pipeline], row).map_err(|e| e.to_string())?;
                rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
            }
            Store::Postgres(c) => Ok(c
                .borrow_mut()
                .query(&q.replace("%1", "$1"), &[&pipeline])
                .map_err(pg_err)?
                .iter()
                .map(pg_row)
                .collect()),
        }
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: r.get(0)?,
        pipeline: r.get(1)?,
        run_hash: r.get(2)?,
        created_at: r.get(3)?,
        triggered_by: r.get(4)?,
        status: r.get(5)?,
    })
}

fn pg_row(r: &postgres::Row) -> RunRow {
    RunRow {
        id: r.get(0),
        pipeline: r.get(1),
        run_hash: r.get(2),
        created_at: r.get(3),
        triggered_by: r.get(4),
        status: r.get(5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Store {
        fn sql(&self) -> &Connection {
            match self {
                Store::Sqlite(c) => c,
                Store::Postgres(_) => panic!("SQLite expected"),
            }
        }
    }

    /// Test PostgreSQL database (MODELFLOW_TEST_DATABASE_URL variable), otherwise `None`.
    fn pg_store(tag: &str) -> Option<Store> {
        let admin = std::env::var("MODELFLOW_TEST_DATABASE_URL").ok()?;
        let url = crate::pg::create_test_database(&admin, &format!("mf_store_{tag}_{}", std::process::id())).unwrap();
        Some(Store::postgres(&crate::pg::connect_store(&url, 2).unwrap()).unwrap())
    }

    fn sample() -> Manifest {
        use mf_core::{FnStep, Pipeline};
        Pipeline::new()
            .step(FnStep::new("data", "1", &[], |_| {
                Ok(serde_json::json!((1..10_000)
                    .map(|i| (i as f64).sqrt() / 7.0)
                    .collect::<Vec<f64>>()))
            }))
            .execute()
            .unwrap()
    }

    #[test]
    fn postgres_roundtrip_dedup_and_integrity() {
        let Some(s) = pg_store("rt") else {
            eprintln!("MODELFLOW_TEST_DATABASE_URL not set: PostgreSQL test skipped");
            return;
        };
        let m = sample();
        assert_eq!(s.save("p", &m, "alice").unwrap(), 1);
        assert_eq!(s.save("p", &m, "bob").unwrap(), 2);
        assert_eq!(s.blob_stats().unwrap().0, 1);
        let (id, back) = s.get("p", None).unwrap();
        assert_eq!((id, back.steps[0].output.clone()), (2, m.steps[0].output.clone()));
        assert_eq!(s.info("p", 1).unwrap().unwrap().triggered_by, "alice");
        assert_eq!(s.list("p").unwrap().len(), 2);
        assert!(s.get("other", None).is_err());
        if let Store::Postgres(c) = &s {
            c.borrow_mut()
                .execute("UPDATE blobs SET content = '[2.0]'", &[])
                .unwrap();
        }
        assert!(s.get("p", Some(1)).unwrap_err().contains("integrity"));
    }

    #[test]
    fn migrates_old_schema() {
        let dir = std::env::temp_dir().join(format!("mf-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runs.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE runs (id INTEGER PRIMARY KEY AUTOINCREMENT, pipeline TEXT NOT NULL, run_hash TEXT NOT NULL,
                 created_at INTEGER NOT NULL, manifest TEXT NOT NULL);
                 INSERT INTO runs (pipeline, run_hash, created_at, manifest) VALUES ('p', 'h', 0, '{\"steps\":[],\"run_hash\":\"h\"}');",
            )
            .unwrap();
        let s = Store::open(&path).unwrap();
        assert_eq!(s.info("p", 1).unwrap().unwrap().triggered_by, "cli");
        let m = s.get("p", None).unwrap().1;
        assert_eq!(s.save("p", &m, "alice").unwrap(), 2);
        assert_eq!(s.list("p").unwrap()[1].triggered_by, "alice");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deduplicates_and_detects_tampering() {
        use mf_core::{FnStep, Pipeline};
        let dir = std::env::temp_dir().join(format!("mf-blobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("runs.db")).unwrap();
        let m = Pipeline::new()
            .step(FnStep::new("data", "1", &[], |_| {
                Ok(serde_json::json!((1..10_000)
                    .map(|i| (i as f64).sqrt() / 7.0)
                    .collect::<Vec<f64>>()))
            }))
            .execute()
            .unwrap();
        s.save("p", &m, "a").unwrap();
        s.save("p", &m, "a").unwrap();
        let (n, bytes) = s.blob_stats().unwrap();
        assert_eq!(n, 1, "a single copy for two identical runs");
        assert!(bytes > 30_000);
        // Exact float round-trip (otherwise the fingerprint differs after storage).
        let manifest_len: i64 = s
            .sql()
            .query_row("SELECT length(manifest) FROM runs WHERE id = 2", [], |r| r.get(0))
            .unwrap();
        assert!(manifest_len < 1_000, "lightweight manifest: {manifest_len} bytes");
        assert_eq!(s.get("p", Some(2)).unwrap().1.steps[0].output, m.steps[0].output);

        s.sql().execute("UPDATE blobs SET content = '[2.0]'", []).unwrap();
        assert!(s.get("p", Some(1)).unwrap_err().contains("integrity"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
