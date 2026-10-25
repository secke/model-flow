use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mf_server::{hash_token, router, AppState, ServerConfig};
use serde_json::{json, Value};
use std::path::PathBuf;
use tower::ServiceExt;

fn workspace(tag: &str) -> PathBuf {
    let ws = std::env::temp_dir().join(format!("mf-api-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    let dir = ws.join("desk");
    std::fs::create_dir_all(&dir).unwrap();
    let rows: String = (0..600)
        .map(|i| format!("{i},{}\n", ((i * 7919 % 1000) as f64 - 500.0) / 25_000.0))
        .collect();
    std::fs::write(dir.join("returns.csv"), format!("day,ret\n{rows}")).unwrap();
    std::fs::write(
        dir.join("pipeline.toml"),
        r#"name = "desk-var"
[[step]]
name = "returns"
kind = "csv"
version = "1"
path = "returns.csv"
column = "ret"
[[step]]
name = "backtest"
kind = "kupiec_backtest"
version = "1"
deps = ["returns"]
params = { alpha = 0.99, window = 250 }
"#,
    )
    .unwrap();
    ws
}

fn app(ws: &std::path::Path) -> axum::Router {
    app_with(ws, 120)
}

fn app_with(ws: &std::path::Path, rate: u32) -> axum::Router {
    let cfg = format!(
        r#"workspace = "{}"
rate_limit_per_minute = {rate}
[[user]]
name = "alice"
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
"#,
        ws.display(),
        hash_token("tok-alice"),
        hash_token("tok-bob"),
        hash_token("tok-carol")
    );
    let cfg = ServerConfig::parse(&cfg, ws).unwrap();
    router(AppState::new(&cfg).unwrap())
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null), text)
}

