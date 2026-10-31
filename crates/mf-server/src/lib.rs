//! ModelFlow REST API: runs, replay, reports, validation decisions and audit log.
//!
//! Bearer token authentication (only the SHA-256 fingerprint is stored in the configuration).
//! Roles: `admin`, `model_owner`, `validator`, `auditor`. Segregation of duties: only a validator
//! can approve or reject, never a run they triggered, and only if the replay is identical.

pub mod alerts;
pub mod governance;
pub mod oidc;
pub mod scheduler;

use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::{header, request::Parts, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use governance::Gov;
use mf_runtime::{report, spec, store::Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

// ---------- Configuration ----------

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    ModelOwner,
    Validator,
    Auditor,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserConfig {
    pub name: String,
    pub role: Role,
    pub token_sha256: String,
}

fn default_bind() -> String {
    "127.0.0.1:8080".into()
}
fn default_concurrency() -> usize {
    2
}
fn default_rate() -> u32 {
    120
}

/// PEM certificate and key to serve HTTPS directly.
#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Directory containing one subdirectory per pipeline (`<dir>/pipeline.toml`).
    pub workspace: PathBuf,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(rename = "user", default)]
    pub users: Vec<UserConfig>,
    /// Maximum number of concurrent runs (further runs wait in a queue).
    #[serde(default = "default_concurrency")]
    pub max_concurrent_runs: usize,
    /// Authenticated requests allowed per user and per minute.
    #[serde(default = "default_rate")]
    pub rate_limit_per_minute: u32,
    pub tls: Option<TlsConfig>,
    /// Shared PostgreSQL database; without this section, SQLite (local files, single instance).
    pub database: Option<DatabaseConfig>,
    /// OpenID Connect single sign-on (in addition to the `[[user]]` service tokens).
    pub oidc: Option<oidc::OidcConfig>,
    /// Public URL of the web UI, used for links in alerts.
    pub public_url: Option<String>,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    pub alerts: Option<alerts::AlertsConfig>,
}

fn yes() -> bool {
    true
}
fn default_tick() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "default_tick")]
    pub interval_secs: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 30,
        }
    }
}

fn default_pool() -> u32 {
    10
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    /// libpq URL; prefer `url_env` to keep the password out of the configuration.
    pub url: Option<String>,
    /// Name of the environment variable holding the URL (e.g. `MODELFLOW_DATABASE_URL`).
    pub url_env: Option<String>,
    #[serde(default = "default_pool")]
    pub max_connections: u32,
}

impl DatabaseConfig {
    pub fn resolve_url(&self) -> Result<String, String> {
        match (&self.url, &self.url_env) {
            (Some(u), None) => Ok(u.clone()),
            (None, Some(v)) => std::env::var(v).map_err(|_| format!("environment variable `{v}` is not set")),
            _ => Err("[database]: specify `url` or `url_env`".into()),
        }
    }
}

impl ServerConfig {
    pub fn parse(text: &str, base: &FsPath) -> Result<Self, String> {
        let mut c: ServerConfig = toml::from_str(text).map_err(|e| format!("invalid configuration: {e}"))?;
        if c.workspace.is_relative() {
            c.workspace = base.join(&c.workspace);
        }
        if let Some(o) = &mut c.oidc {
            if let Some(f) = &mut o.jwks_file {
                if f.is_relative() {
                    *f = base.join(&*f);
                }
            }
        }
        if let Some(t) = &mut c.tls {
            for p in [&mut t.cert, &mut t.key] {
                if p.is_relative() {
                    *p = base.join(&*p);
                }
            }
        }
        if c.max_concurrent_runs == 0 || c.rate_limit_per_minute == 0 {
            return Err("max_concurrent_runs and rate_limit_per_minute must be ≥ 1".into());
        }
        let mut names = std::collections::HashSet::new();
        for u in &c.users {
            if !names.insert(&u.name) {
                return Err(format!("duplicate user: {}", u.name));
            }
            if u.token_sha256.len() != 64 || !u.token_sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(format!(
                    "{}: token_sha256 must be a hexadecimal SHA-256 fingerprint",
                    u.name
                ));
            }
        }
        Ok(c)
    }

    pub fn load(path: &FsPath) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text, path.parent().unwrap_or(FsPath::new(".")))
    }
}

