use crate::steps::{BuiltinStep, JsonStep, Limits, ProcessStep, TableFormat, TableStep};
use mf_core::{Manifest, Mismatch, Pipeline};
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
    /// Tolerances used by `mf diff` (first matching rule wins).
    #[serde(default, rename = "tolerance")]
    pub tolerances: Vec<mf_core::diff::Tolerance>,
    /// Outputs that make the run status (default: every check's status).
    #[serde(default)]
    pub status_from: Vec<String>,
    /// Alert rules on the series of results (`[[alert]]`).
    #[serde(default, rename = "alert")]
    pub alerts: Vec<crate::monitor::AlertRule>,
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
    let spec: PipelineSpec = toml::from_str(text).map_err(|e| format!("invalid pipeline: {e}"))?;
    for t in &spec.tolerances {
        t.check()?;
    }
    crate::monitor::check_rules(&spec.alerts)?;
    Ok(spec)
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

/// Placeholder of the as-of date in the data paths of a dated pipeline.
pub const AS_OF: &str = "${as_of}";

impl PipelineSpec {
    /// A dated pipeline has `${as_of}` in at least one data path: it is run for one date at a time.
    pub fn dated(&self) -> bool {
        self.steps
            .iter()
            .any(|s| s.path.as_deref().is_some_and(|p| p.contains(AS_OF)))
    }
}

/// An as-of value goes into file paths: letters, digits, `-`, `_` and `.` only, no leading dot.
pub fn check_as_of(d: &str) -> Result<(), String> {
    let ok = !d.is_empty()
        && !d.starts_with('.')
        && d.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(format!("invalid as-of date `{d}` (letters, digits, `-`, `_`, `.`)"))
    }
}

fn dated_path(s: &StepSpec, as_of: Option<&str>) -> Result<Option<String>, String> {
    match (&s.path, as_of) {
        (Some(p), Some(d)) if p.contains(AS_OF) => Ok(Some(p.replace(AS_OF, d))),
        (Some(p), None) if p.contains(AS_OF) => Err(format!(
            "step {}: the pipeline is dated (`{AS_OF}` in `{p}`): give an as-of date",
            s.name
        )),
        (p, _) => Ok(p.clone()),
    }
}

/// Executes the pipeline for `as_of` (dated pipelines) and records the date in the manifest.
pub fn run(spec: &PipelineSpec, base: &Path, as_of: Option<&str>) -> Result<Manifest, String> {
    if as_of.is_some() && !spec.dated() {
        return Err(format!(
            "pipeline {} is not dated (no `{AS_OF}` in its paths)",
            spec.name
        ));
    }
    let mut m = build_at(spec, base, as_of)?.execute().map_err(|e| e.to_string())?;
    m.as_of = as_of.map(str::to_string);
    m.status_from = spec.status_from.clone();
    Ok(m)
}

/// Replays a recorded run, with its own as-of date.
pub fn verify(spec: &PipelineSpec, base: &Path, reference: &Manifest) -> Result<Vec<Mismatch>, String> {
    build_at(spec, base, reference.as_of.as_deref())?
        .verify(reference)
        .map_err(|e| e.to_string())
}

/// As-of dates for which every dated path of the pipeline exists, sorted. The dates are read
/// from the file system: `data/${as_of}/curve.json` lists the directories of `data/`.
pub fn available_dates(spec: &PipelineSpec, base: &Path) -> Result<Vec<String>, String> {
    let mut common: Option<std::collections::BTreeSet<String>> = None;
    for s in &spec.steps {
        let Some(template) = s.path.as_deref().filter(|p| p.contains(AS_OF)) else {
            continue;
        };
        let parts: Vec<&str> = template.split('/').collect();
        let k = parts.iter().position(|c| c.contains(AS_OF)).unwrap();
        let dir = base.join(parts[..k].join("/"));
        let (pre, post) = parts[k].split_once(AS_OF).unwrap();
        let mut found = std::collections::BTreeSet::new();
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("step {}: {}: {e}", s.name, dir.display()))?;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.len() <= pre.len() + post.len() || !name.starts_with(pre) || !name.ends_with(post) {
                continue;
            }
            let d = &name[pre.len()..name.len() - post.len()];
            if check_as_of(d).is_ok() && base.join(template.replace(AS_OF, d)).exists() {
                found.insert(d.to_string());
            }
        }
        common = Some(match common {
            None => found,
            Some(c) => c.intersection(&found).cloned().collect(),
        });
    }
    match common {
        Some(c) => Ok(c.into_iter().collect()),
        None => Err(format!(
            "pipeline {} is not dated (no `{AS_OF}` in its paths)",
            spec.name
        )),
    }
}

