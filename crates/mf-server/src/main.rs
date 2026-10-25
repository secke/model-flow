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
            if cfg.users.is_empty() {
                fail("aucun utilisateur configuré");
            }
            let state = AppState::new(&cfg).unwrap_or_else(|e| fail(&e));
            let listener = tokio::net::TcpListener::bind(&cfg.bind)
                .await
                .unwrap_or_else(|e| fail(&e.to_string()));
            eprintln!(
                "ModelFlow API sur http://{} (espace : {})",
                cfg.bind,
                cfg.workspace.display()
            );
            axum::serve(listener, router(state))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
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