pub fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Random 256-bit token.
pub fn new_token() -> String {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("randomness source");
    format!("mft_{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

// ---------- State and errors ----------

enum Backend {
    /// One SQLite database per pipeline (history) and one governance database in the workspace.
    Sqlite {
        gov_db: PathBuf,
    },
    Postgres(mf_runtime::pg::SharedPool),
}

struct Inner {
    workspace: PathBuf,
    users: Vec<UserConfig>,
    public_url: Option<String>,
    scheduler: SchedulerConfig,
    alerts: Option<alerts::AlertsConfig>,
    oidc: Option<Arc<oidc::Oidc>>,
    backend: Backend,
    runs: Arc<tokio::sync::Semaphore>,
    rate: f64,
    buckets: std::sync::Mutex<std::collections::HashMap<String, (f64, std::time::Instant)>>,
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl AppState {
    /// Initializes the state (connection and migrations). Database calls are synchronous:
    /// they run on a dedicated thread, so this can be called from an asynchronous context.
    pub fn new(cfg: &ServerConfig) -> Result<Self, String> {
        std::thread::scope(|sc| {
            sc.spawn(|| Self::init(cfg))
                .join()
                .map_err(|_| "initialization interrupted".to_string())?
        })
    }

    fn init(cfg: &ServerConfig) -> Result<Self, String> {
        let backend = match &cfg.database {
            Some(db) => {
                let schema = format!(
                    "{}
{}",
                    mf_runtime::store::PG_SCHEMA,
                    governance::PG_SCHEMA
                );
                Backend::Postgres(mf_runtime::pg::SharedPool::new(mf_runtime::pg::connect(
                    &db.resolve_url()?,
                    db.max_connections,
                    &schema,
                )?))
            }
            None => Backend::Sqlite {
                gov_db: cfg.workspace.join(".modelflow-server/governance.db"),
            },
        };
        let state = Self(Arc::new(Inner {
            workspace: cfg.workspace.clone(),
            users: cfg.users.clone(),
            public_url: cfg.public_url.clone(),
            scheduler: cfg.scheduler.clone(),
            alerts: cfg.alerts.clone(),
            oidc: cfg.oidc.clone().map(oidc::Oidc::new).transpose()?.map(Arc::new),
            backend,
            runs: Arc::new(tokio::sync::Semaphore::new(cfg.max_concurrent_runs)),
            rate: cfg.rate_limit_per_minute as f64,
            buckets: Default::default(),
        }));
        // Jobs interrupted by a server shutdown are marked as failed.
        state.gov().map_err(|e| e.1)?.fail_interrupted_jobs()?;
        Ok(state)
    }

    /// Storage backend in use (`sqlite` or `postgres`).
    pub fn backend_name(&self) -> &'static str {
        match self.0.backend {
            Backend::Sqlite { .. } => "sqlite",
            Backend::Postgres(_) => "postgres",
        }
    }

    fn store_for(&self, toml: &FsPath) -> ApiResult<Store> {
        match &self.0.backend {
            Backend::Sqlite { .. } => {
                let base = toml.parent().unwrap_or(FsPath::new("."));
                Store::open(&base.join(".modelflow/runs.db")).map_err(ApiError::internal)
            }
            Backend::Postgres(pool) => Store::postgres(pool.pool()).map_err(ApiError::internal),
        }
    }

    /// Per-user token bucket: `rate` requests per minute, burst of the same size.
    fn allow(&self, user: &str) -> bool {
        let mut b = self.0.buckets.lock().unwrap();
        let now = std::time::Instant::now();
        let (tokens, last) = b.entry(user.to_string()).or_insert((self.0.rate, now));
        *tokens = (*tokens + now.duration_since(*last).as_secs_f64() * self.0.rate / 60.0).min(self.0.rate);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn gov(&self) -> Result<Gov, ApiError> {
        match &self.0.backend {
            Backend::Sqlite { gov_db } => Gov::open(gov_db),
            Backend::Postgres(pool) => Gov::postgres(pool.pool()),
        }
        .map_err(ApiError::internal)
    }
}

#[derive(Debug)]
pub struct ApiError(StatusCode, String);

impl ApiError {
    fn new(s: StatusCode, m: impl Into<String>) -> Self {
        Self(s, m.into())
    }
    fn internal(m: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, m)
    }
    fn not_found(m: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, m)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> ApiResult<T> + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
}

// ---------- Authentication ----------

#[derive(Debug, Clone)]
pub struct Auth {
    pub name: String,
    pub role: Role,
}

#[axum::async_trait]
impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing token (Authorization: Bearer …)"))?;
        let token = token.trim().to_string();
        let auth = match (&state.0.oidc, token.matches('.').count() == 2) {
            // JWT: OIDC single sign-on (verification may block: JWKS reload).
            (Some(o), true) => {
                let o = o.clone();
                let t = token.clone();
                match tokio::task::spawn_blocking(move || o.verify(&t))
                    .await
                    .map_err(|e| ApiError::internal(e.to_string()))?
                {
                    Ok((name, role)) => Auth { name, role },
                    Err(e) if e.forbidden => {
                        return Err(ApiError::new(StatusCode::FORBIDDEN, e.message));
                    }
                    Err(e) => return Err(ApiError::new(StatusCode::UNAUTHORIZED, e.message)),
                }
            }
            // Service token: SHA-256 fingerprint comparison.
            _ => {
                let h = hash_token(&token);
                state
                    .0
                    .users
                    .iter()
                    .find(|u| u.token_sha256.eq_ignore_ascii_case(&h))
                    .map(|u| Auth {
                        name: u.name.clone(),
                        role: u.role,
                    })
                    .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "invalid token"))?
            }
        };
        if state.allow(&auth.name) {
            Ok(auth)
        } else {
            Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate limit reached, try again later",
            ))
        }
    }
}

