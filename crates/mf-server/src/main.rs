use mf_server::{hash_token, new_token, router, AppState, ServerConfig};
use std::path::Path;

const USAGE: &str = "usage:
  mf-server serve --config server.toml
  mf-server new-token            génère un jeton et son empreinte à placer dans la configuration
  mf-server hash-token <jeton>   affiche l'empreinte SHA-256 d'un jeton";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("serve") => {
            let cfg_path = args
                .iter()
                .position(|a| a == "--config")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
                .unwrap_or("server.toml");
            let cfg = ServerConfig::load(Path::new(cfg_path)).unwrap_or_else(|e| fail(&e));
            if cfg.users.is_empty() && cfg.oidc.is_none() {
                fail("aucun utilisateur configuré ([[user]] ou [oidc])");
            }
            let state = AppState::new(&cfg).unwrap_or_else(|e| fail(&e));
            mf_server::start_scheduler(state.clone());
            let addr: std::net::SocketAddr = cfg.bind.parse().unwrap_or_else(|e| fail(&format!("bind: {e}")));
            let app = router(state).into_make_service();
            let handle = axum_server::Handle::new();
            let h = handle.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                h.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
            });
            match &cfg.tls {
                Some(tls) => {
                    let _ = rustls::crypto::ring::default_provider().install_default();
                    let rc = axum_server::tls_rustls::RustlsConfig::from_pem_file(&tls.cert, &tls.key)
                        .await
                        .unwrap_or_else(|e| fail(&format!("TLS: {e}")));
                    eprintln!("ModelFlow API sur https://{addr} (espace : {})", cfg.workspace.display());
                    axum_server::bind_rustls(addr, rc).handle(handle).serve(app).await
                }
                None => {
                    eprintln!(
                        "ModelFlow API sur http://{addr} (espace : {}) — TLS désactivé : placer derrière un proxy HTTPS",
                        cfg.workspace.display()
                    );
                    axum_server::bind(addr).handle(handle).serve(app).await
                }
            }
            .unwrap_or_else(|e| fail(&e.to_string()));
        }
        Some("new-token") => {
            let t = new_token();
            println!(
                "jeton (à transmettre à l'utilisateur) : {t}\ntoken_sha256 (configuration)         : {}",
                hash_token(&t)
            );
        }
        Some("hash-token") => match args.get(2) {
            Some(t) => println!("{}", hash_token(t)),
            None => fail(USAGE),
        },
        _ => fail(USAGE),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(2)
}
