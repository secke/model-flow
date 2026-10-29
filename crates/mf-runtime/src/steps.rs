use mf_core::{hash_bytes, Error, Inputs, Step};
use quantparity::benchmark::{compare, prices_from_value, reference_prices, trades_from_value, Tolerances};
use quantparity::curves::Market;
use quantparity::validation::{calibration, discrimination, historical_var, psi, rolling_backtest, var_backtest};
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
        .map(|x| x.as_f64().ok_or_else(|| err(step, "non-numeric value")))
        .collect()
}

/// Numeric series: array, `{"values": [...]}` object, or column `col` of a `{col: [...]}` table.
fn series(v: &Value, col: Option<&str>, step: &str) -> Result<Vec<f64>, Error> {
    let arr = match col {
        Some(c) => v
            .get(c)
            .and_then(Value::as_array)
            .ok_or_else(|| err(step, format!("column `{c}` not found")))?,
        None => v
            .as_array()
            .or_else(|| v.get("values").and_then(Value::as_array))
            .ok_or_else(|| err(step, "expected a series (array of numbers)"))?,
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

/// Identifier of the built-in step implementation: an engine version bump
/// changes their code fingerprint, which replay reports.
pub const ENGINE: &str = concat!("mf-runtime ", env!("CARGO_PKG_VERSION"));

fn engine_hash(kind: &str) -> Option<String> {
    Some(hash_bytes(format!("{ENGINE} {kind}").as_bytes()))
}

// ---------- Tables (CSV, Parquet) ----------
#[derive(Debug, Clone, PartialEq)]
pub enum TableFormat {
    Csv { delimiter: u8 },
    Parquet,
}

/// Reads one numeric column (series) or several numeric columns (`{col: [...]}` table) from a CSV
/// file (RFC 4180: quoting, configurable delimiter) or a Parquet file.
pub struct TableStep {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub columns: Vec<String>,
    pub table: bool,
    pub format: TableFormat,
}

fn read_csv(path: &std::path::Path, columns: &[String], delimiter: u8) -> Result<Vec<Vec<f64>>, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .trim(csv::Trim::All)
        .from_path(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let header = rdr.headers().map_err(|e| e.to_string())?.clone();
    let idx: Vec<usize> = columns
        .iter()
        .map(|c| {
            header
                .iter()
                .position(|h| h == c)
                .ok_or_else(|| format!("column `{c}` not found"))
        })
        .collect::<Result<_, _>>()?;
    let mut cols = vec![vec![]; idx.len()];
    for (n, rec) in rdr.records().enumerate() {
        let rec = rec.map_err(|e| format!("line {}: {e}", n + 2))?;
        for (k, &i) in idx.iter().enumerate() {
            let cell = rec.get(i).ok_or_else(|| format!("line {}: missing column", n + 2))?;
            cols[k].push(
                cell.parse::<f64>()
                    .map_err(|_| format!("line {}: `{cell}` is not numeric", n + 2))?,
            );
        }
    }
    Ok(cols)
}

fn read_parquet(path: &std::path::Path, columns: &[String]) -> Result<Vec<Vec<f64>>, String> {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use parquet::record::Field;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let reader = SerializedFileReader::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
    let schema = reader.metadata().file_metadata().schema_descr_ptr();
    for c in columns {
        if !schema.columns().iter().any(|d| d.name() == c) {
            return Err(format!("column `{c}` not found"));
        }
    }
    let mut cols = vec![vec![]; columns.len()];
    for (n, row) in reader.get_row_iter(None).map_err(|e| e.to_string())?.enumerate() {
        let row = row.map_err(|e| e.to_string())?;
        for (name, field) in row.get_column_iter() {
            if let Some(k) = columns.iter().position(|c| c == name) {
                let v = match field {
                    Field::Double(x) => *x,
                    Field::Float(x) => *x as f64,
                    Field::Long(x) => *x as f64,
                    Field::Int(x) => *x as f64,
                    Field::Short(x) => *x as f64,
                    Field::Byte(x) => *x as f64,
                    Field::UInt(x) => *x as f64,
                    Field::ULong(x) => *x as f64,
                    Field::Bool(b) => f64::from(u8::from(*b)),
                    Field::Null => return Err(format!("line {}: missing value in `{name}`", n + 1)),
                    other => return Err(format!("line {}: `{name}` is not numeric ({other})", n + 1)),
                };
                cols[k].push(v);
            }
        }
    }
    Ok(cols)
}

impl Step for TableStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn deps(&self) -> &[String] {
        &[]
    }
    fn code_hash(&self) -> Option<String> {
        engine_hash(&format!("table {:?}", self.format))
    }
    fn run(&self, _: &Inputs) -> Result<Value, Error> {
        let mut cols = match self.format {
            TableFormat::Csv { delimiter } => read_csv(&self.path, &self.columns, delimiter),
            TableFormat::Parquet => read_parquet(&self.path, &self.columns),
        }
        .map_err(|e| err(&self.name, e))?;
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

// ---------- JSON ----------
/// Reads a JSON document (trades, market parameters, prices…).
pub struct JsonStep {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
}

impl Step for JsonStep {
    fn code_hash(&self) -> Option<String> {
        engine_hash("json")
    }
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
        serde_json::from_str(&text).map_err(|e| err(&self.name, format!("invalid JSON: {e}")))
    }
}

// ---------- Built-in steps ----------
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
        "price_portfolio",
        "pricing_benchmark",
        "curve_bootstrap",
        "portfolio_var",
        "sensitivities",
        "stress_test",
        "pnl_attribution",
        "pd_discrimination",
        "pd_calibration",
    ];

    /// Input named by the `key` parameter, otherwise dependency number `idx`.
    fn dep<'a>(&self, inputs: &'a Inputs, key: &str, idx: usize) -> Result<&'a Value, Error> {
        let name = self
            .params
            .get(key)
            .and_then(Value::as_str)
            .or_else(|| self.deps.get(idx).map(String::as_str))
            .ok_or_else(|| err(&self.name, format!("missing dependency `{key}`")))?;
        inputs
            .get(name)
            .ok_or_else(|| err(&self.name, format!("`{name}` is not declared in deps")))
    }

    fn column(&self) -> Option<&str> {
        self.params.get("column").and_then(Value::as_str)
    }

    fn table_col(&self, t: &Value, key: &str, default: &str) -> Result<Vec<f64>, Error> {
        series(t, Some(param_str(&self.params, key, default)), &self.name)
    }

    /// Raw market document (JSON) named by `market`, or `null`.
    fn market_value(&self, inputs: &Inputs) -> Result<Value, Error> {
        match self.params.get("market").and_then(Value::as_str) {
            Some(_) => Ok(self.dep(inputs, "market", usize::MAX)?.clone()),
            None => Ok(Value::Null),
        }
    }

    /// Market curves: dependency named by the `market` parameter (optional).
    fn market(&self, inputs: &Inputs) -> Result<Market, Error> {
        match self.params.get("market").and_then(Value::as_str) {
            Some(_) => Market::from_value(self.dep(inputs, "market", usize::MAX)?).map_err(|e| err(&self.name, e)),
            None => Ok(Market::default()),
        }
    }

    fn json<T: serde::Serialize>(&self, v: Option<T>, why: &str) -> Result<Value, Error> {
        v.map(|x| serde_json::to_value(x).unwrap())
            .ok_or_else(|| err(&self.name, why))
    }
}