/// Checks the role; a denial is written to the audit log.
async fn authorize(state: &AppState, auth: &Auth, roles: &[Role], action: &str, target: &str) -> ApiResult<()> {
    if roles.contains(&auth.role) {
        return Ok(());
    }
    let (st, a, act, tgt) = (state.clone(), auth.clone(), action.to_string(), target.to_string());
    let _ = blocking(move || {
        st.gov()?
            .audit(&a.name, a.role, &act, &tgt, "denied")
            .map_err(ApiError::internal)
    })
    .await;
    Err(ApiError::new(
        StatusCode::FORBIDDEN,
        format!("role {:?} does not allow `{action}`", auth.role),
    ))
}

// ---------- Pipelines ----------

fn discover(ws: &FsPath) -> ApiResult<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    let entries = std::fs::read_dir(ws).map_err(|e| ApiError::internal(format!("{}: {e}", ws.display())))?;
    for e in entries.flatten() {
        let p = e.path().join("pipeline.toml");
        if p.is_file() {
            if let Ok((sp, _)) = spec::load(&p) {
                out.insert(sp.name, p);
            }
        }
    }
    Ok(out)
}

fn locate(state: &AppState, name: &str) -> ApiResult<PathBuf> {
    discover(&state.0.workspace)?
        .remove(name)
        .ok_or_else(|| ApiError::not_found(format!("unknown pipeline `{name}`")))
}

fn mismatches_json(mm: &[mf_core::Mismatch]) -> Value {
    json!(mm
        .iter()
        .map(|m| json!({"step": m.step, "reason": m.reason}))
        .collect::<Vec<_>>())
}

fn verify_run(st: &AppState, toml: &FsPath, name: &str, id: i64) -> ApiResult<Vec<mf_core::Mismatch>> {
    let (sp, base) = spec::load(toml).map_err(ApiError::internal)?;
    let (_, m) = st.store_for(toml)?.get(name, Some(id)).map_err(ApiError::not_found)?;
    let p = spec::build(&sp, &base).map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    p.verify(&m)
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))
}

// ---------- Handlers ----------

async fn health(State(st): State<AppState>) -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION"), "storage": st.backend_name() }))
}

// ---------- Web UI ----------

const UI_HTML: &str = include_str!("../ui/index.html");
const UI_JS: &str = include_str!("../ui/app.js");
const UI_CSS: &str = include_str!("../ui/app.css");

async fn ui_index() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        UI_HTML,
    )
        .into_response()
}

async fn ui_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        UI_JS,
    )
        .into_response()
}

async fn ui_css() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        UI_CSS,
    )
        .into_response()
}

/// Public UI configuration: SSO parameters (no secret, public PKCE client).
async fn ui_config(State(st): State<AppState>) -> Json<Value> {
    let sso = match st.0.oidc.clone() {
        Some(o) if o.config().client_id.is_some() => {
            let o2 = o.clone();
            let endpoints = tokio::task::spawn_blocking(move || o2.browser_endpoints())
                .await
                .ok()
                .and_then(Result::ok);
            let c = o.config();
            endpoints.map(|(auth, token)| {
                json!({"issuer": c.issuer, "client_id": c.client_id,
                       "scopes": c.scopes.clone().unwrap_or_else(|| "openid profile".into()),
                       "authorization_endpoint": auth, "token_endpoint": token})
            })
        }
        _ => None,
    };
    Json(json!({"version": env!("CARGO_PKG_VERSION"), "sso": sso, "service_tokens": !st.0.users.is_empty()}))
}

/// Security headers applied to every response.
async fn security_headers(
    State(st): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut res = next.run(req).await;
    let idp =
        st.0.oidc
            .as_ref()
            .map(|o| o.browser_origins().join(" "))
            .unwrap_or_default();
    let csp = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
         connect-src 'self' {idp}; frame-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'"
    );
    let h = res.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, k: &'static str, v: &str| {
        if let Ok(v) = axum::http::HeaderValue::from_str(v) {
            h.insert(k, v);
        }
    };
    set(h, "content-security-policy", &csp);
    set(h, "x-content-type-options", "nosniff");
    set(h, "referrer-policy", "no-referrer");
    set(h, "x-frame-options", "DENY");
    set(h, "permissions-policy", "camera=(), microphone=(), geolocation=()");
    res
}

async fn me(auth: Auth) -> Json<Value> {
    Json(json!({ "name": auth.name, "role": auth.role }))
}

