//! API REST de ModelFlow : exécutions, rejeu, rapports, décisions de validation et journal d'audit.
//!
//! Authentification par jeton porteur (seule l'empreinte SHA-256 est stockée dans la configuration).
//! Rôles : `admin`, `model_owner`, `validator`, `auditor`. Séparation des tâches : seul un validateur
//! peut approuver ou rejeter, jamais sur une exécution qu'il a lancée, et uniquement si le rejeu est identique.

pub mod governance;

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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Dossier contenant un sous-dossier par pipeline (`<dossier>/pipeline.toml`).
    pub workspace: PathBuf,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(rename = "user", default)]
    pub users: Vec<UserConfig>,
}

impl ServerConfig {
    pub fn parse(text: &str, base: &FsPath) -> Result<Self, String> {
        let mut c: ServerConfig = toml::from_str(text).map_err(|e| format!("configuration invalide: {e}"))?;
        if c.workspace.is_relative() {
            c.workspace = base.join(&c.workspace);
        }
        let mut names = std::collections::HashSet::new();
        for u in &c.users {
            if !names.insert(&u.name) {
                return Err(format!("utilisateur en double: {}", u.name));
            }
            if u.token_sha256.len() != 64 || !u.token_sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(format!(
                    "{}: token_sha256 doit être une empreinte SHA-256 hexadécimale",
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

/// Jeton aléatoire de 256 bits.
pub fn new_token() -> String {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("source d'aléa");
    format!("mft_{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

// ---------- État et erreurs ----------

struct Inner {
    workspace: PathBuf,
    users: Vec<UserConfig>,
    gov_db: PathBuf,
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl AppState {
    pub fn new(cfg: &ServerConfig) -> Result<Self, String> {
        let gov_db = cfg.workspace.join(".modelflow-server/governance.db");
        Gov::open(&gov_db)?;
        Ok(Self(Arc::new(Inner {
            workspace: cfg.workspace.clone(),
            users: cfg.users.clone(),
            gov_db,
        })))
    }

    fn gov(&self) -> Result<Gov, ApiError> {
        Gov::open(&self.0.gov_db).map_err(ApiError::internal)
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

// ---------- Authentification ----------

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
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "jeton manquant (Authorization: Bearer …)"))?;
        let h = hash_token(token.trim());
        state
            .0
            .users
            .iter()
            .find(|u| u.token_sha256.eq_ignore_ascii_case(&h))
            .map(|u| Auth {
                name: u.name.clone(),
                role: u.role,
            })
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "jeton invalide"))
    }
}

/// Vérifie le rôle ; un refus est inscrit au journal d'audit.
fn authorize(state: &AppState, auth: &Auth, roles: &[Role], action: &str, target: &str) -> ApiResult<()> {
    if roles.contains(&auth.role) {
        return Ok(());
    }
    let _ = state
        .gov()
        .map(|g| g.audit(&auth.name, auth.role, action, target, "denied"));
    Err(ApiError::new(
        StatusCode::FORBIDDEN,
        format!("le rôle {:?} ne permet pas `{action}`", auth.role),
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
        .ok_or_else(|| ApiError::not_found(format!("pipeline `{name}` inconnu")))
}

fn store_for(toml: &FsPath) -> ApiResult<Store> {
    let base = toml.parent().unwrap_or(FsPath::new("."));
    Store::open(&base.join(".modelflow/runs.db")).map_err(ApiError::internal)
}

fn mismatches_json(mm: &[mf_core::Mismatch]) -> Value {
    json!(mm
        .iter()
        .map(|m| json!({"step": m.step, "reason": m.reason}))
        .collect::<Vec<_>>())
}

fn verify_run(toml: &FsPath, name: &str, id: i64) -> ApiResult<Vec<mf_core::Mismatch>> {
    let (sp, base) = spec::load(toml).map_err(ApiError::internal)?;
    let (_, m) = store_for(toml)?.get(name, Some(id)).map_err(ApiError::not_found)?;
    let p = spec::build(&sp, &base).map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    p.verify(&m)
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))
}

// ---------- Handlers ----------

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn me(auth: Auth) -> Json<Value> {
    Json(json!({ "name": auth.name, "role": auth.role }))
}

async fn list_pipelines(State(st): State<AppState>, _auth: Auth) -> ApiResult<Json<Value>> {
    let ws = st.0.workspace.clone();
    blocking(move || {
        let mut out = vec![];
        for (name, toml) in discover(&ws)? {
            let (sp, _) = spec::load(&toml).map_err(ApiError::internal)?;
            let steps: Vec<Value> = sp
                .steps
                .iter()
                .map(|s| json!({"name": s.name, "kind": s.kind, "version": s.version}))
                .collect();
            out.push(json!({ "name": name, "steps": steps }));
        }
        Ok(Json(json!(out)))
    })
    .await
}

async fn list_runs(State(st): State<AppState>, _auth: Auth, Path(name): Path<String>) -> ApiResult<Json<Value>> {
    blocking(move || {
        let toml = locate(&st, &name)?;
        let gov = st.gov()?;
        let rows = store_for(&toml)?.list(&name).map_err(ApiError::internal)?;
        let out: Vec<Value> = rows
            .into_iter()
            .map(|r| {
                let last = gov
                    .decisions(&name, r.id)
                    .ok()
                    .and_then(|d| d.last().map(|x| x.decision.clone()));
                json!({"id": r.id, "run_hash": r.run_hash, "created_at": r.created_at,
                       "triggered_by": r.triggered_by, "decision": last})
            })
            .collect();
        Ok(Json(json!(out)))
    })
    .await
}

async fn create_run(State(st): State<AppState>, auth: Auth, Path(name): Path<String>) -> ApiResult<Response> {
    authorize(&st, &auth, &[Role::ModelOwner, Role::Admin], "run.create", &name)?;
    blocking(move || {
        let toml = locate(&st, &name)?;
        let gov = st.gov()?;
        let (sp, base) = spec::load(&toml).map_err(ApiError::internal)?;
        let result = spec::build(&sp, &base).and_then(|p| p.execute().map_err(|e| e.to_string()));
        let m = match result {
            Ok(m) => m,
            Err(e) => {
                let _ = gov.audit(&auth.name, auth.role, "run.create", &name, &format!("error: {e}"));
                return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e));
            }
        };
        let id = store_for(&toml)?
            .save(&name, &m, &auth.name)
            .map_err(ApiError::internal)?;
        gov.audit(&auth.name, auth.role, "run.create", &format!("{name}#{id}"), "ok")
            .map_err(ApiError::internal)?;
        let steps: Vec<Value> = m
            .steps
            .iter()
            .map(|s| json!({"name": s.name, "version": s.version, "status": s.output.get("status")}))
            .collect();
        Ok((
            StatusCode::CREATED,
            Json(json!({"id": id, "run_hash": m.run_hash, "steps": steps})),
        )
            .into_response())
    })
    .await
}

async fn get_run(
    State(st): State<AppState>,
    _auth: Auth,
    Path((name, id)): Path<(String, i64)>,
) -> ApiResult<Json<Value>> {
    blocking(move || {
        let (_, m) = store_for(&locate(&st, &name)?)?
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
    )?;
    blocking(move || {
        let mm = verify_run(&locate(&st, &name)?, &name, id)?;
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
        let store = store_for(&toml)?;
        let (_, m) = store.get(&name, Some(id)).map_err(ApiError::not_found)?;
        let info = store
            .info(&name, id)
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("exécution"))?;
        let mm = if q.no_verify {
            None
        } else {
            Some(verify_run(&toml, &name, id)?)
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
    authorize(&st, &auth, &[Role::Validator], "run.decide", &target)?;
    if body.decision != "approved" && body.decision != "rejected" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "`decision` doit valoir `approved` ou `rejected`",
        ));
    }
    if body.decision == "rejected" && body.comment.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "un rejet doit être motivé (`comment`)",
        ));
    }
    blocking(move || {
        let toml = locate(&st, &name)?;
        let gov = st.gov()?;
        let info = store_for(&toml)?
            .info(&name, id)
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("exécution #{id} introuvable")))?;
        let refuse = |why: &str| {
            let _ = gov.audit(&auth.name, auth.role, "run.decide", &target, &format!("refused: {why}"));
            Err(ApiError::new(StatusCode::CONFLICT, why.to_string()))
        };
        if info.triggered_by == auth.name {
            return refuse("séparation des tâches : impossible de valider sa propre exécution");
        }
        let mm = verify_run(&toml, &name, id)?;
        if !mm.is_empty() {
            return refuse("le rejeu n'est pas identique : décision impossible");
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
    authorize(&st, &auth, &[Role::Auditor, Role::Admin], "audit.read", "audit")?;
    blocking(move || {
        let rows = st
            .gov()?
            .audit_log(q.limit.unwrap_or(100).min(10_000))
            .map_err(ApiError::internal)?;
        Ok(Json(json!(rows)))
    })
    .await
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/me", get(me))
        .route("/api/pipelines", get(list_pipelines))
        .route("/api/pipelines/:name/runs", get(list_runs).post(create_run))
        .route("/api/pipelines/:name/runs/:id", get(get_run))
        .route("/api/pipelines/:name/runs/:id/verify", post(verify))
        .route("/api/pipelines/:name/runs/:id/report", get(get_report))
        .route(
            "/api/pipelines/:name/runs/:id/decisions",
            get(list_decisions).post(decide),
        )
        .route("/api/audit", get(audit_log))
        .with_state(state)
}
