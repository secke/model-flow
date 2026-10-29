//! PostgreSQL connection: r2d2 pool, TLS (native-tls) and schema creation.
//!
//! The URL follows the libpq format (`postgres://user:pass@host:5432/db?sslmode=require`).
//! With `sslmode=require` or `verify-full`, the server certificate is verified.

use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use r2d2_postgres::PostgresConnectionManager;

pub type PgPool = r2d2::Pool<PostgresConnectionManager<MakeTlsConnector>>;
pub type PgConn = r2d2::PooledConnection<PostgresConnectionManager<MakeTlsConnector>>;

/// Shared pool whose shutdown (dropping the connections) happens on a dedicated thread:
/// the synchronous PostgreSQL client cannot be closed from an asynchronous (tokio) task.
pub struct SharedPool(Option<PgPool>);

impl SharedPool {
    pub fn new(p: PgPool) -> Self {
        Self(Some(p))
    }
    pub fn pool(&self) -> &PgPool {
        self.0.as_ref().expect("pool closed")
    }
}

impl Drop for SharedPool {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::thread::spawn(move || drop(p)).join();
        }
    }
}

/// Opens a pool and applies the `schema` (idempotent statements).
pub fn connect(url: &str, max_size: u32, schema: &str) -> Result<PgPool, String> {
    let config: postgres::Config = url.parse().map_err(|e| format!("invalid PostgreSQL URL: {e}"))?;
    let tls = MakeTlsConnector::new(TlsConnector::new().map_err(|e| format!("TLS : {e}"))?);
    let pool = r2d2::Pool::builder()
        .max_size(max_size)
        .connection_timeout(std::time::Duration::from_secs(10))
        .build(PostgresConnectionManager::new(config, tls))
        .map_err(|e| format!("PostgreSQL : {e}"))?;
    let mut c = pool.get().map_err(|e| format!("PostgreSQL : {e}"))?;
    // Advisory lock: several instances can start at the same time without migration conflicts.
    c.batch_execute(&format!(
        "SELECT pg_advisory_lock(7231); {schema} SELECT pg_advisory_unlock(7231);"
    ))
    .map_err(|e| {
        format!(
            "PostgreSQL (schema): {}",
            e.as_db_error()
                .map(|d| d.message().to_string())
                .unwrap_or(e.to_string())
        )
    })?;
    Ok(pool)
}

/// Pool for the run store.
pub fn connect_store(url: &str, max_size: u32) -> Result<PgPool, String> {
    connect(url, max_size, crate::store::PG_SCHEMA)
}

/// Creates an empty database for a test (requires the CREATEDB privilege) and returns its URL.
pub fn create_test_database(admin_url: &str, name: &str) -> Result<String, String> {
    let msg = |e: postgres::Error| {
        e.as_db_error()
            .map(|d| d.message().to_string())
            .unwrap_or_else(|| e.to_string())
    };
    let mut c = postgres::Client::connect(admin_url, postgres::NoTls).map_err(msg)?;
    c.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .map_err(msg)?;
    c.batch_execute(&format!("CREATE DATABASE {name}")).map_err(msg)?;
    let mut cfg: postgres::Config = admin_url.parse().map_err(|e: postgres::Error| e.to_string())?;
    cfg.dbname(name);
    let (host, port) = match (cfg.get_hosts().first(), cfg.get_ports().first()) {
        (Some(postgres::config::Host::Tcp(h)), Some(p)) => (h.clone(), *p),
        _ => return Err("test URL: a TCP host is required".into()),
    };
    let user = cfg.get_user().unwrap_or("postgres");
    let pass = cfg
        .get_password()
        .map(|p| format!(":{}", String::from_utf8_lossy(p)))
        .unwrap_or_default();
    Ok(format!("postgres://{user}{pass}@{host}:{port}/{name}?sslmode=disable"))
}
