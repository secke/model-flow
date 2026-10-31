use mf_runtime::{dated, diff, report, spec, store::Store};
use std::{env, path::Path, process::ExitCode};

const USAGE: &str = "usage:
  mf run    <pipeline.toml> [-o run.json] run and record the execution (and write its manifest)
            [--as-of DATE|D1,D2|all|new] [--jobs N]
                                         dated pipeline: one run per as-of date, in parallel
  mf verify <pipeline.toml> [--run ID | --as-of DATE]
                                         replay and compare (default: latest run)
  mf runs   <pipeline.toml>              list runs
  mf show   <pipeline.toml> [--run ID | --as-of DATE]
                                         print the JSON manifest
  mf report <pipeline.toml> [--run ID | --as-of DATE] [-o report.html] [--no-verify]
                                         HTML validation report
  mf series <pipeline.toml> [PATH...] [--format text|csv|md|json] [-o file]
                                         values at PATH (wildcards * ?) for every as-of date;
                                         without PATH, the status of every check
  mf diff   <left> [<right>] [--tolerances file.toml] [--format text|md|json]
            [--limit N] [-o file]       compare two runs; exit code 1 beyond tolerance

A run for `mf diff` is `pipeline.toml` (latest run), `pipeline.toml#ID`, `pipeline.toml#previous`,
`pipeline.toml#fresh` (executed now, not recorded) or a manifest file `run.json` (`mf show`).
With a single pipeline, `mf diff pipeline.toml` compares its previous and latest runs.
A dated pipeline has `${as_of}` in its data paths; `pipeline.toml@2024-01-31` designates the
runs of that date (`@2024-01-31#fresh`, `@2024-01-31#previous`...). `--as-of all` runs every date
whose data exists, `--as-of new` only those without a recorded run.

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

/// Run selected by `--run ID` or `--as-of DATE` (latest run of that date), else the latest run.
fn selected(store: &Store, pipeline: &str, args: &[String]) -> Result<(i64, mf_core::Manifest), String> {
    match flag_value(args, "--as-of") {
        Some(d) => store.get_as_of(pipeline, d),
        None => store.get(pipeline, run_id(args)?),
    }
}

fn run_dated(sp: &spec::PipelineSpec, base: &Path, store: &Store, args: &[String], who: &str) -> Result<bool, String> {
    let sel = flag_value(args, "--as-of").ok_or_else(|| {
        format!(
            "pipeline {} is dated (`{}` in its paths): give --as-of DATE, D1,D2, all or new",
            sp.name,
            spec::AS_OF
        )
    })?;
    let dates = dated::select_dates(sp, base, store, sel)?;
    if dates.is_empty() {
        println!("no date to run");
        return Ok(true);
    }
    let jobs = match flag_value(args, "--jobs") {
        Some(n) => n.parse().map_err(|_| "--jobs expects an integer".to_string())?,
        None => 0,
    };
    let started = std::time::Instant::now();
    let results = dated::run_dates(sp, base, &dates, jobs);
    let mut ok = true;
    for (d, r) in &results {
        match r {
            Ok(m) => {
                let id = store.save(&sp.name, m, who)?;
                let status = mf_runtime::store::overall_status(m).unwrap_or_else(|| "-".into());
                println!("{d:<12} run #{id:<5} {status:<5} run_hash={}", &m.run_hash[..16]);
            }
            Err(e) => {
                ok = false;
                println!("{d:<12} ERROR {}", e.lines().next().unwrap_or(""));
            }
        }
    }
    let failed = results.iter().filter(|(_, r)| r.is_err()).count();
    println!(
        "\n{} date{} run in {:.1} s{}",
        results.len() - failed,
        if results.len() - failed == 1 { "" } else { "s" },
        started.elapsed().as_secs_f64(),
        if failed > 0 {
            format!(", {failed} failed")
        } else {
            String::new()
        }
    );
    Ok(ok)
}