impl Step for BuiltinStep {
    fn code_hash(&self) -> Option<String> {
        engine_hash(&self.kind)
    }
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
                let v = historical_var(&r, alpha).ok_or_else(|| err(&self.name, "empty or invalid series"))?;
                Ok(json!({ "alpha": alpha, "var": v, "observations": r.len() }))
            }
            "kupiec_backtest" => {
                let r = series(self.dep(inputs, "series", 0)?, self.column(), &self.name)?;
                let w = param_f64(&self.params, "window", 250.0) as usize;
                self.json(rolling_backtest(&r, w, alpha), "series too short")
            }
            // Forecasts from an external model: {"start": i, "var": [...]}, var[j] forecasts returns[start + j].
            "backtest_forecasts" => {
                let r = series(self.dep(inputs, "returns", 0)?, self.column(), &self.name)?;
                let f = self.dep(inputs, "forecasts", 1)?;
                let start = f.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
                let var = series(f.get("var").unwrap_or(&Value::Null), None, &self.name)?;
                if var.is_empty() || start + var.len() > r.len() {
                    return Err(err(&self.name, "forecasts are empty or longer than the series"));
                }
                self.json(
                    var_backtest(&r[start..start + var.len()], &var, alpha),
                    "backtest not possible",
                )
            }
            "psi" => {
                let col = self.column();
                let base = series(self.dep(inputs, "baseline", 0)?, col, &self.name)?;
                let cur = series(self.dep(inputs, "current", 1)?, col, &self.name)?;
                let bins = param_f64(&self.params, "bins", 10.0) as usize;
                self.json(psi(&base, &cur, bins), "empty samples")
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
                    "invalid data (at least one default and one non-default are required)",
                )
            }
            "pd_calibration" => {
                let t = self.dep(inputs, "table", 0)?;
                let grade = self.table_col(t, "grade", "grade")?;
                let pd = self.table_col(t, "pd", "pd")?;
                let def = flags(&self.table_col(t, "default", "default")?);
                self.json(calibration(&grade, &pd, &def), "invalid data")
            }
            "price_portfolio" => {
                let trades = trades_from_value(self.dep(inputs, "trades", 0)?).map_err(|e| err(&self.name, e))?;
                let market = self.market(inputs)?;
                let vals: Result<Vec<_>, _> = trades
                    .iter()
                    .map(|t| quantparity::value_in(t, &market).map_err(|e| err(&self.name, format!("{}: {e}", t.id))))
                    .collect();
                Ok(serde_json::to_value(vals?).unwrap())
            }
            // Candidate pricer vs. reference (`reference` dependency, otherwise the built-in library).
            "pricing_benchmark" => {
                let trades = trades_from_value(self.dep(inputs, "trades", 0)?).map_err(|e| err(&self.name, e))?;
                let candidate = prices_from_value(self.dep(inputs, "candidate", 1)?).map_err(|e| err(&self.name, e))?;
                let reference = match self.params.get("reference").and_then(Value::as_str) {
                    Some(_) => prices_from_value(self.dep(inputs, "reference", 2)?),
                    None => reference_prices(&trades, &self.market(inputs)?),
                }
                .map_err(|e| err(&self.name, e))?;
                let tol = Tolerances::from_value(self.params.get("tolerances").unwrap_or(&Value::Null))
                    .map_err(|e| err(&self.name, e))?;
                let b = compare(&trades, &reference, &candidate, &tol).map_err(|e| err(&self.name, e))?;
                Ok(serde_json::to_value(b).unwrap())
            }
            // Portfolio risk: full revaluation on historical scenarios, VaR/ES, limits.
            "portfolio_var" => {
                let trades = trades_from_value(self.dep(inputs, "trades", 0)?).map_err(|e| err(&self.name, e))?;
                let market = self.market_value(inputs)?;
                let table = self.dep(inputs, "scenarios", 1)?;
                let label = self.params.get("label").and_then(Value::as_str);
                let sc = quantparity::risk::scenarios_from_table(table, label).map_err(|e| err(&self.name, e))?;
                let (base, pnl) = quantparity::risk::revalue(&trades, &market, &sc).map_err(|e| err(&self.name, e))?;
                let totals: Vec<f64> = pnl.iter().map(|p| p.pnl).collect();
                let cv = param_f64(&self.params, "var_confidence", 0.99);
                let ce = param_f64(&self.params, "es_confidence", 0.975);
                let r = quantparity::risk::var_es(&totals, cv, ce).ok_or_else(|| err(&self.name, "no scenario"))?;
                let mut order: Vec<usize> = (0..pnl.len()).collect();
                order.sort_by(|&a, &b| totals[a].partial_cmp(&totals[b]).unwrap());
                let worst: Vec<Value> = order
                    .iter()
                    .take(5)
                    .map(|&i| json!({"scenario": pnl[i].name, "pnl": totals[i]}))
                    .collect();
                // Position contributions to the scenario that sets the VaR.
                let k = order.iter().position(|&i| -totals[i] <= r.var + 1e-9).unwrap_or(0);
                let at_var = &pnl[order[k]];
                let contributions: Vec<Value> = trades
                    .iter()
                    .zip(&at_var.by_trade)
                    .map(|(t, p)| json!({"id": t.id, "pnl": p}))
                    .collect();
                let mut out = json!({
                    "base_value": base.iter().sum::<f64>(), "observations": r.observations,
                    "var_confidence": cv, "var": r.var, "es_confidence": ce, "es": r.es,
                    "worst_scenarios": worst, "var_scenario": at_var.name, "contributions": contributions, "pnl": totals,
                });
                let mut status = None;
                for (key, value) in [("var_limit", r.var), ("es_limit", r.es)] {
                    if let Some(l) = self.params.get(key).and_then(Value::as_f64) {
                        let u = value / l;
                        out[format!("{key}_utilisation")] = json!(u);
                        out[key] = json!(l);
                        let s = if u > 1.0 {
                            "fail"
                        } else if u > 0.8 {
                            "warn"
                        } else {
                            "pass"
                        };
                        status = Some(match (status, s) {
                            (Some("fail"), _) | (_, "fail") => "fail",
                            (Some("warn"), _) | (_, "warn") => "warn",
                            _ => "pass",
                        });
                    }
                }
                if let Some(s) = status {
                    out["status"] = json!(s);
                }
                Ok(out)
            }
            "sensitivities" => {
                let trades = trades_from_value(self.dep(inputs, "trades", 0)?).map_err(|e| err(&self.name, e))?;
                let market = self.market_value(inputs)?;
                let s = quantparity::risk::sensitivities(&trades, &market).map_err(|e| err(&self.name, e))?;
                Ok(json!({"factors": s}))
            }
            "stress_test" => {
                let trades = trades_from_value(self.dep(inputs, "trades", 0)?).map_err(|e| err(&self.name, e))?;
                let market = self.market_value(inputs)?;
                let doc = self.dep(inputs, "scenarios", 1)?;
                let sc: Vec<quantparity::risk::Scenario> =
                    serde_json::from_value(doc.get("scenarios").cloned().unwrap_or_else(|| doc.clone()))
                        .map_err(|e| err(&self.name, format!("invalid scenarios: {e}")))?;
                let r = quantparity::risk::stress(&trades, &market, &sc).map_err(|e| err(&self.name, e))?;
                let status = r.iter().map(|x| x.status).max().unwrap_or(quantparity::Status::Pass);
                Ok(json!({"scenarios": r, "status": status}))
            }
            // P&L attribution test (FRTB): hypothetical P&L vs. risk-theoretical P&L.
            "pnl_attribution" => {
                let hpl = series(
                    self.dep(inputs, "hypothetical", 0)?.get("pnl").unwrap_or(&Value::Null),
                    None,
                    &self.name,
                )?;
                let rtpl = match self.params.get("risk_theoretical").and_then(Value::as_str) {
                    Some(_) => series(
                        self.dep(inputs, "risk_theoretical", 1)?
                            .get("pnl")
                            .unwrap_or(&Value::Null),
                        None,
                        &self.name,
                    )?,
                    None => {
                        let sens: Vec<quantparity::risk::Sensitivity> =
                            serde_json::from_value(self.dep(inputs, "sensitivities", 1)?["factors"].clone())
                                .map_err(|e| err(&self.name, format!("invalid sensitivities: {e}")))?;
                        let label = self.params.get("label").and_then(Value::as_str);
                        let sc = quantparity::risk::scenarios_from_table(self.dep(inputs, "scenarios", 2)?, label)
                            .map_err(|e| err(&self.name, e))?;
                        quantparity::risk::risk_theoretical_pnl(&sens, &sc)
                    }
                };
                if hpl.len() != rtpl.len() {
                    return Err(err(
                        &self.name,
                        format!("series have different lengths ({} and {})", hpl.len(), rtpl.len()),
                    ));
                }
                let min = param_f64(&self.params, "min_observations", 250.0) as usize;
                self.json(
                    quantparity::risk::pnl_attribution(&hpl, &rtpl, min),
                    "insufficient series",
                )
            }
            // Curve construction and re-pricing check of the input instruments.
            "curve_bootstrap" => {
                let market = Market::from_value(self.dep(inputs, "market", 0)?).map_err(|e| err(&self.name, e))?;
                let all: Vec<String> = market.curves.keys().chain(market.dated.keys()).cloned().collect();
                let name = match self.params.get("curve").and_then(Value::as_str) {
                    Some(n) => n.to_string(),
                    None if all.len() == 1 => all[0].clone(),
                    None => return Err(err(&self.name, "several curves: specify `curve`")),
                };
                quantparity::curves::simple::curve_report(
                    &market,
                    &name,
                    param_f64(&self.params, "tolerance", 1e-10),
                    param_f64(&self.params, "grid_step", 1.0),
                )
                .map_err(|e| err(&self.name, e))
            }
            k => Err(err(&self.name, format!("unknown type `{k}`"))),
        }
    }
}