#[tokio::test]
async fn full_validation_workflow() {
    let ws = workspace("flow");
    let app = app(&ws);

    // Authentification
    assert_eq!(call(&app, "GET", "/api/health", None, None).await.0, StatusCode::OK);
    assert_eq!(
        call(&app, "GET", "/api/pipelines", None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "GET", "/api/pipelines", Some("faux"), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (s, me, _) = call(&app, "GET", "/api/me", Some("tok-bob"), None).await;
    assert_eq!((s, me["role"].as_str()), (StatusCode::OK, Some("validator")));

    let (s, list, _) = call(&app, "GET", "/api/pipelines", Some("tok-carol"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list[0]["name"], "desk-var");

    // Seul le propriétaire du modèle (ou un admin) lance une exécution
    assert_eq!(
        call(&app, "POST", "/api/pipelines/desk-var/runs", Some("tok-bob"), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (s, run, _) = call(
        &app,
        "POST",
        "/api/pipelines/desk-var/runs?wait=true",
        Some("tok-alice"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{run}");
    assert_eq!(run["id"], 1);
    assert_eq!(
        call(&app, "POST", "/api/pipelines/inconnu/runs", Some("tok-alice"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    // Rejeu
    let (s, v, _) = call(
        &app,
        "POST",
        "/api/pipelines/desk-var/runs/1/verify",
        Some("tok-bob"),
        None,
    )
    .await;
    assert_eq!((s, v["reproducible"].as_bool()), (StatusCode::OK, Some(true)));

    // Décision : réservée au validateur, rejet motivé
    let approve = json!({"decision": "approved", "comment": "backtest conforme"});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/pipelines/desk-var/runs/1/decisions",
            Some("tok-alice"),
            Some(approve.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/pipelines/desk-var/runs/1/decisions",
            Some("tok-bob"),
            Some(json!({"decision": "rejected"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (s, _, body) = call(
        &app,
        "POST",
        "/api/pipelines/desk-var/runs/1/decisions",
        Some("tok-bob"),
        Some(approve.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    let (_, runs, _) = call(&app, "GET", "/api/pipelines/desk-var/runs", Some("tok-carol"), None).await;
    assert_eq!(
        (runs[0]["triggered_by"].as_str(), runs[0]["decision"].as_str()),
        (Some("alice"), Some("approved"))
    );

    // Rapport HTML avec la décision
    let (s, _, html) = call(
        &app,
        "GET",
        "/api/pipelines/desk-var/runs/1/report",
        Some("tok-carol"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(html.contains("Décisions de validation") && html.contains("backtest conforme"));

    // Altération des données : rejeu en écart, décision refusée
    let csv = ws.join("desk/returns.csv");
    let original = std::fs::read_to_string(&csv).unwrap();
    let tampered: Vec<String> = original
        .lines()
        .map(|l| {
            if l.starts_with("300,") {
                "300,-0.3".to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    std::fs::write(&csv, tampered.join("\n")).unwrap();
    let (_, v, _) = call(
        &app,
        "POST",
        "/api/pipelines/desk-var/runs/1/verify",
        Some("tok-bob"),
        None,
    )
    .await;
    assert_eq!(v["reproducible"], false, "{v}");
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/pipelines/desk-var/runs/1/decisions",
            Some("tok-bob"),
            Some(approve)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );

    // Journal d'audit : réservé à l'auditeur, trace les refus
    assert_eq!(
        call(&app, "GET", "/api/audit", Some("tok-alice"), None).await.0,
        StatusCode::FORBIDDEN
    );
    let (s, log, _) = call(&app, "GET", "/api/audit", Some("tok-carol"), None).await;
    assert_eq!(s, StatusCode::OK);
    let outcomes: Vec<String> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            format!(
                "{}:{}:{}",
                e["user"].as_str().unwrap(),
                e["action"].as_str().unwrap(),
                e["outcome"].as_str().unwrap()
            )
        })
        .collect();
    for expected in [
        "bob:run.create:denied",
        "alice:run.create:ok",
        "bob:run.decide:approved",
        "bob:run.verify:mismatch",
        "alice:audit.read:denied",
    ] {
        assert!(
            outcomes.iter().any(|o| o == expected),
            "{expected} absent de {outcomes:?}"
        );
    }
    assert!(outcomes.iter().any(|o| o.starts_with("bob:run.decide:refused")));

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn runs_are_queued_and_tracked() {
    let ws = workspace("jobs");
    let app = app(&ws);
    let (s, job, _) = call(&app, "POST", "/api/pipelines/desk-var/runs", Some("tok-alice"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{job}");
    let href = job["href"].as_str().unwrap().to_string();
    let mut last = Value::Null;
    for _ in 0..200 {
        last = call(&app, "GET", &href, Some("tok-carol"), None).await.1;
        if last["status"] == "succeeded" || last["status"] == "failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(
        (last["status"].as_str(), last["run_id"].as_i64()),
        (Some("succeeded"), Some(1)),
        "{last}"
    );
    assert_eq!(
        call(&app, "GET", "/api/jobs/999", Some("tok-carol"), None).await.0,
        StatusCode::NOT_FOUND
    );

    // Échec d'exécution : le travail passe en `failed` avec le message d'erreur.
    std::fs::write(ws.join("desk/returns.csv"), "day,ret\n0,abc\n").unwrap();
    let (s, _, body) = call(
        &app,
        "POST",
        "/api/pipelines/desk-var/runs?wait=true",
        Some("tok-alice"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.contains("non numérique"));
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn rate_limit_per_user() {
    let ws = workspace("rate");
    let app = app_with(&ws, 3);
    for _ in 0..3 {
        assert_eq!(
            call(&app, "GET", "/api/me", Some("tok-bob"), None).await.0,
            StatusCode::OK
        );
    }
    assert_eq!(
        call(&app, "GET", "/api/me", Some("tok-bob"), None).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    // La limite est propre à chaque utilisateur.
    assert_eq!(
        call(&app, "GET", "/api/me", Some("tok-carol"), None).await.0,
        StatusCode::OK
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn rejects_bad_config() {
    let ws = std::env::temp_dir();
    assert!(ServerConfig::parse(
        "workspace = \".\"\n[[user]]\nname = \"a\"\nrole = \"admin\"\ntoken_sha256 = \"court\"\n",
        &ws
    )
    .is_err());
    assert!(ServerConfig::parse(
        "workspace = \".\"\n[[user]]\nname = \"a\"\nrole = \"root\"\ntoken_sha256 = \"\"\n",
        &ws
    )
    .is_err());
}