async fn list_pipelines(State(st): State<AppState>, _auth: Auth) -> ApiResult<Json<Value>> {
    blocking(move || {
        let gov = st.gov()?;
        let mut out = vec![];
        for (name, toml) in discover(&st.0.workspace)? {
            let (sp, _) = spec::load(&toml).map_err(ApiError::internal)?;
            let steps: Vec<Value> = sp
                .steps
                .iter()
                .map(|s| json!({"name": s.name, "kind": s.kind, "version": s.version}))
                .collect();
            let runs = st.store_for(&toml)?.list(&name).map_err(ApiError::internal)?;
            let last = runs.last().map(|r| {
                let decision = gov
                    .decisions(&name, r.id)
                    .ok()
                    .and_then(|d| d.last().map(|x| x.decision.clone()));
                json!({"id": r.id, "created_at": r.created_at, "status": r.status, "decision": decision})
            });
            let next_run = sp
                .schedule
                .as_deref()
                .and_then(|e| scheduler::parse(e).ok())
                .and_then(|c| scheduler::next_after(&c, chrono::Utc::now().timestamp()));
            out.push(
                json!({ "name": name, "steps": steps, "runs": runs.len(), "last_run": last,
                             "model": sp.model, "schedule": sp.schedule, "next_run": next_run }),
            );
        }
        Ok(Json(json!(out)))
    })
    .await
}

async fn list_runs(State(st): State<AppState>, _auth: Auth, Path(name): Path<String>) -> ApiResult<Json<Value>> {
    blocking(move || {
        let toml = locate(&st, &name)?;
        let gov = st.gov()?;
        let rows = st.store_for(&toml)?.list(&name).map_err(ApiError::internal)?;
        let out: Vec<Value> = rows
            .into_iter()
            .map(|r| {
                let last = gov
                    .decisions(&name, r.id)
                    .ok()
                    .and_then(|d| d.last().map(|x| x.decision.clone()));
                json!({"id": r.id, "run_hash": r.run_hash, "created_at": r.created_at,
                       "triggered_by": r.triggered_by, "status": r.status, "decision": last})
            })
            .collect();
        Ok(Json(json!(out)))
    })
    .await
}

#[derive(Deserialize)]
struct RunQuery {
    /// Wait for the run to finish instead of returning immediately (202).
    #[serde(default)]
    wait: bool,
}

/// Runs a pipeline for an already recorded job (blocking thread), then alerts if needed.
fn execute_job(st: &AppState, job: i64, name: &str, auth: &Auth) -> Result<(i64, Value), String> {
    let gov = st.gov().map_err(|e| e.1)?;
    gov.job_started(job)?;
    let result = (|| -> Result<(i64, Value, Option<String>), String> {
        let toml = locate(st, name).map_err(|e| e.1)?;
        let (sp, base) = spec::load(&toml)?;
        let m = spec::build(&sp, &base)?.execute().map_err(|e| e.to_string())?;
        let id = st.store_for(&toml).map_err(|e| e.1)?.save(name, &m, &auth.name)?;
        let steps: Vec<Value> = m
            .steps
            .iter()
            .map(|s| json!({"name": s.name, "version": s.version, "status": s.output.get("status")}))
            .collect();
        let status = mf_runtime::store::overall_status(&m);
        Ok((
            id,
            json!({"id": id, "run_hash": m.run_hash, "status": status, "steps": steps}),
            status,
        ))
    })();
    let link = |id: Option<i64>| {
        st.0.public_url.as_ref().map(|u| {
            let base = format!("{}/#/p/{}", u.trim_end_matches('/'), name);
            id.map(|i| format!("{base}/r/{i}")).unwrap_or(base)
        })
    };
    let (event_kind, run_id, detail) = match &result {
        Ok((id, _, status)) => {
            gov.job_finished(job, Some(*id), None)?;
            gov.audit(&auth.name, auth.role, "run.create", &format!("{name}#{id}"), "ok")?;
            (
                status.clone().unwrap_or_else(|| "pass".into()),
                Some(*id),
                String::new(),
            )
        }
        Err(e) => {
            gov.job_finished(job, None, Some(e))?;
            gov.audit(&auth.name, auth.role, "run.create", name, &format!("error: {e}"))?;
            ("error".to_string(), None, e.clone())
        }
    };
    if let Some(cfg) = &st.0.alerts {
        let ev = alerts::Event {
            pipeline: name,
            run_id,
            kind: &event_kind,
            detail: &detail,
            triggered_by: &auth.name,
            link: link(run_id),
        };
        match alerts::notify(cfg, &ev) {
            Ok(true) => gov.audit("system", Role::Admin, "alert.send", name, &event_kind)?,
            Ok(false) => {}
            Err(e) => gov.audit("system", Role::Admin, "alert.send", name, &format!("error: {e}"))?,
        }
    }
    result.map(|(id, v, _)| (id, v))
}

/// Records a job and queues it (execution bounded by the semaphore).
async fn enqueue(
    st: &AppState,
    name: &str,
    auth: &Auth,
) -> ApiResult<(i64, tokio::task::JoinHandle<Result<(i64, Value), String>>)> {
    let (st2, name2, user) = (st.clone(), name.to_string(), auth.name.clone());
    let job = blocking(move || {
        locate(&st2, &name2)?;
        st2.gov()?.create_job(&name2, &user).map_err(ApiError::internal)
    })
    .await?;
    let (st3, name3, auth3) = (st.clone(), name.to_string(), auth.clone());
    let sem = st.0.runs.clone();
    let task = tokio::spawn(async move {
        let _permit = sem.acquire_owned().await.expect("semaphore");
        tokio::task::spawn_blocking(move || execute_job(&st3, job, &name3, &auth3))
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
    });
    Ok((job, task))
}

