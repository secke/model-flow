use mf_core::Manifest;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Store(Connection);

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunRow {
    pub id: i64,
    pub pipeline: String,
    pub run_hash: String,
    pub created_at: i64,
    pub triggered_by: String,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

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
             CREATE INDEX IF NOT EXISTS runs_pipeline ON runs(pipeline);",
        )
        .map_err(|e| e.to_string())?;
        // Migration v0.5 : auteur de l'exécution.
        let has_col: bool = c
            .prepare("SELECT 1 FROM pragma_table_info('runs') WHERE name = 'triggered_by'")
            .and_then(|mut s| s.exists([]))
            .map_err(|e| e.to_string())?;
        if !has_col {
            c.execute_batch("ALTER TABLE runs ADD COLUMN triggered_by TEXT NOT NULL DEFAULT 'cli'")
                .map_err(|e| e.to_string())?;
        }
        Ok(Self(c))
    }

    pub fn save(&self, pipeline: &str, m: &Manifest, triggered_by: &str) -> Result<i64, String> {
        self.0
            .execute(
                "INSERT INTO runs (pipeline, run_hash, created_at, manifest, triggered_by) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    pipeline,
                    m.run_hash,
                    now(),
                    serde_json::to_string(m).unwrap(),
                    triggered_by
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.0.last_insert_rowid())
    }

    /// Exécution `id`, ou la dernière du pipeline si `id` est absent.
    pub fn get(&self, pipeline: &str, id: Option<i64>) -> Result<(i64, Manifest), String> {
        let (rid, text): (i64, String) = match id {
            Some(i) => self.0.query_row(
                "SELECT id, manifest FROM runs WHERE id = ?1 AND pipeline = ?2",
                params![i, pipeline],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ),
            None => self.0.query_row(
                "SELECT id, manifest FROM runs WHERE pipeline = ?1 ORDER BY id DESC LIMIT 1",
                params![pipeline],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ),
        }
        .map_err(|_| "aucune exécution trouvée".to_string())?;
        Ok((rid, serde_json::from_str(&text).map_err(|e| e.to_string())?))
    }

    pub fn info(&self, pipeline: &str, id: i64) -> Result<Option<RunRow>, String> {
        self.0
            .query_row(
                "SELECT id, pipeline, run_hash, created_at, triggered_by FROM runs WHERE id = ?1 AND pipeline = ?2",
                params![id, pipeline],
                row,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn list(&self, pipeline: &str) -> Result<Vec<RunRow>, String> {
        let mut st = self
            .0
            .prepare(
                "SELECT id, pipeline, run_hash, created_at, triggered_by FROM runs WHERE pipeline = ?1 ORDER BY id",
            )
            .map_err(|e| e.to_string())?;
        let rows = st.query_map(params![pipeline], row).map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: r.get(0)?,
        pipeline: r.get(1)?,
        run_hash: r.get(2)?,
        created_at: r.get(3)?,
        triggered_by: r.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
