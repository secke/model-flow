use crate::steps::{BuiltinStep, JsonStep, Limits, ProcessStep, TableFormat, TableStep};
use mf_core::Pipeline;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct PipelineSpec {
    pub name: String,
    /// Model identifier in the inventory (optional).
    #[serde(default)]
    pub model: Option<String>,
    /// 5-field cron schedule, in UTC (e.g. `"0 6 * * 1-5"`), run by the server.
    #[serde(default)]
    pub schedule: Option<String>,
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
    pub delimiter: Option<String>,
    // process
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub code: Vec<String>,
    /// Maximum duration of an external process (default: 300 s).
    pub timeout_secs: Option<u64>,
    /// Environment variables passed to the process (the environment is cleared by default).
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub pass_env: Vec<String>,
    pub memory_mb: Option<u64>,
    pub cpu_secs: Option<u64>,
    pub file_size_mb: Option<u64>,
    pub max_output_mb: Option<u64>,
}

pub fn parse(text: &str) -> Result<PipelineSpec, String> {
    toml::from_str(text).map_err(|e| format!("invalid pipeline: {e}"))
}

/// Loads a pipeline file; relative paths start from its directory.
pub fn load(path: &Path) -> Result<(PipelineSpec, PathBuf), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((parse(&text)?, base_dir(path)))
}

/// Directory of a pipeline file. A bare file name (`pipeline.toml`) has an empty parent,
/// which is not a valid working directory for external processes: use `.` instead.
fn base_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

pub fn build(spec: &PipelineSpec, base: &Path) -> Result<Pipeline, String> {
    let mut p = Pipeline::new();
    for s in &spec.steps {
        let need = |o: &Option<String>, f: &str| o.clone().ok_or(format!("step {}: field `{f}` is required", s.name));
        p = match s.kind.as_str() {
            "csv" | "parquet" => {
                let (columns, table) = match (&s.column, s.columns.is_empty()) {
                    (Some(c), true) => (vec![c.clone()], false),
                    (None, false) => (s.columns.clone(), true),
                    _ => return Err(format!("step {}: specify `column` or `columns`", s.name)),
                };
                let format = if s.kind == "parquet" {
                    TableFormat::Parquet
                } else {
                    let d = s.delimiter.as_deref().unwrap_or(",");
                    if d.len() != 1 {
                        return Err(format!("step {}: `delimiter` must be an ASCII character", s.name));
                    }
                    TableFormat::Csv {
                        delimiter: d.as_bytes()[0],
                    }
                };
                p.step(TableStep {
                    name: s.name.clone(),
                    version: s.version.clone(),
                    path: base.join(need(&s.path, "path")?),
                    columns,
                    table,
                    format,
                })
            }
            "json" => p.step(JsonStep {
                name: s.name.clone(),
                version: s.version.clone(),
                path: base.join(need(&s.path, "path")?),
            }),
            "process" => {
                let command = s
                    .command
                    .clone()
                    .filter(|c| !c.is_empty())
                    .ok_or(format!("step {}: `command` is required", s.name))?;
                p.step(ProcessStep {
                    name: s.name.clone(),
                    version: s.version.clone(),
                    deps: s.deps.clone(),
                    command,
                    code: s.code.iter().map(|c| base.join(c)).collect(),
                    params: s.params.clone(),
                    cwd: base.to_path_buf(),
                    timeout: std::time::Duration::from_secs(s.timeout_secs.unwrap_or(300)),
                    env: s.env.clone(),
                    pass_env: s.pass_env.clone(),
                    limits: Limits {
                        memory_mb: s.memory_mb,
                        cpu_secs: s.cpu_secs.or(Some(s.timeout_secs.unwrap_or(300))),
                        file_size_mb: s.file_size_mb,
                        max_output_mb: s.max_output_mb.unwrap_or(256),
                    },
                })
            }
            k if BuiltinStep::KINDS.contains(&k) => p.step(BuiltinStep {
                name: s.name.clone(),
                version: s.version.clone(),
                deps: s.deps.clone(),
                kind: k.to_string(),
                params: s.params.clone(),
            }),
            k => return Err(format!("step {}: unknown type `{k}`", s.name)),
        };
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_file_name_runs_in_current_directory() {
        assert_eq!(base_dir(Path::new("pipeline.toml")), PathBuf::from("."));
        assert_eq!(base_dir(Path::new("cases/p/pipeline.toml")), PathBuf::from("cases/p"));
        assert_eq!(base_dir(Path::new("/w/pipeline.toml")), PathBuf::from("/w"));
    }

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
