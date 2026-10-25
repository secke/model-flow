use mf_core::{hash_bytes, Error, Inputs, Step};
use mf_validate::{calibration, discrimination, historical_var, psi, rolling_backtest, var_backtest};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn err(step: &str, msg: impl Into<String>) -> Error {
    Error::Step {
        step: step.into(),
        msg: msg.into(),
    }
}

fn to_floats(arr: &[Value], step: &str) -> Result<Vec<f64>, Error> {
    arr.iter()
        .map(|x| x.as_f64().ok_or_else(|| err(step, "valeur non numérique")))
        .collect()
}

/// Série numérique : tableau, objet `{"values": [...]}`, ou colonne `col` d'une table `{col: [...]}`.
fn series(v: &Value, col: Option<&str>, step: &str) -> Result<Vec<f64>, Error> {
    let arr = match col {
        Some(c) => v
            .get(c)
            .and_then(Value::as_array)
            .ok_or_else(|| err(step, format!("colonne `{c}` absente")))?,
        None => v
            .as_array()
            .or_else(|| v.get("values").and_then(Value::as_array))
            .ok_or_else(|| err(step, "série attendue (tableau de nombres)"))?,
    };
    to_floats(arr, step)
}

fn flags(v: &[f64]) -> Vec<bool> {
    v.iter().map(|x| *x != 0.0).collect()
}

fn param_f64(p: &Value, k: &str, default: f64) -> f64 {
    p.get(k).and_then(Value::as_f64).unwrap_or(default)
}

fn param_str<'a>(p: &'a Value, k: &str, default: &'a str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or(default)
}

// ---------- CSV ----------
/// Lit une colonne (série) ou plusieurs colonnes (table `{col: [...]}`) numériques.
pub struct CsvStep {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub columns: Vec<String>,
    pub table: bool,
}

impl Step for CsvStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn deps(&self) -> &[String] {
        &[]
    }
    fn run(&self, _: &Inputs) -> Result<Value, Error> {
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| err(&self.name, format!("{}: {e}", self.path.display())))?;
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let header: Vec<&str> = lines
            .next()
            .ok_or_else(|| err(&self.name, "fichier vide"))?
            .split(',')
            .map(str::trim)
            .collect();
        let idx: Vec<usize> = self
            .columns
            .iter()
            .map(|c| {
                header
                    .iter()
                    .position(|h| h == c)
                    .ok_or_else(|| err(&self.name, format!("colonne `{c}` absente")))
            })
            .collect::<Result<_, _>>()?;
        let mut cols: Vec<Vec<f64>> = vec![vec![]; idx.len()];
        for (n, l) in lines.enumerate() {
            let cells: Vec<&str> = l.split(',').collect();
            for (k, &i) in idx.iter().enumerate() {
                let cell = cells
                    .get(i)
                    .ok_or_else(|| err(&self.name, format!("ligne {}: colonne manquante", n + 2)))?
                    .trim();
                let v = cell
                    .parse::<f64>()
                    .map_err(|_| err(&self.name, format!("ligne {}: `{cell}` non numérique", n + 2)))?;
                cols[k].push(v);
            }
        }
        if self.table {
            Ok(Value::Object(
                self.columns
                    .iter()
                    .cloned()
                    .zip(cols.into_iter().map(|c| json!(c)))
                    .collect(),
            ))
        } else {
            Ok(json!(cols.pop().unwrap_or_default()))
        }
    }
}

// ---------- Étapes intégrées ----------
pub struct BuiltinStep {
    pub name: String,
    pub version: String,
    pub deps: Vec<String>,
    pub kind: String,
    pub params: Value,
}

