//! Inventaire des modèles, planification et alertes.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mf_server::{hash_token, router, scheduler_tick, AppState, ServerConfig};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

fn workspace(tag: &str, schedule: &str, failing: bool) -> PathBuf {
    let ws = std::env::temp_dir().join(format!("mf-inv-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    let dir = ws.join("desk");
    std::fs::create_dir_all(&dir).unwrap();
    let rows: String = (0..600)
        .map(|i| format!("{i},{}\n", ((i * 7919 % 1000) as f64 - 500.0) / 25_000.0))
        .collect();
    std::fs::write(dir.join("returns.csv"), format!("day,ret\n{rows}")).unwrap();
    let extra = if failing {
        "[[step]]\nname = \"broken\"\nkind = \"process\"\nversion = \"1\"\ncommand = [\"sh\", \"-c\", \"echo panne >&2; exit 2\"]\n"
    } else {
        ""
    };
    std::fs::write(
        dir.join("pipeline.toml"),
        format!(
            "name = \"desk-var\"\nmodel = \"VAR-DESK-01\"\n{schedule}\n[[step]]\nname = \"returns\"\nkind = \"csv\"\nversion = \"1\"\npath = \"returns.csv\"\ncolumn = \"ret\"\n[[step]]\nname = \"backtest\"\nkind = \"kupiec_backtest\"\nversion = \"1\"\ndeps = [\"returns\"]\nparams = {{ alpha = 0.99, window = 250 }}\n{extra}"
        ),
    )
    .unwrap();
    ws
}

fn state(ws: &Path, extra: &str) -> AppState {
    let cfg = format!(
        r#"workspace = "{}"
public_url = "https://modelflow.example"
[scheduler]
enabled = false
[[user]]
name = "alice"
role = "model_owner"
token_sha256 = "{}"
[[user]]
name = "zoe"
role = "model_owner"
token_sha256 = "{}"
[[user]]
name = "bob"
role = "validator"
token_sha256 = "{}"
[[user]]
name = "carol"
role = "auditor"
token_sha256 = "{}"
{extra}"#,
        ws.display(),
        hash_token("tok-alice"),
        hash_token("tok-zoe"),
        hash_token("tok-bob"),
        hash_token("tok-carol")
    );
    AppState::new(&ServerConfig::parse(&cfg, ws).unwrap()).unwrap()
}

async fn call(app: &axum::Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let s = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn model(owner: &str) -> Value {
    json!({"id": "VAR-DESK-01", "name": "VaR historique du desk actions", "owner": owner,
           "purpose": "Capital réglementaire risque de marché", "tier": 1, "next_review": "2020-01-31"})
}

#[tokio::test]
async fn inventory_rules() {
    let ws = workspace("rules", "", false);
    let app = router(state(&ws, ""));

    // Création : propriétaire pour lui-même uniquement, validation des champs.
    assert_eq!(
        call(&app, "POST", "/api/models", "tok-alice", Some(model("zoe")))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "POST", "/api/models", "tok-bob", Some(model("bob"))).await.0,
        StatusCode::FORBIDDEN
    );
    let mut bad = model("alice");
    bad["tier"] = json!(5);
    assert_eq!(
        call(&app, "POST", "/api/models", "tok-alice", Some(bad)).await.0,
        StatusCode::BAD_REQUEST
    );
    let (s, m) = call(&app, "POST", "/api/models", "tok-alice", Some(model("alice"))).await;
    assert_eq!((s, m["status"].as_str()), (StatusCode::CREATED, Some("development")));
    assert_eq!(
        call(&app, "POST", "/api/models", "tok-alice", Some(model("alice")))
            .await
            .0,
        StatusCode::CONFLICT
    );

    // Liste : pipeline rattaché et revue en retard.
    let (_, list) = call(&app, "GET", "/api/models", "tok-carol", None).await;
    let m = &list["models"][0];
    assert_eq!(
        (m["pipelines"][0]["pipeline"].as_str(), m["review_overdue"].as_bool()),
        (Some("desk-var"), Some(true))
    );

    // Modification : seul le propriétaire ; le statut n'est pas modifiable par cette voie.
    let mut upd = model("alice");
    upd["next_review"] = json!("2099-06-30");
    upd["status"] = json!("approved");
    assert_eq!(
        call(&app, "PUT", "/api/models/VAR-DESK-01", "tok-zoe", Some(upd.clone()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (s, m) = call(&app, "PUT", "/api/models/VAR-DESK-01", "tok-alice", Some(upd)).await;
    assert_eq!(
        (s, m["status"].as_str(), m["next_review"].as_str()),
        (StatusCode::OK, Some("development"), Some("2099-06-30"))
    );

    // Circuit des statuts.
    let st = |s: &str| Some(json!({"status": s}));
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/models/VAR-DESK-01/status",
            "tok-alice",
            st("approved")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/models/VAR-DESK-01/status",
            "tok-alice",
            st("in_use")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/models/VAR-DESK-01/status",
            "tok-alice",
            st("in_validation")
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", "/api/models/VAR-DESK-01/status", "tok-bob", st("in_use"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // L'approbation exige une dernière exécution approuvée sur chaque pipeline rattaché.
    let (s, e) = call(
        &app,
        "POST",
        "/api/models/VAR-DESK-01/status",
        "tok-bob",
        st("approved"),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(e["error"].as_str().unwrap().contains("aucune exécution"));
    call(
        &app,
        "POST",
        "/api/pipelines/desk-var/runs?wait=true",
        "tok-alice",
        None,
    )
    .await;
    let (s, e) = call(
        &app,
        "POST",
        "/api/models/VAR-DESK-01/status",
        "tok-bob",
        st("approved"),
    )
    .await;
    assert!(
        s == StatusCode::CONFLICT && e["error"].as_str().unwrap().contains("en attente"),
        "{e}"
    );
    let ok = Some(json!({"decision": "approved", "comment": "backtest conforme"}));
    assert_eq!(
        call(&app, "POST", "/api/pipelines/desk-var/runs/1/decisions", "tok-bob", ok)
            .await
            .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/models/VAR-DESK-01/status",
            "tok-bob",
            st("approved")
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", "/api/models/VAR-DESK-01/status", "tok-zoe", st("in_use"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (s, m) = call(
        &app,
        "POST",
        "/api/models/VAR-DESK-01/status",
        "tok-alice",
        st("in_use"),
    )
    .await;
    assert_eq!((s, m["status"].as_str()), (StatusCode::OK, Some("in_use")));
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/models/VAR-DESK-01/status",
            "tok-carol",
            st("retired")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let (_, log) = call(&app, "GET", "/api/audit", "tok-carol", None).await;
    let text = log.to_string();
    for needle in [
        "model.create",
        "development -> in_validation",
        "in_validation -> approved",
        "approved -> in_use",
        "denied: development -> approved",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
    let _ = std::fs::remove_dir_all(&ws);
}

async fn wait_job(app: &axum::Router, id: i64) -> Value {
    for _ in 0..400 {
        let (_, j) = call(app, "GET", &format!("/api/jobs/{id}"), "tok-carol", None).await;
        if j["status"] == "succeeded" || j["status"] == "failed" {
            return j;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("travail {id} non terminé");
}

#[tokio::test]
async fn scheduler_claims_due_slots_once() {
    let ws = workspace("sched", "schedule = \"*/5 * * * *\"", false);
    let st = state(&ws, "");
    let app = router(st.clone());
    let t0 = 1_800_000_000; // multiple de 300 s
    assert!(
        scheduler_tick(&st, t0 + 10).await.unwrap().is_empty(),
        "première découverte : rien n'est lancé"
    );
    assert!(
        scheduler_tick(&st, t0 + 200).await.unwrap().is_empty(),
        "créneau pas encore atteint"
    );
    let jobs = scheduler_tick(&st, t0 + 310).await.unwrap();
    assert_eq!(jobs.len(), 1);
    let j = wait_job(&app, jobs[0]).await;
    assert_eq!(
        (j["status"].as_str(), j["user"].as_str()),
        (Some("succeeded"), Some("planificateur"))
    );
    assert!(
        scheduler_tick(&st, t0 + 320).await.unwrap().is_empty(),
        "créneau déjà traité"
    );
    // Une heure d'interruption : un seul rattrapage (le dernier créneau).
    assert_eq!(scheduler_tick(&st, t0 + 3_900).await.unwrap().len(), 1);
    let (_, p) = call(&app, "GET", "/api/pipelines", "tok-carol", None).await;
    assert_eq!(p[0]["schedule"], "*/5 * * * *");
    assert!(p[0]["next_run"].as_i64().unwrap() > 0);
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduler_single_winner_across_instances() {
    let Ok(admin) = std::env::var("MODELFLOW_TEST_DATABASE_URL") else {
        eprintln!("MODELFLOW_TEST_DATABASE_URL absent : test PostgreSQL ignoré");
        return;
    };
    let name = format!("mf_sched_{}", std::process::id());
    let url = tokio::task::spawn_blocking(move || mf_runtime::pg::create_test_database(&admin, &name).unwrap())
        .await
        .unwrap();
    let ws = workspace("sched-pg", "schedule = \"*/5 * * * *\"", false);
    let db = format!("[database]\nurl = \"{url}\"\n");
    let (a, b) = (state(&ws, &db), state(&ws, &db));
    let t0 = 1_800_000_000;
    scheduler_tick(&a, t0 + 10).await.unwrap();
    for k in 1..=5 {
        let now = t0 + 300 * k + 5;
        let (ja, jb) = tokio::join!(scheduler_tick(&a, now), scheduler_tick(&b, now));
        assert_eq!(
            ja.unwrap().len() + jb.unwrap().len(),
            1,
            "créneau {k} : une seule instance l'exécute"
        );
    }
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test(flavor = "multi_thread")]
async fn alerts_on_failure() {
    let received: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = received.clone();
    let hook = axum::Router::new().route(
        "/hook",
        axum::routing::post(move |axum::Json(v): axum::Json<Value>| {
            let s = sink.clone();
            async move {
                s.lock().unwrap().push(v);
                "ok"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, hook).await.unwrap() });

    let ws = workspace("alert", "", true);
    let app = router(state(
        &ws,
        &format!("[alerts]\nwebhook_url = \"{url}\"\non = [\"error\", \"fail\"]\n"),
    ));
    let (s, _) = call(
        &app,
        "POST",
        "/api/pipelines/desk-var/runs?wait=true",
        "tok-alice",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let got = received.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0]["status"].as_str(), got[0]["pipeline"].as_str()),
        (Some("error"), Some("desk-var"))
    );
    assert!(got[0]["text"].as_str().unwrap().contains("panne"));
    assert_eq!(got[0]["link"], "https://modelflow.example/#/p/desk-var");
    let (_, log) = call(&app, "GET", "/api/audit", "tok-carol", None).await;
    assert!(log.to_string().contains("alert.send"));
    let _ = std::fs::remove_dir_all(&ws);
}