// ---------- External process ----------
/// Limits applied to the external process (Unix: `setrlimit`).
#[derive(Debug, Clone)]
pub struct Limits {
    /// Maximum virtual memory (RLIMIT_AS), in MB.
    pub memory_mb: Option<u64>,
    /// Maximum CPU time (RLIMIT_CPU), in seconds.
    pub cpu_secs: Option<u64>,
    /// Maximum size of a written file (RLIMIT_FSIZE), in MB.
    pub file_size_mb: Option<u64>,
    /// Maximum size of standard output, in MB.
    pub max_output_mb: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_mb: None,
            cpu_secs: None,
            file_size_mb: None,
            max_output_mb: 256,
        }
    }
}

/// Protocol: the process receives `{"inputs": {...}, "params": {...}}` on stdin
/// and writes a single JSON document to stdout. Exit code ≠ 0 = failure.
///
/// Isolation: cleared environment (only `PATH`, `LANG`, `HOME`, `TMPDIR`, `PYTHONHASHSEED`, the
/// `env` variables and those listed in `pass_env` are passed), resource limits,
/// output cap, dedicated process group killed at the end (no orphan processes).
pub struct ProcessStep {
    pub name: String,
    pub version: String,
    pub deps: Vec<String>,
    pub command: Vec<String>,
    /// Files whose content is part of the code fingerprint.
    pub code: Vec<PathBuf>,
    pub params: Value,
    pub cwd: PathBuf,
    pub timeout: Duration,
    pub env: std::collections::BTreeMap<String, String>,
    pub pass_env: Vec<String>,
    pub limits: Limits,
}