/// One scheduler pass at time `now`: claims the due slots and starts the runs.
/// Returns the created jobs.
pub async fn scheduler_tick(st: &AppState, now: i64) -> Result<Vec<i64>, String> {
    let st2 = st.clone();
    let due = tokio::task::spawn_blocking(move || -> Result<Vec<String>, String> {
        let gov = st2.gov().map_err(|e| e.1)?;
        let mut due = vec![];
        for (name, toml) in discover(&st2.0.workspace).map_err(|e| e.1)? {
            let Ok((sp, _)) = spec::load(&toml) else { continue };
            let Some(expr) = sp.schedule else { continue };
            let cron = match scheduler::parse(&expr) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{name}: {e}");
                    continue;
                }
            };
            match gov.schedule_last(&name)? {
                None => {
                    gov.schedule_claim(&name, None, now)?;
                }
                Some(last) => {
                    if let Some(slot) = scheduler::latest_due(&cron, last, now) {
                        if gov.schedule_claim(&name, Some(last), slot)? {
                            gov.audit(
                                "scheduler",
                                Role::ModelOwner,
                                "schedule.trigger",
                                &name,
                                &format!("slot {slot}"),
                            )?;
                            due.push(name.clone());
                        }
                    }
                }
            }
        }
        Ok(due)
    })
    .await
    .map_err(|e| e.to_string())??;
    let system = Auth {
        name: "scheduler".into(),
        role: Role::ModelOwner,
    };
    let mut jobs = vec![];
    for name in due {
        jobs.push(enqueue(st, &name, &system).await.map_err(|e| e.1)?.0);
    }
    Ok(jobs)
}

/// Starts the scheduler loop (if enabled).
pub fn start_scheduler(st: AppState) {
    if !st.0.scheduler.enabled {
        return;
    }
    let every = std::time::Duration::from_secs(st.0.scheduler.interval_secs.max(1));
    tokio::spawn(async move {
        loop {
            let now = chrono::Utc::now().timestamp();
            if let Err(e) = scheduler_tick(&st, now).await {
                eprintln!("scheduler: {e}");
            }
            tokio::time::sleep(every).await;
        }
    });
}

async fn create_run(
    State(st): State<AppState>,
    auth: Auth,
    Path(name): Path<String>,
    Query(q): Query<RunQuery>,
) -> ApiResult<Response> {
    authorize(&st, &auth, &[Role::ModelOwner, Role::Admin], "run.create", &name).await?;
    let (job, task) = enqueue(&st, &name, &auth).await?;
    if !q.wait {
        let body = json!({"job_id": job, "status": "queued", "href": format!("/api/jobs/{job}")});
        return Ok((StatusCode::ACCEPTED, Json(body)).into_response());
    }
    match task.await.map_err(|e| ApiError::internal(e.to_string()))? {
        Ok((_, body)) => Ok((StatusCode::CREATED, Json(body)).into_response()),
        Err(e) => Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e)),
    }
}

async fn get_job(State(st): State<AppState>, _auth: Auth, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    blocking(move || {
        st.gov()?
            .job(id)
            .map_err(ApiError::internal)?
            .map(Json)
            .ok_or_else(|| ApiError::not_found(format!("job #{id} not found")))
    })
    .await
}

async fn get_run(
    State(st): State<AppState>,
    _auth: Auth,
    Path((name, id)): Path<(String, i64)>,
) -> ApiResult<Json<Value>> {
    blocking(move || {
        let (_, m) = st
            .store_for(&locate(&st, &name)?)?
            .get(&name, Some(id))
            .map_err(ApiError::not_found)?;
        Ok(Json(serde_json::to_value(m).unwrap()))
    })
    .await
}

async fn verify(
    State(st): State<AppState>,
    auth: Auth,
    Path((name, id)): Path<(String, i64)>,
) -> ApiResult<Json<Value>> {
    let target = format!("{name}#{id}");
    authorize(
        &st,
        &auth,
        &[Role::Validator, Role::ModelOwner, Role::Admin],
        "run.verify",
        &target,
    )
    .await?;
    blocking(move || {
        let mm = verify_run(&st, &locate(&st, &name)?, &name, id)?;
        let outcome = if mm.is_empty() { "reproducible" } else { "mismatch" };
        st.gov()?
            .audit(&auth.name, auth.role, "run.verify", &target, outcome)
            .map_err(ApiError::internal)?;
        Ok(Json(
            json!({"run_id": id, "reproducible": mm.is_empty(), "mismatches": mismatches_json(&mm)}),
        ))
    })
    .await
}

#[derive(Deserialize)]
struct DiffQuery {
    left: i64,
    right: i64,
    /// `json` (default) or `md`.
    #[serde(default)]
    format: Option<String>,
}

