use mf_runtime::{diff, report, spec, store::Store};
use std::{env, path::Path, process::ExitCode};

const USAGE: &str = "usage:
  mf run    <pipeline.toml> [-o run.json] run and record the execution (and write its manifest)
  mf verify <pipeline.toml> [--run ID]   replay and compare (default: latest run)
  mf runs   <pipeline.toml>              list runs
  mf show   <pipeline.toml> [--run ID]   print the JSON manifest
  mf report <pipeline.toml> [--run ID] [-o report.html] [--no-verify]
                                         HTML validation report
  mf diff   <left> [<right>] [--tolerances file.toml] [--format text|md|json]
            [--limit N] [-o file]       compare two runs; exit code 1 beyond tolerance

A run for `mf diff` is `pipeline.toml` (latest run), `pipeline.toml#ID`, `pipeline.toml#previous`,
`pipeline.toml#fresh` (executed now, not recorded) or a manifest file `run.json` (`mf show`).
With a single pipeline, `mf diff pipeline.toml` compares its previous and latest runs.

History: SQLite in <pipeline directory>/.modelflow, or PostgreSQL when MODELFLOW_DATABASE_URL is set.";

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
        format!("{}… ({} bytes)", &s[..140], s.len())
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
            .ok_or_else(|| "--run expects an integer".into()),
        None => Ok(None),
    }
}

fn diff_main(args: &[String]) -> Result<bool, String> {
    const VALUED: [&str; 4] = ["--tolerances", "--format", "--limit", "-o"];
    let mut refs = vec![];
    let mut i = 2;
    while i < args.len() {
        if VALUED.contains(&args[i].as_str()) {
            i += 2;
            continue;
        }
        if args[i].starts_with('-') {
            return Err(format!("unknown option {}\n{USAGE}", args[i]));
        }
        refs.push(args[i].as_str());
        i += 1;
    }
    let (left, right) = match refs[..] {
        [one] if !one.contains('#') && !one.ends_with(".json") => (format!("{one}#previous"), format!("{one}#latest")),
        [l, r] => (l.to_string(), r.to_string()),
        _ => return Err(USAGE.into()),
    };
    let (l, r) = (diff::resolve(&left)?, diff::resolve(&right)?);
    let (llabel, lm, rlabel, rm) = (l.label, l.manifest, r.label, r.manifest);
    let mut rules = match flag_value(args, "--tolerances") {
        Some(f) => diff::load_tolerances(Path::new(f))?,
        None => vec![],
    };
    // The candidate's rules first, then the reference's.
    rules.extend(r.spec.into_iter().chain(l.spec).flat_map(|s| s.tolerances));
    let limit = match flag_value(args, "--limit") {
        Some(n) => n.parse().map_err(|_| "--limit expects an integer".to_string())?,
        None => 10,
    };
    let d = mf_core::diff::diff(&lm, &rm, &rules);
    let out = match flag_value(args, "--format").unwrap_or("text") {
        "text" => diff::render_text(&d, &llabel, &rlabel, limit),
        "md" | "markdown" => diff::render_markdown(&d, &llabel, &rlabel, limit),
        "json" => serde_json::to_string_pretty(&serde_json::json!({
            "left": llabel, "right": rlabel, "verdict": diff::verdict(&d), "diff": d,
        }))
        .unwrap(),
        f => return Err(format!("unknown format `{f}` (text, md, json)")),
    };
    match flag_value(args, "-o") {
        Some(f) => {
            std::fs::write(f, &out).map_err(|e| format!("{f}: {e}"))?;
            println!("{}", diff::verdict(&d));
        }
        None => print!("{out}"),
    }
    Ok(d.ok)
}

fn real_main(args: &[String]) -> Result<bool, String> {
    if args.get(1).map(String::as_str) == Some("diff") {
        return diff_main(args);
    }
    let (cmd, file) = match (args.get(1), args.get(2)) {
        (Some(c), Some(f)) => (c.as_str(), Path::new(f)),
        _ => return Err(USAGE.into()),
    };
    let (sp, base) = spec::load(file)?;
    let store = Store::for_pipeline(&base)?;
    match cmd {
        "run" => {
            let m = spec::build(&sp, &base)?.execute().map_err(|e| e.to_string())?;
            for s in &m.steps {
                println!("{:<20} v{:<8} {}", s.name, s.version, short(&s.output));
            }
            let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
            let id = store.save(&sp.name, &m, &format!("cli:{user}"))?;
            println!("\nrun #{id}  run_hash={}", m.run_hash);
            if let Some(f) = flag_value(args, "-o") {
                std::fs::write(f, serde_json::to_string_pretty(&m).unwrap()).map_err(|e| format!("{f}: {e}"))?;
                println!("manifest written: {f}");
            }
            Ok(true)
        }
        "verify" => {
            let (id, reference) = store.get(&sp.name, run_id(args)?)?;
            let mm = spec::build(&sp, &base)?.verify(&reference).map_err(|e| e.to_string())?;
            if mm.is_empty() {
                println!("OK: run #{id} replayed identically ({} steps)", reference.steps.len());
                Ok(true)
            } else {
                for x in &mm {
                    println!("MISMATCH [{}] {}", x.step, x.reason);
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
            println!("report written: {}", out.display());
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