pub fn build(spec: &PipelineSpec, base: &Path) -> Result<Pipeline, String> {
    build_at(spec, base, None)
}

/// Builds the pipeline for one as-of date (`None` for an undated pipeline).
pub fn build_at(spec: &PipelineSpec, base: &Path, as_of: Option<&str>) -> Result<Pipeline, String> {
    if let Some(d) = as_of {
        check_as_of(d)?;
    }
    let mut p = Pipeline::new();
    for s in &spec.steps {
        let path = dated_path(s, as_of)?;
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
                    path: base.join(need(&path, "path")?),
                    columns,
                    table,
                    format,
                })
            }
            "json" => p.step(JsonStep {
                name: s.name.clone(),
                version: s.version.clone(),
                path: base.join(need(&path, "path")?),
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
                    as_of: as_of.map(str::to_string),
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
    fn dated_pipelines() {
        let dir = std::env::temp_dir().join(format!("mf-dated-{}", std::process::id()));
        for d in ["2024-01-31", "2024-02-29", "2024-03-29"] {
            std::fs::create_dir_all(dir.join("data").join(d)).unwrap();
            std::fs::write(dir.join("data").join(d).join("r.csv"), "x\n1\n2\n").unwrap();
        }
        std::fs::create_dir_all(dir.join("data/.hidden")).unwrap();
        std::fs::write(dir.join("m-2024-01-31.json"), "{}").unwrap();
        std::fs::write(dir.join("m-2024-02-29.json"), "{}").unwrap();
        let s = parse(
            "name='d'\n[[step]]\nname='r'\nkind='csv'\nversion='1'\npath='data/${as_of}/r.csv'\ncolumn='x'\n\
             [[step]]\nname='m'\nkind='json'\nversion='1'\npath='m-${as_of}.json'\n",
        )
        .unwrap();
        assert!(s.dated());
        assert_eq!(available_dates(&s, &dir).unwrap(), ["2024-01-31", "2024-02-29"]);
        let m = run(&s, &dir, Some("2024-02-29")).unwrap();
        assert_eq!(m.as_of.as_deref(), Some("2024-02-29"));
        assert!(verify(&s, &dir, &m).unwrap().is_empty());
        assert!(run(&s, &dir, None).unwrap_err().contains("dated"));
        assert!(run(&s, &dir, Some("../etc")).unwrap_err().contains("invalid as-of"));
        let undated = parse("name='u'\nstep=[]\n").unwrap();
        assert!(run(&undated, &dir, Some("2024-01-31"))
            .unwrap_err()
            .contains("not dated"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_unknown_kind() {
        let s = parse("name='x'\n[[step]]\nname='a'\nkind='nope'\nversion='1'\n").unwrap();
        assert!(build(&s, Path::new(".")).is_err());
    }

    #[test]
    fn parses_tolerances() {
        let s = parse(
            "name='x'\nstep=[]\n[[tolerance]]\npath='curve.*'\nbp=0.5\n[[tolerance]]\npath='*.pnl'\nignore=true\n",
        )
        .unwrap();
        assert_eq!(s.tolerances.len(), 2);
        assert_eq!(s.tolerances[0].bp, Some(0.5));
        assert!(s.tolerances[1].ignore);
        assert!(parse("name='x'\nstep=[]\n[[tolerance]]\nabs=1\nbp=1\n").is_err());
        assert!(parse("name='x'\nstep=[]\n[[tolerance]]\ntol=1\n").is_err());
    }

    #[test]
    fn parses_status_and_alerts() {
        let s = parse(
            "name='x'\nstatus_from=['b.zone']\nstep=[]\n[[alert]]\nname='red'\npath='b.zone'\none_of=['red']\n\
             [[alert]]\nname='psi'\npath='d.psi'\nabove=0.25\nconsecutive=3\n",
        )
        .unwrap();
        assert_eq!(s.status_from, ["b.zone"]);
        assert_eq!(s.alerts[1].consecutive, 3);
        assert!(parse("name='x'\nstep=[]\n[[alert]]\nname='a'\npath='b'\n").is_err());
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