impl BuiltinStep {
    pub const KINDS: &'static [&'static str] = &[
        "var_historical",
        "kupiec_backtest",
        "backtest_forecasts",
        "psi",
        "pd_discrimination",
        "pd_calibration",
    ];

    /// Entrée nommée par le paramètre `key`, sinon la dépendance n° `idx`.
    fn dep<'a>(&self, inputs: &'a Inputs, key: &str, idx: usize) -> Result<&'a Value, Error> {
        let name = self
            .params
            .get(key)
            .and_then(Value::as_str)
            .or_else(|| self.deps.get(idx).map(String::as_str))
            .ok_or_else(|| err(&self.name, format!("dépendance `{key}` manquante")))?;
        inputs
            .get(name)
            .ok_or_else(|| err(&self.name, format!("`{name}` n'est pas déclaré dans deps")))
    }

    fn column(&self) -> Option<&str> {
        self.params.get("column").and_then(Value::as_str)
    }

    fn table_col(&self, t: &Value, key: &str, default: &str) -> Result<Vec<f64>, Error> {
        series(t, Some(param_str(&self.params, key, default)), &self.name)
    }

    fn json<T: serde::Serialize>(&self, v: Option<T>, why: &str) -> Result<Value, Error> {
        v.map(|x| serde_json::to_value(x).unwrap())
            .ok_or_else(|| err(&self.name, why))
    }
}

impl Step for BuiltinStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn deps(&self) -> &[String] {
        &self.deps
    }
    fn run(&self, inputs: &Inputs) -> Result<Value, Error> {
        let alpha = param_f64(&self.params, "alpha", 0.99);
        match self.kind.as_str() {
            "var_historical" => {
                let r = series(self.dep(inputs, "series", 0)?, self.column(), &self.name)?;
                let v = historical_var(&r, alpha).ok_or_else(|| err(&self.name, "série vide ou invalide"))?;
                Ok(json!({ "alpha": alpha, "var": v, "observations": r.len() }))
            }
            "kupiec_backtest" => {
                let r = series(self.dep(inputs, "series", 0)?, self.column(), &self.name)?;
                let w = param_f64(&self.params, "window", 250.0) as usize;
                self.json(rolling_backtest(&r, w, alpha), "série trop courte")
            }
            // Prévisions d'un modèle externe : {"start": i, "var": [...]}, var[j] prévoit returns[start + j].
            "backtest_forecasts" => {
                let r = series(self.dep(inputs, "returns", 0)?, self.column(), &self.name)?;
                let f = self.dep(inputs, "forecasts", 1)?;
                let start = f.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
                let var = series(f.get("var").unwrap_or(&Value::Null), None, &self.name)?;
                if var.is_empty() || start + var.len() > r.len() {
                    return Err(err(&self.name, "prévisions vides ou plus longues que la série"));
                }
                self.json(
                    var_backtest(&r[start..start + var.len()], &var, alpha),
                    "backtest impossible",
                )
            }
            "psi" => {
                let col = self.column();
                let base = series(self.dep(inputs, "baseline", 0)?, col, &self.name)?;
                let cur = series(self.dep(inputs, "current", 1)?, col, &self.name)?;
                let bins = param_f64(&self.params, "bins", 10.0) as usize;
                self.json(psi(&base, &cur, bins), "échantillons vides")
            }
            "pd_discrimination" => {
                let t = self.dep(inputs, "table", 0)?;
                let score = self.table_col(t, "score", "pd")?;
                let def = flags(&self.table_col(t, "default", "default")?);
                let (p, w) = (
                    param_f64(&self.params, "min_gini_pass", 0.40),
                    param_f64(&self.params, "min_gini_warn", 0.25),
                );
                self.json(
                    discrimination(&score, &def, p, w),
                    "données invalides (au moins un défaut et un non-défaut requis)",
                )
            }
            "pd_calibration" => {
                let t = self.dep(inputs, "table", 0)?;
                let grade = self.table_col(t, "grade", "grade")?;
                let pd = self.table_col(t, "pd", "pd")?;
                let def = flags(&self.table_col(t, "default", "default")?);
                self.json(calibration(&grade, &pd, &def), "données invalides")
            }
            k => Err(err(&self.name, format!("type inconnu `{k}`"))),
        }
    }
}

// ---------- Processus externe ----------
/// Protocole : le processus reçoit sur stdin `{"inputs": {...}, "params": {...}}`
/// et écrit un unique document JSON sur stdout. Code de sortie ≠ 0 = échec.
pub struct ProcessStep {
    pub name: String,
    pub version: String,
    pub deps: Vec<String>,
    pub command: Vec<String>,
    /// Fichiers dont le contenu entre dans l'empreinte de code.
    pub code: Vec<PathBuf>,
    pub params: Value,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

impl Step for ProcessStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn deps(&self) -> &[String] {
        &self.deps
    }