#[cfg(unix)]
fn kill_group(pid: u32) {
    // SAFETY: system call with no effect on the current process's memory.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

#[cfg(unix)]
fn apply_limits(cmd: &mut Command, l: &Limits) {
    use std::os::unix::process::CommandExt;
    let mb = |x: u64| x.saturating_mul(1024 * 1024);
    let (mem, cpu, fsize) = (l.memory_mb.map(mb), l.cpu_secs, l.file_size_mb.map(mb));
    cmd.process_group(0);
    // SAFETY: the closure only calls setrlimit, which is safe between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            let set = |res, v: u64| {
                let r = libc::rlimit {
                    rlim_cur: v as libc::rlim_t,
                    rlim_max: v as libc::rlim_t,
                };
                if libc::setrlimit(res, &r) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            };
            set(libc::RLIMIT_CORE, 0)?;
            if let Some(v) = mem {
                set(libc::RLIMIT_AS, v)?;
            }
            if let Some(v) = cpu {
                set(libc::RLIMIT_CPU, v)?;
            }
            if let Some(v) = fsize {
                set(libc::RLIMIT_FSIZE, v)?;
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn apply_limits(_cmd: &mut Command, _l: &Limits) {}

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
        // The explicit environment affects the computation; only the names of passed variables count.
        buf.extend(serde_json::to_vec(&(&self.env, &self.pass_env)).unwrap());
        Some(hash_bytes(&buf))
    }

    fn run(&self, inputs: &Inputs) -> Result<Value, Error> {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let payload = serde_json::to_vec(&json!({ "inputs": inputs, "params": self.params })).unwrap();
        let mut cmd = Command::new(&self.command[0]);
        cmd.args(&self.command[1..])
            .current_dir(&self.cwd)
            .env_clear()
            .env(
                "PATH",
                std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
            )
            .env("LANG", "C.UTF-8")
            .env("HOME", &self.cwd)
            .env("TMPDIR", std::env::temp_dir())
            .env("PYTHONHASHSEED", "0");
        for k in &self.pass_env {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
        cmd.envs(&self.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        apply_limits(&mut cmd, &self.limits);
        let mut child = cmd
            .spawn()
            .map_err(|e| err(&self.name, format!("failed to start `{}`: {e}", self.command[0])))?;
        let pid = child.id();
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&payload);
        });
        let cap = self.limits.max_output_mb.saturating_mul(1024 * 1024);
        let overflow = Arc::new(AtomicBool::new(false));
        let flag = overflow.clone();
        let out = std::thread::spawn(move || {
            let mut b = vec![];
            let _ = stdout.take(cap + 1).read_to_end(&mut b);
            if b.len() as u64 > cap {
                flag.store(true, Ordering::SeqCst);
            }
            b
        });
        let errs = std::thread::spawn(move || {
            let mut b = vec![];
            let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut b);
            b
        });
        let deadline = Instant::now() + self.timeout;
        let finish = |child: &mut std::process::Child| {
            kill_group(pid);
            let _ = child.kill();
            let _ = child.wait();
        };
        let status = loop {
            if overflow.load(Ordering::SeqCst) {
                finish(&mut child);
                return Err(err(
                    &self.name,
                    format!("output larger than {} MB", self.limits.max_output_mb),
                ));
            }
            match child.try_wait().map_err(|e| err(&self.name, e.to_string()))? {
                Some(s) => break s,
                None if Instant::now() >= deadline => {
                    finish(&mut child);
                    return Err(err(
                        &self.name,
                        format!("timeout exceeded ({} s)", self.timeout.as_secs_f64()),
                    ));
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        };
        // Any descendant processes still running are killed (which releases the pipes).
        kill_group(pid);
        let stdout = out.join().unwrap_or_default();
        if overflow.load(Ordering::SeqCst) {
            return Err(err(
                &self.name,
                format!("output larger than {} MB", self.limits.max_output_mb),
            ));
        }
        if !status.success() {
            let msg = String::from_utf8_lossy(&errs.join().unwrap_or_default())
                .trim()
                .to_string();
            return Err(err(&self.name, format!("code {status}: {msg}")));
        }
        serde_json::from_slice(&stdout).map_err(|e| err(&self.name, format!("invalid JSON output: {e}")))
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
            env: Default::default(),
            pass_env: vec![],
            limits: Limits::default(),
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mf-tables-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    fn table(path: PathBuf, cols: &[&str], format: TableFormat) -> TableStep {
        TableStep {
            name: "t".into(),
            version: "1".into(),
            path,
            columns: cols.iter().map(|c| c.to_string()).collect(),
            table: true,
            format,
        }
    }

    #[test]
    fn csv_handles_quotes_and_delimiters() {
        let p = tmp("q.csv");
        std::fs::write(
            &p,
            "\"desk; name\";pd;default\n\"Equity; EU\";0.02;1\n \"Rates\" ; 0.01 ;0\n",
        )
        .unwrap();
        let out = table(p.clone(), &["pd", "default"], TableFormat::Csv { delimiter: b';' })
            .run(&BTreeMap::new())
            .unwrap();
        assert_eq!(out["pd"], json!([0.02, 0.01]));
        std::fs::write(&p, "a,b\n1,x\n").unwrap();
        let e = table(p, &["b"], TableFormat::Csv { delimiter: b',' })
            .run(&BTreeMap::new())
            .unwrap_err();
        assert!(e.to_string().contains("line 2"));
    }

    #[test]
    fn parquet_roundtrip() {
        use parquet::data_type::{DoubleType, Int64Type};
        use parquet::file::writer::SerializedFileWriter;
        use parquet::schema::parser::parse_message_type;
        use std::sync::Arc;
        let p = tmp("r.parquet");
        let schema = Arc::new(parse_message_type("message m { REQUIRED INT64 day; REQUIRED DOUBLE ret; }").unwrap());
        let mut w = SerializedFileWriter::new(std::fs::File::create(&p).unwrap(), schema, Default::default()).unwrap();
        let mut rg = w.next_row_group().unwrap();
        let mut c = rg.next_column().unwrap().unwrap();
        c.typed::<Int64Type>().write_batch(&[1, 2, 3], None, None).unwrap();
        c.close().unwrap();
        let mut c = rg.next_column().unwrap().unwrap();
        c.typed::<DoubleType>()
            .write_batch(&[0.01, -0.02, 0.005], None, None)
            .unwrap();
        c.close().unwrap();
        rg.close().unwrap();
        w.close().unwrap();
        let out = table(p.clone(), &["ret", "day"], TableFormat::Parquet)
            .run(&BTreeMap::new())
            .unwrap();
        assert_eq!(
            (out["ret"].clone(), out["day"].clone()),
            (json!([0.01, -0.02, 0.005]), json!([1.0, 2.0, 3.0]))
        );
        assert!(table(p, &["absent"], TableFormat::Parquet)
            .run(&BTreeMap::new())
            .is_err());
    }

    #[test]
    fn builtin_code_hash_tracks_engine() {
        let b = BuiltinStep {
            name: "b".into(),
            version: "1".into(),
            deps: vec![],
            kind: "psi".into(),
            params: Value::Null,
        };
        assert_eq!(b.code_hash(), engine_hash("psi"));
        assert_ne!(engine_hash("psi"), engine_hash("pd_calibration"));
    }

    #[test]
    fn environment_is_cleared() {
        std::env::set_var("MF_TEST_SECRET", "leak");
        let mut p = sh(
            r#"printf '{"secret":"%s","explicit":"%s"}' "$MF_TEST_SECRET" "$MF_EXPLICIT""#,
            5000,
        );
        p.env.insert("MF_EXPLICIT".into(), "ok".into());
        let out = p.run(&BTreeMap::new()).unwrap();
        assert_eq!(
            (out["secret"].as_str(), out["explicit"].as_str()),
            (Some(""), Some("ok"))
        );
        p.pass_env = vec!["MF_TEST_SECRET".into()];
        assert_eq!(p.run(&BTreeMap::new()).unwrap()["secret"], "leak");
    }

    #[test]
    fn output_cap_is_enforced() {
        let mut p = sh("yes", 10_000);
        p.limits.max_output_mb = 1;
        let t = Instant::now();
        assert!(p
            .run(&BTreeMap::new())
            .unwrap_err()
            .to_string()
            .contains("output larger"));
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn orphan_processes_do_not_block() {
        let t = Instant::now();
        let out = sh(r#"sleep 30 & echo '{"ok":1}'"#, 10_000)
            .run(&BTreeMap::new())
            .unwrap();
        assert_eq!(out["ok"], 1);
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn memory_limit_is_enforced() {
        let mut p = sh(r#"python3 -c "x = bytearray(800 * 1024 * 1024); print('{}')""#, 20_000);
        p.limits.memory_mb = Some(200);
        assert!(p.run(&BTreeMap::new()).is_err());
        p.limits.memory_mb = None;
        assert!(p.run(&BTreeMap::new()).is_ok());
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
        assert!(e.contains("timeout") && t.elapsed() < Duration::from_secs(3));
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
