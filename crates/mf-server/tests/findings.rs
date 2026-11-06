//! Validation findings: lifecycle, permissions, evidence, approval blocking, escalation, report.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mf_server::{hash_token, router, scheduler_tick, AppState, ServerConfig};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

fn workspace(tag: &str) -> PathBuf {
    let ws = std::env::temp_dir().join(format!("mf-find-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    let dir = ws.join("desk");
    std::fs::create_dir_all(&dir).unwrap();
    let rows: String = (0..600)
        .map(|i| format!("{i},{}\n", ((i * 7919 % 1000) as f64 - 500.0) / 25_000.0))
        .collect();
    std::fs::write(dir.join("returns.csv"), format!("day,ret\n{rows}")).unwrap();
    std::fs::write(
        dir.join("pipeline.toml"),
        "name = \"desk-var\"\nmodel = \"VAR-DESK-01\"\n[[step]]\nname = \"returns\"\nkind = \"csv\"\nversion = \"1\"\npath = \"returns.csv\"\ncolumn = \"ret\"\n[[step]]\nname = \"backtest\"\nkind = \"kupiec_backtest\"\nversion = \"1\"\ndeps = [\"returns\"]\nparams = { alpha = 0.99, window = 250 }\n",
    )
    .unwrap();
    ws
}

fn state(ws: &Path, extra: &str) -> AppState {
    let mut cfg = format!(
        "workspace = \"{}\"\npublic_url = \"https://modelflow.example\"\n[scheduler]\nenabled = false\n",
        ws.display()
    );
    for (name, role) in [
        ("alice", "model_owner"),
        ("zoe", "model_owner"),
        ("bob", "validator"),
        ("dave", "validator"),
        ("carol", "auditor"),
    ] {
        cfg += &format!(
            "[[user]]\nname = \"{name}\"\nrole = \"{role}\"\ntoken_sha256 = \"{}\"\n",
            hash_token(&format!("tok-{name}"))
        );
    }
    cfg += extra;
    AppState::new(&ServerConfig::parse(&cfg, ws).unwrap()).unwrap()
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
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
    let text = String::from_utf8_lossy(&bytes).to_string();
    (s, serde_json::from_str(&text).unwrap_or(Value::Null), text)
}

async fn act(app: &axum::Router, id: i64, token: &str, body: Value) -> (StatusCode, Value) {
    let (s, v, _) = call(app, "POST", &format!("/api/findings/{id}/actions"), token, Some(body)).await;
    (s, v)
}

async fn webhook() -> (String, Arc<Mutex<Vec<Value>>>) {
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
    (url, received)
}

#[tokio::test(flavor = "multi_thread")]
async fn findings_lifecycle_sqlite() {
    findings_lifecycle(None).await
}

#[tokio::test(flavor = "multi_thread")]
async fn findings_lifecycle_postgres() {
    let Ok(admin) = std::env::var("MODELFLOW_TEST_DATABASE_URL") else {
        eprintln!("MODELFLOW_TEST_DATABASE_URL not set: PostgreSQL test skipped");
        return;
    };
    let name = format!("mf_find_{}", std::process::id());
    let url = tokio::task::spawn_blocking(move || mf_runtime::pg::create_test_database(&admin, &name).unwrap())
        .await
        .unwrap();
    findings_lifecycle(Some(url)).await
}

async fn findings_lifecycle(db: Option<String>) {
    let (hook, received) = webhook().await;
    let ws = workspace(if db.is_some() { "pg" } else { "lite" });
    let database = db
        .map(|u| format!("[database]\nurl = \"{u}\"\nmax_connections = 4\n"))
        .unwrap_or_default();
    let st = state(&ws, &format!("[alerts]\nwebhook_url = \"{hook}\"\n{database}"));
    let app = router(st.clone());
    let model = json!({"id": "VAR-DESK-01", "name": "Historical VaR", "owner": "alice", "tier": 1});
    assert_eq!(
        call(&app, "POST", "/api/models", "tok-alice", Some(model)).await.0,
        StatusCode::CREATED
    );
    let (s, run, _) = call(
        &app,
        "POST",
        "/api/pipelines/desk-var/runs?wait=true",
        "tok-alice",
        None,
    )
    .await;
    assert!(s.is_success(), "{s} {run}");

    // Raising: validators only, on an existing model and run; deadline from the severity.
    let new = json!({"model": "VAR-DESK-01", "title": "Backtest window too short", "severity": "high",
                     "description": "250 days do not cover a stress period", "pipeline": "desk-var", "run_id": 1,
                     "step": "backtest"});
    assert_eq!(
        call(&app, "POST", "/api/findings", "tok-alice", Some(new.clone()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut bad = new.clone();
    bad["severity"] = json!("severe");
    assert_eq!(
        call(&app, "POST", "/api/findings", "tok-bob", Some(bad)).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut ghost = new.clone();
    ghost["run_id"] = json!(9);
    assert_eq!(
        call(&app, "POST", "/api/findings", "tok-bob", Some(ghost)).await.0,
        StatusCode::NOT_FOUND
    );
    let (s, f1, _) = call(&app, "POST", "/api/findings", "tok-bob", Some(new)).await;
    assert_eq!(
        (s, f1["status"].as_str(), f1["owner"].as_str()),
        (StatusCode::CREATED, Some("open"), Some("alice"))
    );
    let due = chrono::NaiveDate::parse_from_str(f1["due_date"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    assert_eq!((due - chrono::Utc::now().date_naive()).num_days(), 90);
    let id1 = f1["id"].as_i64().unwrap();

    // A high finding blocks the model's approval, even with an approved run.
    assert!(call(
        &app,
        "POST",
        "/api/models/VAR-DESK-01/status",
        "tok-alice",
        Some(json!({"status": "in_validation"}))
    )
    .await
    .0
    .is_success());
    let (s, d, _) = call(
        &app,
        "POST",
        "/api/pipelines/desk-var/runs/1/decisions",
        "tok-bob",
        Some(json!({"decision": "approved", "comment": "ok"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    let (s, e, _) = call(
        &app,
        "POST",
        "/api/models/VAR-DESK-01/status",
        "tok-bob",
        Some(json!({"status": "approved"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(
        e["error"]
            .as_str()
            .unwrap()
            .contains(&format!("finding #{id1} (high, open)")),
        "{e}"
    );
    let (_, models, _) = call(&app, "GET", "/api/models", "tok-carol", None).await;
    assert_eq!(
        models["models"][0]["findings"],
        json!({"active": 1, "overdue": 0, "blocking": 1})
    );

    // Remediation: the remediation owner only, with a run that replays identically as evidence.
    assert_eq!(
        act(&app, id1, "tok-zoe", json!({"action": "plan", "comment": "x"}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        act(&app, id1, "tok-carol", json!({"action": "comment", "comment": "x"}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        act(&app, id1, "tok-alice", json!({"action": "plan", "comment": ""}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (s, f) = act(
        &app,
        id1,
        "tok-alice",
        json!({"action": "plan", "comment": "Extend to 1,000 days"}),
    )
    .await;
    assert_eq!(
        (s, f["remediation"].as_str()),
        (StatusCode::OK, Some("Extend to 1,000 days"))
    );
    assert_eq!(
        act(&app, id1, "tok-bob", json!({"action": "close", "comment": "x"}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (s, _) = act(
        &app,
        id1,
        "tok-alice",
        json!({"action": "remediate", "comment": "done", "evidence": "desk-var#7"}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, f) = act(
        &app,
        id1,
        "tok-alice",
        json!({"action": "remediate", "comment": "Window extended", "evidence": "desk-var#1"}),
    )
    .await;
    assert_eq!(
        (s, f["status"].as_str(), f["evidence"].as_str()),
        (StatusCode::OK, Some("remediated"), Some("desk-var#1"))
    );
    // Closure review: validators only.
    assert_eq!(
        act(&app, id1, "tok-alice", json!({"action": "close", "comment": "x"}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (s, f) = act(
        &app,
        id1,
        "tok-dave",
        json!({"action": "close", "comment": "Verified on run #1"}),
    )
    .await;
    assert_eq!(
        (s, f["status"].as_str(), f["closed_by"].as_str()),
        (StatusCode::OK, Some("closed"), Some("dave"))
    );
    let actions: Vec<&str> = f["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["raise", "plan", "remediate", "close"]);
    assert!(f["events"][2]["comment"]
        .as_str()
        .unwrap()
        .contains("evidence: desk-var#1"));
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/models/VAR-DESK-01/status",
        "tok-bob",
        Some(json!({"status": "approved"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // An overdue finding is escalated once, by the scheduler pass; a new deadline resets it.
    let late = json!({"model": "VAR-DESK-01", "title": "Undocumented proxy", "severity": "medium",
                      "description": "Proxy series not documented", "due_date": "2020-01-31"});
    let (_, f2, _) = call(&app, "POST", "/api/findings", "tok-bob", Some(late)).await;
    let id2 = f2["id"].as_i64().unwrap();
    assert!(f2["overdue_days"].as_i64().unwrap() > 2000);
    let (_, list, _) = call(&app, "GET", "/api/findings?overdue=true", "tok-carol", None).await;
    assert_eq!(list["findings"].as_array().unwrap().len(), 1);
    let now = chrono::Utc::now().timestamp();
    scheduler_tick(&st, now).await.unwrap();
    scheduler_tick(&st, now + 60).await.unwrap();
    let got = received.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0]["status"].as_str(), got[0]["finding_id"].as_i64()),
        (Some("finding_overdue"), Some(id2))
    );
    assert_eq!(got[0]["link"], format!("https://modelflow.example/#/findings/{id2}"));
    assert_eq!(
        act(
            &app,
            id2,
            "tok-bob",
            json!({"action": "extend", "comment": "x", "due_date": "2019-01-01"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (s, f) = act(
        &app,
        id2,
        "tok-bob",
        json!({"action": "extend", "comment": "New data source", "due_date": "2021-01-31"}),
    )
    .await;
    assert_eq!(
        (s, f["due_date"].as_str(), f["escalated_at"].is_null()),
        (StatusCode::OK, Some("2021-01-31"), true)
    );
    scheduler_tick(&st, now + 120).await.unwrap();
    assert_eq!(received.lock().unwrap().len(), 2);

    // Reopen after an insufficient remediation, then accept the risk.
    act(
        &app,
        id2,
        "tok-alice",
        json!({"action": "remediate", "comment": "Documented"}),
    )
    .await;
    let (s, f) = act(
        &app,
        id2,
        "tok-bob",
        json!({"action": "reopen", "comment": "The document is a draft"}),
    )
    .await;
    assert_eq!(
        (s, f["status"].as_str(), f["remediated_by"].is_null()),
        (StatusCode::OK, Some("open"), true)
    );
    let (s, f) = act(
        &app,
        id2,
        "tok-bob",
        json!({"action": "accept", "comment": "Immaterial, accepted by the committee"}),
    )
    .await;
    assert_eq!((s, f["status"].as_str()), (StatusCode::OK, Some("risk_accepted")));
    assert_eq!(
        act(&app, id2, "tok-bob", json!({"action": "reopen", "comment": "x"}))
            .await
            .0,
        StatusCode::CONFLICT
    );

    // Governance report and audit trail.
    let (_, r, _) = call(&app, "GET", "/api/findings/report", "tok-carol", None).await;
    assert_eq!(
        (
            r["counts"]["closed"]["high"].as_u64(),
            r["counts"]["risk_accepted"]["medium"].as_u64()
        ),
        (Some(1), Some(1))
    );
    assert_eq!(r["closed_last_90_days"], 2);
    let (s, _, md) = call(&app, "GET", "/api/findings/report?format=md", "tok-carol", None).await;
    assert!(s.is_success() && md.contains("0 active findings, 0 overdue"), "{md}");
    let (_, one, _) = call(&app, "GET", &format!("/api/findings/{id2}"), "tok-carol", None).await;
    let actions: Vec<&str> = one["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "raise",
            "escalate",
            "extend",
            "escalate",
            "remediate",
            "reopen",
            "accept"
        ]
    );
    let (_, log, _) = call(&app, "GET", "/api/audit?limit=200", "tok-carol", None).await;
    let log = log.to_string();
    for a in [
        "finding.raise",
        "finding.remediate",
        "finding.close",
        "finding.escalate",
        "finding.accept",
    ] {
        assert!(log.contains(a), "{a} missing from the audit log");
    }
    assert!(log.contains("refused: `close` is not possible: the finding is open"));
    let _ = std::fs::remove_dir_all(&ws);
}
