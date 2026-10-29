//! OIDC SSO: tokens signed by a simulated identity provider. The test RSA keys are generated
//! at runtime (openssl): no private key is committed.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use http_body_util::BodyExt;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use mf_server::{hash_token, router, AppState, ServerConfig};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const ISS: &str = "https://idp.bank.example/realms/risk";
const AUD: &str = "modelflow-api";

/// Directory containing `idp-key.pem`, `other-key.pem`, `jwks.json` (kid test-key-1) and
/// `jwks-rotated.json` (test-key-1 and test-key-2), created once per test run.
fn fixtures() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        use base64::Engine;
        let dir = std::env::temp_dir().join(format!("mf-oidc-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jwk = |file: &str, kid: &str| {
            let p = dir.join(file);
            let ok = std::process::Command::new("openssl")
                .args([
                    "genpkey",
                    "-algorithm",
                    "RSA",
                    "-pkeyopt",
                    "rsa_keygen_bits:2048",
                    "-out",
                ])
                .arg(&p)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("openssl is required for the OIDC tests")
                .success();
            assert!(ok);
            let out = std::process::Command::new("openssl")
                .args(["rsa", "-noout", "-modulus", "-in"])
                .arg(&p)
                .output()
                .unwrap();
            let hex = String::from_utf8(out.stdout)
                .unwrap()
                .trim()
                .trim_start_matches("Modulus=")
                .to_string();
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
            json!({"kty": "RSA", "kid": kid, "use": "sig", "alg": "RS256", "n": b64.encode(bytes), "e": "AQAB"})
        };
        let (k1, k2) = (jwk("idp-key.pem", "test-key-1"), jwk("other-key.pem", "test-key-2"));
        std::fs::write(dir.join("jwks.json"), json!({"keys": [k1.clone()]}).to_string()).unwrap();
        std::fs::write(dir.join("jwks-rotated.json"), json!({"keys": [k1, k2]}).to_string()).unwrap();
        dir
    })
    .clone()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn sign(key: &str, kid: &str, claims: Value) -> String {
    let pem = std::fs::read(fixtures().join(key)).unwrap();
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some(kid.into());
    encode(&h, &claims, &EncodingKey::from_rsa_pem(&pem).unwrap()).unwrap()
}

fn claims(user: &str, groups: &[&str]) -> Value {
    json!({"iss": ISS, "aud": AUD, "sub": format!("id-{user}"), "preferred_username": user,
           "groups": groups, "iat": now(), "exp": now() + 600})
}

fn app(oidc: &str) -> axum::Router {
    let ws = std::env::temp_dir().join(format!("mf-oidc-{}-{}", std::process::id(), now()));
    std::fs::create_dir_all(&ws).unwrap();
    let cfg = format!(
        r#"workspace = "{}"
[[user]]
name = "ci-service"
role = "admin"
token_sha256 = "{}"
[oidc]
audience = "{AUD}"
{oidc}
roles_claim = "groups"
[oidc.role_mapping]
"MF-Owners" = "model_owner"
"MF-Validators" = "validator"
"MF-Auditors" = "auditor"
"#,
        ws.display(),
        hash_token("tok-service")
    );
    let cfg = ServerConfig::parse(&cfg, &fixtures()).unwrap();
    router(AppState::new(&cfg).unwrap())
}

