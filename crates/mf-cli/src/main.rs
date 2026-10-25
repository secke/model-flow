use mf_core::{FnStep, Manifest, Pipeline};
use mf_validate::{historical_var, rolling_backtest};
use serde_json::json;
use std::{env, fs, process::ExitCode};

/// Rendements synthétiques déterministes (somme de 12 uniformes ≈ normale).
fn synthetic_returns(n: usize, seed: u64, vol: f64) -> Vec<f64> {
    let mut s = seed;
    let mut u = move || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..n).map(|_| ((0..12).map(|_| u()).sum::<f64>() - 6.0) * vol).collect()
}

fn demo_pipeline() -> Pipeline {
    Pipeline::new()
        .add(FnStep::new("returns", "1.0.0", &[], |_| {
            Ok(json!(synthetic_returns(1500, 42, 0.01)))
        }))
        .add(FnStep::new("var_99", "1.0.0", &["returns"], |i| {
            let r: Vec<f64> = serde_json::from_value(i["returns"].clone()).map_err(|e| e.to_string())?;
            let v = historical_var(&r, 0.99).ok_or("série vide")?;
            Ok(json!({ "alpha": 0.99, "var": v }))
        }))
        .add(FnStep::new("backtest_kupiec", "1.0.0", &["returns"], |i| {
            let r: Vec<f64> = serde_json::from_value(i["returns"].clone()).map_err(|e| e.to_string())?;
            let k = rolling_backtest(&r, 250, 0.99).ok_or("série trop courte")?;
            serde_json::to_value(k).map_err(|e| e.to_string())
        }))
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("demo") => {
            let out = args.get(2).map(String::as_str).unwrap_or("manifest.json");
            let m = demo_pipeline().execute().expect("exécution");
            for s in m.steps.iter().filter(|s| s.name != "returns") {
                println!("{} v{} -> {}", s.name, s.version, s.output);
            }
            println!("run_hash = {}", m.run_hash);
            fs::write(out, serde_json::to_string_pretty(&m).unwrap()).expect("écriture");
            println!("manifeste écrit: {out}");
            ExitCode::SUCCESS
        }
        Some("verify") => {
            let Some(path) = args.get(2) else { eprintln!("usage: mf verify <manifest.json>"); return ExitCode::from(2) };
            let m: Manifest = serde_json::from_str(&fs::read_to_string(path).expect("lecture")).expect("manifeste invalide");
            let mm = demo_pipeline().verify(&m).expect("exécution");
            if mm.is_empty() {
                println!("OK : rejeu identique ({} étapes)", m.steps.len());
                ExitCode::SUCCESS
            } else {
                for x in &mm { println!("ÉCART [{}] {}", x.step, x.reason); }
                ExitCode::FAILURE
            }
        }
        _ => { eprintln!("usage: mf demo [out.json] | mf verify <manifest.json>"); ExitCode::from(2) }
    }
}