/// Comparison of two runs of a pipeline, with the pipeline's tolerances.
async fn diff_runs(
    State(st): State<AppState>,
    _auth: Auth,
    Path(name): Path<String>,
    Query(q): Query<DiffQuery>,
) -> ApiResult<Response> {
    blocking(move || {
        let toml = locate(&st, &name)?;
        let (sp, _) = spec::load(&toml).map_err(ApiError::internal)?;
        let store = st.store_for(&toml)?;
        let (_, a) = store.get(&name, Some(q.left)).map_err(ApiError::not_found)?;
        let (_, b) = store.get(&name, Some(q.right)).map_err(ApiError::not_found)?;
        let d = mf_core::diff::diff(&a, &b, &sp.tolerances);
        let (l, r) = (format!("{name}#{}", q.left), format!("{name}#{}", q.right));
        match q.format.as_deref().unwrap_or("json") {
            "json" => Ok(Json(json!({
                "left": l, "right": r, "verdict": mf_runtime::diff::verdict(&d), "diff": d,
            }))
            .into_response()),
            "md" => Ok((
                [(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
                mf_runtime::diff::render_markdown(&d, &l, &r, 10),
            )
                .into_response()),
            f => Err(ApiError::new(StatusCode::BAD_REQUEST, format!("unknown format `{f}`"))),
        }
    })
    .await
}

#[derive(Deserialize)]
struct ReportQuery {
    #[serde(default)]
    no_verify: bool,
}

async fn get_report(
    State(st): State<AppState>,
    _auth: Auth,
    Path((name, id)): Path<(String, i64)>,
    Query(q): Query<ReportQuery>,
) -> ApiResult<Html<String>> {
    blocking(move || {
        let toml = locate(&st, &name)?;
        let store = st.store_for(&toml)?;
        let (_, m) = store.get(&name, Some(id)).map_err(ApiError::not_found)?;
        let info = store
            .info(&name, id)
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("run not found"))?;
        let mm = if q.no_verify {
            None
        } else {
            Some(verify_run(&st, &toml, &name, id)?)
        };
        let decisions = st.gov()?.decisions(&name, id).map_err(ApiError::internal)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let meta = report::ReportMeta {
            pipeline: &name,
            run_id: id,
            created_at: info.created_at,
            generated_at: now,
            replay: mm.as_deref(),
            decisions: &decisions,
        };
        Ok(Html(report::render_html(&meta, &m)))
    })
    .await
}

#[derive(Deserialize)]
struct DecisionBody {
    decision: String,
    #[serde(default)]
    comment: String,
}

async fn decide(
    State(st): State<AppState>,
    auth: Auth,
    Path((name, id)): Path<(String, i64)>,
    Json(body): Json<DecisionBody>,
) -> ApiResult<Response> {
    let target = format!("{name}#{id}");
    authorize(&st, &auth, &[Role::Validator], "run.decide", &target).await?;
    if body.decision != "approved" && body.decision != "rejected" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "`decision` must be `approved` or `rejected`",
        ));
    }
    if body.decision == "rejected" && body.comment.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "a rejection must be justified (`comment`)",
        ));
    }
    blocking(move || {
        let toml = locate(&st, &name)?;
        let gov = st.gov()?;
        let info = st
            .store_for(&toml)?
            .info(&name, id)
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("run #{id} not found")))?;
        let refuse = |why: &str| {
            let _ = gov.audit(&auth.name, auth.role, "run.decide", &target, &format!("refused: {why}"));
            Err(ApiError::new(StatusCode::CONFLICT, why.to_string()))
        };
        if info.triggered_by == auth.name {
            return refuse("segregation of duties: you cannot validate your own run");
        }
        let mm = verify_run(&st, &toml, &name, id)?;
        if !mm.is_empty() {
            return refuse("the replay is not identical: no decision possible");
        }
        let did = gov
            .decide(&name, id, &body.decision, &body.comment, &auth.name)
            .map_err(ApiError::internal)?;
        gov.audit(&auth.name, auth.role, "run.decide", &target, &body.decision)
            .map_err(ApiError::internal)?;
        Ok((StatusCode::CREATED, Json(json!({"id": did, "decision": body.decision}))).into_response())
    })
    .await
}

async fn list_decisions(
    State(st): State<AppState>,
    _auth: Auth,
    Path((name, id)): Path<(String, i64)>,
) -> ApiResult<Json<Value>> {
    blocking(move || {
        locate(&st, &name)?;
        let d = st.gov()?.decisions(&name, id).map_err(ApiError::internal)?;
        Ok(Json(serde_json::to_value(d).unwrap()))
    })
    .await
}

#[derive(Deserialize)]
struct AuditQuery {
    limit: Option<u32>,
}

async fn audit_log(State(st): State<AppState>, auth: Auth, Query(q): Query<AuditQuery>) -> ApiResult<Json<Value>> {
    authorize(&st, &auth, &[Role::Auditor, Role::Admin], "audit.read", "audit").await?;
    blocking(move || {
        let rows = st
            .gov()?
            .audit_log(q.limit.unwrap_or(100).min(10_000))
            .map_err(ApiError::internal)?;
        Ok(Json(json!(rows)))
    })
    .await
}