async fn me(app: &axum::Router, token: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .uri("/api/me")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn static_jwks() -> String {
    format!("issuer = \"{ISS}\"\njwks_file = \"jwks.json\"")
}

#[tokio::test]
async fn valid_tokens_map_to_roles() {
    let app = app(&static_jwks());
    let (s, b) = me(
        &app,
        &sign(
            "idp-key.pem",
            "test-key-1",
            claims("dana", &["MF-Owners", "Other-Group"]),
        ),
    )
    .await;
    assert_eq!(
        (s, b["name"].as_str(), b["role"].as_str()),
        (StatusCode::OK, Some("dana"), Some("model_owner"))
    );
    let (_, b) = me(
        &app,
        &sign(
            "idp-key.pem",
            "test-key-1",
            claims("eli", &["MF-Validators", "MF-Auditors"]),
        ),
    )
    .await;
    assert_eq!(b["role"], "validator");
    // Service tokens are still accepted.
    let (s, b) = me(&app, "tok-service").await;
    assert_eq!((s, b["role"].as_str()), (StatusCode::OK, Some("admin")));
}

#[tokio::test]
async fn segregation_of_duties_and_unmapped_groups() {
    let app = app(&static_jwks());
    let (s, b) = me(
        &app,
        &sign(
            "idp-key.pem",
            "test-key-1",
            claims("fred", &["MF-Owners", "MF-Validators"]),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(b["error"].as_str().unwrap().contains("segregation of duties"));
    let (s, _) = me(&app, &sign("idp-key.pem", "test-key-1", claims("gina", &["Finance"]))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invalid_tokens_are_rejected() {
    let app = app(&static_jwks());
    let mut c = claims("hugo", &["MF-Owners"]);
    let cases = {
        let mut v = vec![];
        let mut x = c.clone();
        x["aud"] = json!("other-api");
        v.push(("audience", sign("idp-key.pem", "test-key-1", x)));
        let mut x = c.clone();
        x["iss"] = json!("https://pirate.example");
        v.push(("issuer", sign("idp-key.pem", "test-key-1", x)));
        let mut x = c.clone();
        x["exp"] = json!(now() - 3600);
        v.push(("expired", sign("idp-key.pem", "test-key-1", x)));
        let mut x = c.clone();
        x["nbf"] = json!(now() + 3600);
        v.push(("not yet valid", sign("idp-key.pem", "test-key-1", x)));
        // Unpublished key presented under the published key's kid.
        v.push(("bad signature", sign("other-key.pem", "test-key-1", c.clone())));
        v.push(("unknown kid", sign("other-key.pem", "unknown", c.clone())));
        // Algorithm confusion attack: HMAC signed with the public key.
        let jwks = std::fs::read_to_string(fixtures().join("jwks.json")).unwrap();
        let mut h = Header::new(Algorithm::HS256);
        h.kid = Some("test-key-1".into());
        v.push((
            "HS256",
            encode(&h, &c, &EncodingKey::from_secret(jwks.as_bytes())).unwrap(),
        ));
        // alg = none
        use base64::Engine;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let none = format!(
            "{}.{}.",
            b64.encode(br#"{"alg":"none","typ":"JWT"}"#),
            b64.encode(c.to_string())
        );
        v.push(("alg none", none));
        v
    };
    for (what, token) in cases {
        assert_eq!(me(&app, &token).await.0, StatusCode::UNAUTHORIZED, "{what}");
    }
    c.as_object_mut().unwrap().remove("preferred_username");
    assert_eq!(
        me(&app, &sign("idp-key.pem", "test-key-1", c)).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn discovery_and_key_rotation() {
    // Simulated identity provider: discovery + mutable JWKS.
    let jwks = Arc::new(Mutex::new(
        std::fs::read_to_string(fixtures().join("jwks.json")).unwrap(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (b2, j2) = (base.clone(), jwks.clone());
    let idp = axum::Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let b = b2.clone();
                async move { axum::Json(json!({"issuer": b, "jwks_uri": format!("{b}/jwks")})) }
            }),
        )
        .route(
            "/jwks",
            get(move || {
                let j = j2.clone();
                async move { j.lock().unwrap().clone() }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, idp).await.unwrap() });

    let app = app(&format!("issuer = \"{base}\"\njwks_refresh_secs = 0"));
    let mk = |key: &str, kid: &str| {
        let mut c = claims("ines", &["MF-Auditors"]);
        c["iss"] = json!(base);
        sign(key, kid, c)
    };
    assert_eq!(me(&app, &mk("idp-key.pem", "test-key-1")).await.0, StatusCode::OK);
    // New key not yet published: rejected.
    assert_eq!(
        me(&app, &mk("other-key.pem", "test-key-2")).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Provider-side rotation: the key is reloaded the first time its kid is used.
    *jwks.lock().unwrap() = std::fs::read_to_string(fixtures().join("jwks-rotated.json")).unwrap();
    assert_eq!(me(&app, &mk("other-key.pem", "test-key-2")).await.0, StatusCode::OK);
}

#[test]
fn rejects_unsafe_oidc_config() {
    let base =
        format!("workspace = \".\"\n[oidc]\nissuer = \"{ISS}\"\naudience = \"{AUD}\"\njwks_file = \"jwks.json\"\n");
    let hs = format!("{base}algorithms = [\"HS256\"]\n[oidc.role_mapping]\n\"G\" = \"auditor\"\n");
    let cfg = ServerConfig::parse(&hs, &fixtures()).unwrap();
    assert!(AppState::new(&cfg).is_err());
    let empty = format!("{base}[oidc.role_mapping]\n");
    let cfg = ServerConfig::parse(&empty, &fixtures()).unwrap();
    assert!(AppState::new(&cfg).is_err());
}