fn series_main(args: &[String]) -> Result<bool, String> {
    const VALUED: [&str; 2] = ["--format", "-o"];
    let file = args.get(2).ok_or(USAGE)?;
    let (sp, base) = spec::load(Path::new(file))?;
    let store = Store::for_pipeline(&base)?;
    let mut paths = vec![];
    let mut i = 3;
    while i < args.len() {
        if VALUED.contains(&args[i].as_str()) {
            i += 2;
            continue;
        }
        if args[i].starts_with('-') {
            return Err(format!("unknown option {}\n{USAGE}", args[i]));
        }
        paths.push(args[i].clone());
        i += 1;
    }
    let s = dated::series(&store, &sp.name, &paths)?;
    if s.rows.is_empty() {
        return Err(format!(
            "no dated run recorded for {} (mf run {file} --as-of all)",
            sp.name
        ));
    }
    let out = match flag_value(args, "--format").unwrap_or("text") {
        "text" => dated::render_text(&s),
        "csv" => dated::render_csv(&s),
        "md" | "markdown" => dated::render_markdown(&s),
        "json" => serde_json::to_string_pretty(&s).unwrap(),
        f => return Err(format!("unknown format `{f}` (text, csv, md, json)")),
    };
    match flag_value(args, "-o") {
        Some(f) => {
            std::fs::write(f, &out).map_err(|e| format!("{f}: {e}"))?;
            println!(
                "series written: {f} ({} dates, {} columns)",
                s.rows.len(),
                s.columns.len()
            );
        }
        None => print!("{out}"),
    }
    Ok(true)
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
    match args.get(1).map(String::as_str) {
        Some("diff") => return diff_main(args),
        Some("series") => return series_main(args),
        _ => {}
    }
    let (cmd, file) = match (args.get(1), args.get(2)) {
        (Some(c), Some(f)) => (c.as_str(), Path::new(f)),
        _ => return Err(USAGE.into()),
    };
    let (sp, base) = spec::load(file)?;
    let store = Store::for_pipeline(&base)?;
    match cmd {
        "run" => {
            let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
            let who = format!("cli:{user}");
            let single = flag_value(args, "--as-of").filter(|d| !d.contains(',') && *d != "all" && *d != "new");
            if sp.dated() && single.is_none() {
                return run_dated(&sp, &base, &store, args, &who);
            }
            let m = spec::run(&sp, &base, single)?;
            for s in &m.steps {
                println!("{:<20} v{:<8} {}", s.name, s.version, short(&s.output));
            }
            let id = store.save(&sp.name, &m, &who)?;
            let at = m.as_of.as_deref().map(|d| format!("  as_of={d}")).unwrap_or_default();
            println!("\nrun #{id}{at}  run_hash={}", m.run_hash);
            if let Some(f) = flag_value(args, "-o") {
                std::fs::write(f, serde_json::to_string_pretty(&m).unwrap()).map_err(|e| format!("{f}: {e}"))?;
                println!("manifest written: {f}");
            }
            Ok(true)
        }
        "verify" => {
            let (id, reference) = selected(&store, &sp.name, args)?;
            let mm = spec::verify(&sp, &base, &reference)?;
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
                    "#{:<4} {}  {:<16} {}  {:<5} {}",
                    r.id,
                    report::utc(r.created_at),
                    r.triggered_by,
                    &r.run_hash[..16],
                    r.status.as_deref().unwrap_or("-"),
                    r.as_of.as_deref().unwrap_or("")
                );
            }
            Ok(true)
        }
        "show" => {
            let (_, m) = selected(&store, &sp.name, args)?;
            println!("{}", serde_json::to_string_pretty(&m).unwrap());
            Ok(true)
        }
        "report" => {
            let (id, m) = selected(&store, &sp.name, args)?;
            let mismatches = if args.iter().any(|a| a == "--no-verify") {
                None
            } else {
                Some(spec::verify(&sp, &base, &m)?)
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
