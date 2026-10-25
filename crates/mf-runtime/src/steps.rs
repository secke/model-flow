use mf_core::{hash_bytes, Error, Inputs, Step};
use mf_validate::{historical_var, kupiec_pof, rolling_backtest};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn err(step: &str, msg: impl Into<String>) -> Error {
    Error::Step { step: step.into(), msg: msg.into() }
}

fn series(v: &Value, step: &str) -> Result<Vec<f64>, Error> {
    let arr = v.as_array().or_else(|| v.get("values").and_then(Value::as_array))
        .ok_or_else(|| err(step, "série attendue (tableau de nombres)"))?;
    arr.iter().map(|x| x.as_f64().ok_or_else(|| err(step, "valeur non numérique"))).collect()
}

fn param_f64(p: &Value, k: &str, default: f64) -> f64 { p.get(k).and_then(Value::as_f64).unwrap_or(default) }

// ---------- CSV ----------
pub struct CsvStep {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub column: String,
}

impl Step for CsvStep {
    fn name(&self) -> &str { &self.name }
    fn version(&self) -> &str { &self.version }
    fn deps(&self) -> &[String] { &[] }
    fn run(&self, _: &Inputs) -> Result<Value, Error> {
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| err(&self.name, format!("{}: {e}", self.path.display())))?;
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let header = lines.next().ok_or_else(|| err(&self.name, "fichier vide"))?;
        let col = header.split(',').position(|h| h.trim() == self.column)
            .ok_or_else(|| err(&self.name, format!("colonne `{}` absente", self.column)))?;
        let mut out = vec![];
        for (i, l) in lines.enumerate() {
            let cell = l.split(',').nth(col).ok_or_else(|| err(&self.name, format!("ligne {}: colonne manquante", i + 2)))?;
            out.push(cell.trim().parse::<f64>().map_err(|_| err(&self.name, format!("ligne {}: `{cell}` non numérique", i + 2)))?);
        }
        Ok(json!(out))
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
    pub const KINDS: &'static [&'static str] = &["var_historical", "kupiec_backtest", "backtest_forecasts"];

    fn dep<'a>(&self, inputs: &'a Inputs, key: &str, idx: usize) -> Result<&'a Value, Error> {
        let name = self.params.get(key).and_then(Value::as_str)
            .or_else(|| self.deps.get(idx).map(String::as_str))
            .ok_or_else(|| err(&self.name, format!("dépendance `{key}` manquante")))?;
        inputs.get(name).ok_or_else(|| err(&self.name, format!("`{name}` n'est pas déclaré dans deps")))
    }
}

impl Step for BuiltinStep {
    fn name(&self) -> &str { &self.name }
    fn version(&self) -> &str { &self.version }
    fn deps(&self) -> &[String] { &self.deps }
    fn run(&self, inputs: &Inputs) -> Result<Value, Error> {
        let alpha = param_f64(&self.params, "alpha", 0.99);
        match self.kind.as_str() {
            "var_historical" => {
                let r = series(self.dep(inputs, "series", 0)?, &self.name)?;
                let v = historical_var(&r, alpha).ok_or_else(|| err(&self.name, "série vide"))?;
                Ok(json!({ "alpha": alpha, "var": v, "observations": r.len() }))
            }
            "kupiec_backtest" => {
                let r = series(self.dep(inputs, "series", 0)?, &self.name)?;
                let w = param_f64(&self.params, "window", 250.0) as usize;
                let k = rolling_backtest(&r, w, alpha).ok_or_else(|| err(&self.name, "série trop courte"))?;
                Ok(serde_json::to_value(k).unwrap())
            }
            // Compare des prévisions de VaR produites par un modèle externe aux rendements réalisés.
            // Le modèle renvoie {"start": i, "var": [...]} : var[j] est la prévision pour returns[start + j].
            "backtest_forecasts" => {
                let r = series(self.dep(inputs, "returns", 0)?, &self.name)?;
                let f = self.dep(inputs, "forecasts", 1)?;
                let start = f.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
                let var = series(f.get("var").unwrap_or(&Value::Null), &self.name)?;
                if start + var.len() > r.len() { return Err(err(&self.name, "prévisions plus longues que la série")); }
                if var.is_empty() { return Err(err(&self.name, "aucune prévision")); }
                let exc = var.iter().enumerate().filter(|(j, v)| r[start + j] < -**v).count();
                Ok(serde_json::to_value(kupiec_pof(var.len(), exc, alpha)).unwrap())
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
}

impl Step for ProcessStep {
    fn name(&self) -> &str { &self.name }
    fn version(&self) -> &str { &self.version }
    fn deps(&self) -> &[String] { &self.deps }

    fn code_hash(&self) -> Option<String> {
        let mut buf = self.command.join("\u{1f}").into_bytes();
        for f in &self.code {
            buf.extend(f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().bytes());
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
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn()
            .map_err(|e| err(&self.name, format!("lancement `{}`: {e}", self.command[0])))?;
        let mut stdin = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || stdin.write_all(&payload));
        let out = child.wait_with_output().map_err(|e| err(&self.name, e.to_string()))?;
        let _ = writer.join();
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr);
            return Err(err(&self.name, format!("code {}: {}", out.status, msg.trim())));
        }
        serde_json::from_slice(&out.stdout).map_err(|e| err(&self.name, format!("sortie JSON invalide: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn process_roundtrip_with_sh() {
        let s = ProcessStep {
            name: "echo".into(), version: "1".into(), deps: vec![],
            command: vec!["sh".into(), "-c".into(), "cat".into()],
            code: vec![], params: json!({"k": 1}), cwd: ".".into(),
        };
        let out = s.run(&BTreeMap::new()).unwrap();
        assert_eq!(out["params"]["k"], 1);
    }

    #[test]
    fn process_failure_is_reported() {
        let s = ProcessStep {
            name: "bad".into(), version: "1".into(), deps: vec![],
            command: vec!["sh".into(), "-c".into(), "echo boom >&2; exit 3".into()],
            code: vec![], params: Value::Null, cwd: ".".into(),
        };
        let e = s.run(&BTreeMap::new()).unwrap_err().to_string();
        assert!(e.contains("boom"));
    }

    #[test]
    fn backtest_forecasts_counts_exceptions() {
        let mut i = BTreeMap::new();
        i.insert("r".to_string(), json!([0.0, -0.05, 0.01, -0.001]));
        i.insert("m".to_string(), json!({"start": 1, "var": [0.02, 0.02, 0.02]}));
        let s = BuiltinStep { name: "bt".into(), version: "1".into(), deps: vec!["r".into(), "m".into()],
            kind: "backtest_forecasts".into(), params: json!({"alpha": 0.99}) };
        assert_eq!(s.run(&i).unwrap()["exceptions"], 1);
    }
}