    fn code_hash(&self) -> Option<String> {
        let mut buf = self.command.join("\u{1f}").into_bytes();
        for f in &self.code {
            buf.extend(
                f.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
                    .bytes(),
            );
            buf.extend(std::fs::read(f).unwrap_or_else(|_| b"<absent>".to_vec()));
        }
        Some(hash_bytes(&buf))
    }

    fn run(&self, inputs: &Inputs) -> Result<Value, Error> {
        let payload = serde_json::to_vec(&json!({ "inputs": inputs, "params": self.params })).unwrap();
        let mut child = Command::new(&self.command[0])
            .args(&self.command[1..])
            .current_dir(&self.cwd)
            .env("PYTHONHASHSEED", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| err(&self.name, format!("lancement `{}`: {e}", self.command[0])))?;
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&payload);
        });
        let out = std::thread::spawn(move || {
            let mut b = vec![];
            let _ = stdout.read_to_end(&mut b);
            b
        });
        let errs = std::thread::spawn(move || {
            let mut b = vec![];
            let _ = stderr.read_to_end(&mut b);
            b
        });
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait().map_err(|e| err(&self.name, e.to_string()))? {
                Some(s) => break s,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(err(
                        &self.name,
                        format!("délai dépassé ({} s)", self.timeout.as_secs_f64()),
                    ));
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        };
        let stdout = out.join().unwrap_or_default();
        if !status.success() {
            let msg = String::from_utf8_lossy(&errs.join().unwrap_or_default())
                .trim()
                .to_string();
            return Err(err(&self.name, format!("code {status}: {msg}")));
        }
        serde_json::from_slice(&stdout).map_err(|e| err(&self.name, format!("sortie JSON invalide: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sh(script: &str, timeout_ms: u64) -> ProcessStep {
        ProcessStep {
            name: "p".into(),
            version: "1".into(),
            deps: vec![],
            command: vec!["sh".into(), "-c".into(), script.into()],
            code: vec![],
            params: json!({"k": 1}),
            cwd: ".".into(),
            timeout: Duration::from_millis(timeout_ms),
        }
    }

    #[test]
    fn process_roundtrip_with_sh() {
        assert_eq!(sh("cat", 5000).run(&BTreeMap::new()).unwrap()["params"]["k"], 1);
    }

    #[test]
    fn process_failure_is_reported() {
        let e = sh("echo boom >&2; exit 3", 5000)
            .run(&BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(e.contains("boom"));
    }

    #[test]
    fn process_timeout_kills() {
        let t = Instant::now();
        let e = sh("sleep 5", 200).run(&BTreeMap::new()).unwrap_err().to_string();
        assert!(e.contains("délai") && t.elapsed() < Duration::from_secs(3));
    }

    fn builtin(kind: &str, deps: &[&str], params: Value) -> BuiltinStep {
        BuiltinStep {
            name: "b".into(),
            version: "1".into(),
            deps: deps.iter().map(|d| d.to_string()).collect(),
            kind: kind.into(),
            params,
        }
    }

    #[test]
    fn backtest_forecasts_counts_exceptions() {
        let mut i = BTreeMap::new();
        i.insert("r".to_string(), json!([0.0, -0.05, 0.01, -0.001]));
        i.insert("m".to_string(), json!({"start": 1, "var": [0.02, 0.02, 0.02]}));
        let out = builtin("backtest_forecasts", &["r", "m"], json!({"alpha": 0.99}))
            .run(&i)
            .unwrap();
        assert_eq!(out["exceptions"], 1);
        assert!(out["traffic_light"]["zone"].is_string());
    }

    #[test]
    fn pd_steps_read_table_columns() {
        let mut i = BTreeMap::new();
        i.insert(
            "t".to_string(),
            json!({"grade": [1, 1, 2, 2], "pd": [0.1, 0.1, 0.4, 0.4], "default": [0, 0, 1, 0]}),
        );
        let d = builtin("pd_discrimination", &["t"], Value::Null).run(&i).unwrap();
        assert!(d["gini"].as_f64().unwrap() > 0.0);
        let c = builtin("pd_calibration", &["t"], Value::Null).run(&i).unwrap();
        assert_eq!(c["grades"].as_array().unwrap().len(), 2);
    }
}
