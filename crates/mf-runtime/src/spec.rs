use crate::steps::{BuiltinStep, CsvStep, ProcessStep};
use mf_core::Pipeline;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct PipelineSpec {
    pub name: String,
    #[serde(rename = "step")]
    pub steps: Vec<StepSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StepSpec {
    pub name: String,
    pub kind: String,
    pub version: String,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub params: Value,
    // csv
    pub path: Option<String>,
    pub column: Option<String>,
    #[serde(default)]
    pub columns: Vec<String>,
    // process
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub code: Vec<String>,
    /// Délai maximal d'un processus externe (défaut : 300 s).
    pub timeout_secs: Option<u64>,
}

pub fn parse(text: &str) -> Result<PipelineSpec, String> {
    toml::from_str(text).map_err(|e| format!("pipeline invalide: {e}"))
}

/// Charge un fichier pipeline ; les chemins relatifs partent de son dossier.
pub fn load(path: &Path) -> Result<(PipelineSpec, PathBuf), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok((parse(&text)?, base))
}

pub fn build(spec: &PipelineSpec, base: &Path) -> Result<Pipeline, String> {
    let mut p = Pipeline::new();
    for s in &spec.steps {
        let need = |o: &Option<String>, f: &str| o.clone().ok_or(format!("étape {}: champ `{f}` requis", s.name));
        p = match s.kind.as_str() {
            "csv" => {
                let (columns, table) = match (&s.column, s.columns.is_empty()) {
                    (Some(c), true) => (vec![c.clone()], false),
                    (None, false) => (s.columns.clone(), true),
                    _ => return Err(format!("étape {}: indiquer `column` ou `columns`", s.name)),
                };
                p.step(CsvStep {
                    name: s.name.clone(),
                    version: s.version.clone(),
                    path: base.join(need(&s.path, "path")?),
                    columns,
                    table,
                })
            }
            "process" => {
                let command = s
                    .command
                    .clone()
                    .filter(|c| !c.is_empty())
                    .ok_or(format!("étape {}: `command` requis", s.name))?;
                p.step(ProcessStep {
                    name: s.name.clone(),
                    version: s.version.clone(),
                    deps: s.deps.clone(),
                    command,
                    code: s.code.iter().map(|c| base.join(c)).collect(),
                    params: s.params.clone(),
                    cwd: base.to_path_buf(),
                    timeout: std::time::Duration::from_secs(s.timeout_secs.unwrap_or(300)),
                })
            }
            k if BuiltinStep::KINDS.contains(&k) => p.step(BuiltinStep {
                name: s.name.clone(),
                version: s.version.clone(),
                deps: s.deps.clone(),
                kind: k.to_string(),
                params: s.params.clone(),
            }),
            k => return Err(format!("étape {}: type inconnu `{k}`", s.name)),
        };
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_kind() {
        let s = parse("name='x'\n[[step]]\nname='a'\nkind='nope'\nversion='1'\n").unwrap();
        assert!(build(&s, Path::new(".")).is_err());
    }

    #[test]
    fn parses_params() {
        let s = parse(
            "name='x'\n[[step]]\nname='v'\nkind='var_historical'\nversion='1'\ndeps=['r']\nparams={alpha=0.99}\n",
        )
        .unwrap();
        assert_eq!(s.steps[0].params["alpha"], 0.99);
    }
}