// ---------- Model inventory ----------

#[derive(Deserialize)]
struct ModelBody {
    id: String,
    name: String,
    owner: String,
    #[serde(default)]
    purpose: String,
    tier: i64,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    next_review: Option<String>,
}

fn bad_request(m: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, m)
}

fn validate_model(b: &ModelBody) -> ApiResult<()> {
    if b.id.is_empty() || b.id.len() > 64 || !b.id.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        return Err(bad_request(
            "`id`: 1 to 64 characters among letters, digits, `.`, `_`, `-`",
        ));
    }
    if b.name.trim().is_empty() || b.owner.trim().is_empty() {
        return Err(bad_request("`name` and `owner` are required"));
    }
    if !(1..=3).contains(&b.tier) {
        return Err(bad_request("`tier`: 1 (high risk), 2 or 3 (low)"));
    }
    if let Some(d) = &b.next_review {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| bad_request("`next_review`: date YYYY-MM-DD"))?;
    }
    Ok(())
}

async fn list_models(State(st): State<AppState>, _auth: Auth) -> ApiResult<Json<Value>> {
    blocking(move || {
        let gov = st.gov()?;
        let today = chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string();
        let mut linked: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for (name, toml) in discover(&st.0.workspace)? {
            let Ok((sp, _)) = spec::load(&toml) else { continue };
            let Some(model) = sp.model else { continue };
            let runs = st.store_for(&toml)?.list(&name).map_err(ApiError::internal)?;
            let last = runs.last().map(|r| {
                let decision = gov
                    .decisions(&name, r.id)
                    .ok()
                    .and_then(|d| d.last().map(|x| x.decision.clone()));
                json!({"id": r.id, "status": r.status, "decision": decision, "created_at": r.created_at})
            });
            linked
                .entry(model)
                .or_default()
                .push(json!({"pipeline": name, "schedule": sp.schedule, "last_run": last}));
        }
        let out: Vec<Value> = gov
            .models()
            .map_err(ApiError::internal)?
            .into_iter()
            .map(|m| {
                let overdue = m.next_review.as_deref().is_some_and(|d| d < today.as_str()) && m.status != "retired";
                let mut v = serde_json::to_value(&m).unwrap();
                v["pipelines"] = json!(linked.remove(&m.id).unwrap_or_default());
                v["review_overdue"] = json!(overdue);
                v
            })
            .collect();
        Ok(Json(
            json!({"models": out, "unregistered": linked.keys().collect::<Vec<_>>()}),
        ))
    })
    .await
}

async fn create_model(State(st): State<AppState>, auth: Auth, Json(b): Json<ModelBody>) -> ApiResult<Response> {
    authorize(&st, &auth, &[Role::ModelOwner, Role::Admin], "model.create", &b.id).await?;
    validate_model(&b)?;
    if auth.role == Role::ModelOwner && b.owner != auth.name {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "an owner can only register their own models",
        ));
    }
    let status = b.status.clone().unwrap_or_else(|| "development".into());
    if !["development", "in_validation"].contains(&status.as_str()) {
        return Err(bad_request("initial status: `development` or `in_validation`"));
    }
    blocking(move || {
        let gov = st.gov()?;
        let m = governance::ModelRecord {
            id: b.id.clone(),
            name: b.name,
            owner: b.owner,
            purpose: b.purpose,
            tier: b.tier,
            status,
            next_review: b.next_review,
            updated_at: 0,
            updated_by: auth.name.clone(),
        };
        if !gov.create_model(&m, &auth.name).map_err(ApiError::internal)? {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                format!("model `{}` already exists", m.id),
            ));
        }
        gov.audit(&auth.name, auth.role, "model.create", &m.id, "ok")
            .map_err(ApiError::internal)?;
        Ok((StatusCode::CREATED, Json(serde_json::to_value(&m).unwrap())).into_response())
    })
    .await
}

fn find_model(gov: &Gov, id: &str) -> ApiResult<governance::ModelRecord> {
    gov.models()
        .map_err(ApiError::internal)?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| ApiError::not_found(format!("model `{id}` not found")))
}

/// Updates the model record (except the status, which follows its own workflow).
async fn update_model(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(mut b): Json<ModelBody>,
) -> ApiResult<Json<Value>> {
    authorize(&st, &auth, &[Role::ModelOwner, Role::Admin], "model.update", &id).await?;
    b.id = id.clone();
    validate_model(&b)?;
    blocking(move || {
        let gov = st.gov()?;
        let cur = find_model(&gov, &id)?;
        if auth.role == Role::ModelOwner && cur.owner != auth.name {
            let _ = gov.audit(&auth.name, auth.role, "model.update", &id, "denied");
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "only the model owner (or an admin) can modify it",
            ));
        }
        let m = governance::ModelRecord {
            id: id.clone(),
            name: b.name,
            owner: b.owner,
            purpose: b.purpose,
            tier: b.tier,
            status: cur.status,
            next_review: b.next_review,
            updated_at: 0,
            updated_by: auth.name.clone(),
        };
        gov.update_model(&m, &auth.name).map_err(ApiError::internal)?;
        gov.audit(&auth.name, auth.role, "model.update", &id, "ok")
            .map_err(ApiError::internal)?;
        Ok(Json(serde_json::to_value(find_model(&gov, &id)?).unwrap()))
    })
    .await
}

