use mf_core::Manifest;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Store(Connection);

pub struct RunRow {
    pub id: i64,
    pub pipeline: String,
    pub run_hash: String,
    pub created_at: i64,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(d) = path.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
        let c = Connection::open(path).map_err(|e| e.to_string())?;
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pipeline TEXT NOT NULL,
                run_hash TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                manifest TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS runs_pipeline ON runs(pipeline);",
        ).map_err(|e| e.to_string())?;
        Ok(Self(c))
    }

    pub fn save(&self, pipeline: &str, m: &Manifest) -> Result<i64, String> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        self.0.execute(
            "INSERT INTO runs (pipeline, run_hash, created_at, manifest) VALUES (?1, ?2, ?3, ?4)",
            params![pipeline, m.run_hash, now, serde_json::to_string(m).unwrap()],
        ).map_err(|e| e.to_string())?;
        Ok(self.0.last_insert_rowid())
    }

    /// Exécution `id`, ou la dernière du pipeline si `id` est absent.
    pub fn get(&self, pipeline: &str, id: Option<i64>) -> Result<(i64, Manifest), String> {
        let (rid, text): (i64, String) = match id {
            Some(i) => self.0.query_row("SELECT id, manifest FROM runs WHERE id = ?1 AND pipeline = ?2",
                params![i, pipeline], |r| Ok((r.get(0)?, r.get(1)?))),
            None => self.0.query_row("SELECT id, manifest FROM runs WHERE pipeline = ?1 ORDER BY id DESC LIMIT 1",
                params![pipeline], |r| Ok((r.get(0)?, r.get(1)?))),
        }.map_err(|_| "aucune exécution trouvée".to_string())?;
        Ok((rid, serde_json::from_str(&text).map_err(|e| e.to_string())?))
    }

    pub fn list(&self, pipeline: &str) -> Result<Vec<RunRow>, String> {
        let mut st = self.0.prepare("SELECT id, pipeline, run_hash, created_at FROM runs WHERE pipeline = ?1 ORDER BY id")
            .map_err(|e| e.to_string())?;
        let rows = st.query_map(params![pipeline], |r| Ok(RunRow { id: r.get(0)?, pipeline: r.get(1)?, run_hash: r.get(2)?, created_at: r.get(3)? }))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
}
