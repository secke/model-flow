use mf_runtime::{report, spec, store::Store};
use std::{env, path::Path, process::ExitCode};

const USAGE: &str = "usage:
  mf run    <pipeline.toml>              exécute et enregistre l'exécution
  mf verify <pipeline.toml> [--run ID]   rejoue et compare (défaut : dernière exécution)
  mf runs   <pipeline.toml>              liste les exécutions
  mf show   <pipeline.toml> [--run ID]   affiche le manifeste JSON
  mf report <pipeline.toml> [--run ID] [-o rapport.html] [--no-verify]
                                         rapport de validation HTML

Historique : SQLite dans <dossier du pipeline>/.modelflow, ou PostgreSQL si MODELFLOW_DATABASE_URL est défini.";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn short(v: &serde_json::Value) -> String {
    if let Some(st) = v.get("status").and_then(|s| s.as_str()) {
        return format!("[{}]", st.to_uppercase());
    }
    let s = v.to_string();
    if s.len() > 140 {
        format!("{}… ({} octets)", &s[..140], s.len())
    } else {
        s
    }
}

fn run_id(args: &[String]) -> Result<Option<i64>, String> {
    match args.iter().position(|a| a == "--run") {
        Some(i) => args
            .get(i + 1)
            .and_then(|v| v.parse().ok())
            .map(Some)
            .ok_or_else(|| "--run attend un entier".into()),
        None => Ok(None),
    }
}

/// Historique : PostgreSQL si `MODELFLOW_DATABASE_URL` est défini, sinon SQLite à côté du pipeline.
fn open_store(base: &Path) -> Result<Store, String> {
    match std::env::var("MODELFLOW_DATABASE_URL") {
        Ok(url) if !url.is_empty() => Store::postgres(&mf_runtime::pg::connect_store(&url, 2)?),
        _ => Store::open(&base.join(".modelflow/runs.db")),
    }
}

fn real_main(args: &[String]) -> Result<bool, String> {
    let (cmd, file) = match (args.get(1), args.get(2)) {
        (Some(c), Some(f)) => (c.as_str(), Path::new(f)),
        _ => return Err(USAGE.into()),
    };
    let (sp, base) = spec::load(file)?;
    let store = open_store(&base)?;
    match cmd {
        "run" => {
            let m = spec::build(&sp, &base)?.execute().map_err(|e| e.to_string())?;
            for s in &m.steps {
                println!("{:<20} v{:<8} {}", s.name, s.version, short(&s.output));
            }
            let user = std::env::var("USER").unwrap_or_else(|_| "inconnu".into());
            let id = store.save(&sp.name, &m, &format!("cli:{user}"))?;
            println!("\nexécution #{id}  run_hash={}", m.run_hash);
            Ok(true)
        }
        "verify" => {
            let (id, reference) = store.get(&sp.name, run_id(args)?)?;
            let mm = spec::build(&sp, &base)?.verify(&reference).map_err(|e| e.to_string())?;
            if mm.is_empty() {
                println!(
                    "OK : exécution #{id} rejouée à l'identique ({} étapes)",
                    reference.steps.len()
                );
                Ok(true)
            } else {
                for x in &mm {
                    println!("ÉCART [{}] {}", x.step, x.reason);
                }
                Ok(false)
            }
        }
        "runs" => {
            for r in store.list(&sp.name)? {
                println!(
                    "#{:<4} {}  {:<16} {}",
                    r.id,
                    report::utc(r.created_at),
                    r.triggered_by,
                    &r.run_hash[..16]
                );
            }
            Ok(true)
        }
        "show" => {
            let (_, m) = store.get(&sp.name, run_id(args)?)?;
            println!("{}", serde_json::to_string_pretty(&m).unwrap());
            Ok(true)
        }
        "report" => {
            let (id, m) = store.get(&sp.name, run_id(args)?)?;
            let mismatches = if args.iter().any(|a| a == "--no-verify") {
                None
            } else {
                Some(spec::build(&sp, &base)?.verify(&m).map_err(|e| e.to_string())?)
            };
            let meta = report::ReportMeta {
                pipeline: &sp.name,
                run_id: id,
                created_at: store.info(&sp.name, id)?.map(|r| r.created_at).unwrap_or_default(),
                generated_at: now(),
                replay: mismatches.as_deref(),
                decisions: &[],
            };
            let out = flag_value(args, "-o")
                .map(Into::into)
                .unwrap_or_else(|| base.join(format!(".modelflow/report-{id}.html")));
            std::fs::write(&out, report::render_html(&meta, &m)).map_err(|e| e.to_string())?;
            println!("rapport écrit : {}", out.display());
            Ok(true)
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match real_main(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