#[derive(Deserialize)]
struct StatusBody {
    status: String,
}

/// Status workflow: the validator approves (or sends back to validation); the owner or
/// the admin handles development, deployment to use (after approval) and retirement.
async fn set_model_status(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<StatusBody>,
) -> ApiResult<Json<Value>> {
    authorize(
        &st,
        &auth,
        &[Role::ModelOwner, Role::Admin, Role::Validator],
        "model.status",
        &id,
    )
    .await?;
    if !governance::MODEL_STATUSES.contains(&b.status.as_str()) {
        return Err(bad_request(format!(
            "unknown status; values: {}",
            governance::MODEL_STATUSES.join(", ")
        )));
    }
    blocking(move || {
        let gov = st.gov()?;
        let mut m = find_model(&gov, &id)?;
        let to = b.status.as_str();
        let allowed = match auth.role {
            Role::Validator => ["approved", "in_validation"].contains(&to) && m.status != "retired",
            Role::ModelOwner | Role::Admin => {
                let own = auth.role == Role::Admin || m.owner == auth.name;
                own && match to {
                    "development" | "in_validation" | "retired" => true,
                    "in_use" => ["approved", "in_use"].contains(&m.status.as_str()),
                    _ => false, // "approved" is reserved for the validator
                }
            }
            Role::Auditor => false,
        };
        let change = format!("{} -> {to}", m.status);
        if allowed && to == "approved" {
            // Model approval relies on validated runs: the latest run of
            // every linked pipeline must have been approved.
            let mut blockers = vec![];
            let mut linked = 0;
            for (name, toml) in discover(&st.0.workspace)? {
                let Ok((sp, _)) = spec::load(&toml) else { continue };
                if sp.model.as_deref() != Some(id.as_str()) {
                    continue;
                }
                linked += 1;
                let last = st.store_for(&toml)?.list(&name).map_err(ApiError::internal)?.pop();
                let decision = last
                    .as_ref()
                    .and_then(|r| gov.decisions(&name, r.id).ok())
                    .and_then(|d| d.last().map(|x| x.decision.clone()));
                match (last, decision.as_deref()) {
                    (None, _) => blockers.push(format!("{name}: no run")),
                    (Some(r), Some("approved")) => drop(r),
                    (Some(r), Some(d)) => blockers.push(format!("{name} #{}: latest run {d}", r.id)),
                    (Some(r), None) => blockers.push(format!("{name} #{}: awaiting decision", r.id)),
                }
            }
            if linked == 0 {
                blockers.push("no validation pipeline declares this model".into());
            }
            if !blockers.is_empty() {
                let _ = gov.audit(
                    &auth.name,
                    auth.role,
                    "model.status",
                    &id,
                    &format!("refused: {change}"),
                );
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    format!("approval not possible: {}", blockers.join("; ")),
                ));
            }
        }
        if !allowed {
            let _ = gov.audit(&auth.name, auth.role, "model.status", &id, &format!("denied: {change}"));
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                format!("status change not allowed ({change})"),
            ));
        }
        m.status = to.to_string();
        gov.update_model(&m, &auth.name).map_err(ApiError::internal)?;
        gov.audit(&auth.name, auth.role, "model.status", &id, &change)
            .map_err(ApiError::internal)?;
        Ok(Json(serde_json::to_value(find_model(&gov, &id)?).unwrap()))
    })
    .await
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(ui_index))
        .route("/ui/app.js", get(ui_js))
        .route("/ui/app.css", get(ui_css))
        .route("/api/ui-config", get(ui_config))
        .route("/api/health", get(health))
        .route("/api/me", get(me))
        .route("/api/pipelines", get(list_pipelines))
        .route("/api/pipelines/:name/runs", get(list_runs).post(create_run))
        .route("/api/pipelines/:name/runs/:id", get(get_run))
        .route("/api/pipelines/:name/diff", get(diff_runs))
        .route("/api/pipelines/:name/runs/:id/verify", post(verify))
        .route("/api/pipelines/:name/runs/:id/report", get(get_report))
        .route(
            "/api/pipelines/:name/runs/:id/decisions",
            get(list_decisions).post(decide),
        )
        .route("/api/jobs/:id", get(get_job))
        .route("/api/models", get(list_models).post(create_model))
        .route("/api/models/:id", axum::routing::put(update_model))
        .route("/api/models/:id/status", post(set_model_status))
        .route("/api/audit", get(audit_log))
        .layer(axum::middleware::from_fn_with_state(state.clone(), security_headers))
        .with_state(state)
}
