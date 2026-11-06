use mf_core::{hash_bytes, Error, Inputs, Step};
use quantparity::benchmark::{compare, prices_from_value, reference_prices, trades_from_value, Tolerances};
use quantparity::curves::Market;
use quantparity::ifrs9::{EclInput, MigrationMatrix, PdCurve, Repayment};
use quantparity::validation::Z2Null;
use quantparity::validation::{
    calibration, discrimination, es_backtest, historical_var, parameter_backtest, psi, realized_ccf, staging_check,
    traffic_light, var_backtest, StagingRecord, StagingRules,
};
use quantparity::var_models::{var_forecasts_with, Innovations, VarForecasts, Volatility};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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

/// Regulatory backtesting window (Basel: the last 250 business days).
const BASEL_WINDOW: usize = 250;

/// `20200316` -> `"2020-03-16"`; any other value is kept as is.
fn date_label(d: f64) -> Value {
    let n = d as u64;
    let (y, m, day) = (n / 10_000, n / 100 % 100, n % 100);
    if d.fract() == 0.0 && (1000..=9999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&day) {
        json!(format!("{y:04}-{m:02}-{day:02}"))
    } else {
        json!(d)
    }
}

/// Details a validator needs to investigate a VaR backtest, added to the summary statistics:
/// every exception (dated when `dates` is given), the Basel traffic light on consecutive
/// 250-day windows ending at the latest observation, and exceptions per calendar year.
/// `offset` is the index of `realized[0]` in the original return series (and in `dates`).
fn backtest_details(realized: &[f64], var: &[f64], alpha: f64, offset: usize, dates: Option<&[f64]>) -> Value {
    let date = |i: usize| dates.and_then(|d| d.get(offset + i)).map(|d| date_label(*d));
    let hits: Vec<bool> = realized.iter().zip(var).map(|(r, v)| *r < -*v).collect();
    let exceptions: Vec<Value> = (0..hits.len())
        .filter(|&i| hits[i])
        .map(|i| {
            let mut e = json!({"index": offset + i, "return": realized[i], "var": var[i], "excess_loss": -realized[i] - var[i]});
            if let Some(d) = date(i) {
                e["date"] = d;
            }
            e
        })
        .collect();
    // Consecutive full windows, the last one ending at the latest observation.
    let windows: Vec<Value> = (0..hits.len() / BASEL_WINDOW)
        .rev()
        .map(|k| hits.len() - (k + 1) * BASEL_WINDOW)
        .map(|start| {
            let n = hits[start..start + BASEL_WINDOW].iter().filter(|h| **h).count();
            let t = traffic_light(BASEL_WINDOW, n, alpha);
            let mut w = json!({"exceptions": n, "zone": t.zone, "plus_factor": t.plus_factor, "status": t.status});
            if let (Some(a), Some(b)) = (date(start), date(start + BASEL_WINDOW - 1)) {
                w["from"] = a;
                w["to"] = b;
            } else {
                w["from"] = json!(offset + start);
                w["to"] = json!(offset + start + BASEL_WINDOW - 1);
            }
            w
        })
        .collect();
    let mut out = json!({"exception_list": exceptions, "basel_windows": windows});
    if let Some(last) = windows.last() {
        out["latest_basel_window"] = last.clone();
    }
    let years: Option<Vec<u64>> = (0..hits.len())
        .map(|i| date(i).and_then(|d| d.as_str().and_then(|s| s[..4].parse().ok())))
        .collect();
    if let Some(years) = years {
        let mut by_year: BTreeMap<u64, (usize, usize)> = BTreeMap::new();
        for (y, h) in years.iter().zip(&hits) {
            let e = by_year.entry(*y).or_default();
            e.0 += 1;
            e.1 += usize::from(*h);
        }
        out["by_year"] = json!(by_year
            .into_iter()
            .map(|(y, (n, x))| json!({"year": y, "observations": n, "exceptions": x, "rate": x as f64 / n as f64}))
            .collect::<Vec<_>>());
    }
    out
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
        "var_model",
        "es_backtest",
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
        "ecl",
        "lgd_backtest",
        "ccf_backtest",
        "staging_check",
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

    /// Merges [`backtest_details`] into a backtest result; dates come from the optional `dates`
    /// dependency (a series, or column `date_column` of a table, aligned with the returns).
    fn add_details(
        &self,
        out: &mut Value,
        realized: &[f64],
        var: &[f64],
        alpha: f64,
        offset: usize,
        inputs: &Inputs,
    ) -> Result<(), Error> {
        let dates = self.dates(inputs)?;
        if let (Value::Object(o), Value::Object(d)) =
            (out, backtest_details(realized, var, alpha, offset, dates.as_deref()))
        {
            o.extend(d);
        }
        Ok(())
    }

    /// Forecasts from the `from` parameter (index of the first return evaluated) when it is after
    /// their start: models with different warm-ups are then backtested on the same days.
    fn evaluated_from(&self, start: usize, f: &[f64]) -> Result<(usize, Vec<f64>), Error> {
        let from = match self.params.get("from") {
            None => return Ok((start, f.to_vec())),
            Some(v) => v
                .as_u64()
                .ok_or_else(|| err(&self.name, "`from`: index of the first return evaluated"))?
                as usize,
        };
        if from < start {
            return Err(err(
                &self.name,
                format!("`from` = {from} is before the first forecast ({start})"),
            ));
        }
        if from >= start + f.len() {
            return Err(err(&self.name, format!("`from` = {from} is after the last forecast")));
        }
        Ok((from, f[from - start..].to_vec()))
    }

    /// Optional `dates` dependency (a series, or column `date_column` of a table).
    fn dates(&self, inputs: &Inputs) -> Result<Option<Vec<f64>>, Error> {
        match self.params.get("dates").and_then(Value::as_str) {
            Some(_) => Ok(Some(series(
                self.dep(inputs, "dates", usize::MAX)?,
                self.params.get("date_column").and_then(Value::as_str),
                &self.name,
            )?)),
            None => Ok(None),
        }
    }

    fn json<T: serde::Serialize>(&self, v: Option<T>, why: &str) -> Result<Value, Error> {
        v.map(|x| serde_json::to_value(x).unwrap())
            .ok_or_else(|| err(&self.name, why))
    }

    /// Column named by parameter `key`, otherwise column `default` if the table has one.
    fn opt_col(&self, t: &Value, key: &str, default: &str) -> Result<Option<Vec<f64>>, Error> {
        match self.params.get(key).and_then(Value::as_str) {
            Some(c) => series(t, Some(c), &self.name).map(Some),
            None if t.get(default).is_some() => series(t, Some(default), &self.name).map(Some),
            None => Ok(None),
        }
    }
}

// ---------- IFRS 9 ----------
/// Longest remaining life (and migration-matrix horizon) accepted by the `ecl` step, in years.
const MAX_ECL_YEARS: f64 = 100.0;

impl BuiltinStep {
    /// PD curve of each facility: `pd_curves` input (curves by segment, or a migration matrix
    /// whose state is the facility's 1-based `pd_segment`), otherwise a flat 12-month PD column.
    fn pd_curves(&self, inputs: &Inputs, t: &Value, n: usize) -> Result<Vec<PdCurve>, Error> {
        let e = |m: String| err(&self.name, m);
        if self.params.get("pd_curves").is_none() {
            let pd = self.table_col(t, "pd_12m", "pd_12m")?;
            return pd.iter().map(|&p| PdCurve::flat(p).map_err(e)).collect();
        }
        let doc = self.dep(inputs, "pd_curves", usize::MAX)?;
        let seg = self.table_col(t, "pd_segment", "grade")?;
        if seg.len() != n {
            return Err(e("`pd_segment` and the facilities differ in length".into()));
        }
        let mut cache: BTreeMap<i64, PdCurve> = BTreeMap::new();
        if let Some(curves) = doc.get("curves").and_then(Value::as_object) {
            for (k, v) in curves {
                let c: Vec<f64> = serde_json::from_value(v.clone()).map_err(|x| e(format!("curve `{k}`: {x}")))?;
                let k = k
                    .parse::<f64>()
                    .map_err(|_| e(format!("curve `{k}`: the key must be a number")))?;
                cache.insert(k.round() as i64, PdCurve::new(c).map_err(e)?);
            }
        } else if let (Some(states), Some(m)) = (doc.get("states"), doc.get("matrix")) {
            let states: Vec<String> = serde_json::from_value(states.clone()).map_err(|x| e(x.to_string()))?;
            let m = MigrationMatrix::new(states, serde_json::from_value(m.clone()).map_err(|x| e(x.to_string()))?)
                .map_err(e)?;
            let years = param_f64(&self.params, "years", 30.0);
            if !(1.0..=MAX_ECL_YEARS).contains(&years) {
                return Err(e(format!("`years` must be between 1 and {MAX_ECL_YEARS}")));
            }
            let years = years as usize;
            for s in 1..m.states.len() {
                cache.insert(s as i64, m.pd_curve(s - 1, years).map_err(e)?);
            }
        } else {
            return Err(e("`pd_curves` must have `curves` or `states` and `matrix`".into()));
        }
        seg.iter()
            .map(|s| {
                cache
                    .get(&(s.round() as i64))
                    .cloned()
                    .ok_or_else(|| e(format!("no PD curve for segment {s}")))
            })
            .collect()
    }

    fn ecl(&self, inputs: &Inputs) -> Result<Value, Error> {
        let e = |m: String| err(&self.name, m);
        let t = self.dep(inputs, "table", 0)?;
        let stage = self.table_col(t, "stage", "stage")?;
        let n = stage.len();
        let col = |k: &str| -> Result<Vec<f64>, Error> {
            let v = self.table_col(t, k, k)?;
            (v.len() == n)
                .then_some(v)
                .ok_or_else(|| e(format!("column `{k}`: {} rows for {n}", stage.len())))
        };
        let (balance, lgd, eir, periods) = (col("balance")?, col("lgd")?, col("eir")?, col("periods")?);
        let opt = |k: &str| -> Result<Option<Vec<f64>>, Error> {
            match self.opt_col(t, k, k)? {
                Some(v) if v.len() != n => Err(e(format!("column `{k}`: {} rows for {n}", v.len()))),
                v => Ok(v),
            }
        };
        let (reported, real_12m, real_life) = (opt("reported")?, opt("realized_12m")?, opt("realized_lifetime")?);
        let ppy = param_f64(&self.params, "periods_per_year", 12.0);
        if !(1.0..=366.0).contains(&ppy) || ppy.fract() != 0.0 {
            return Err(e("`periods_per_year` must be a whole number between 1 and 366".into()));
        }
        let ppy = ppy as u32;
        let repayment = Repayment::parse(param_str(&self.params, "repayment", "annuity")).map_err(e)?;
        let discount = self.params.get("discount").and_then(Value::as_bool).unwrap_or(true);
        let curves = self.pd_curves(inputs, t, n)?;
        let mut rows = Vec::with_capacity(n);
        for i in 0..n {
            let s = stage[i] as u8;
            // Bounded so that a malformed table cannot make the step allocate without limit.
            let max_periods = MAX_ECL_YEARS * ppy as f64;
            if !(0.0..=max_periods).contains(&periods[i]) {
                return Err(e(format!(
                    "facility {i}: {} periods, outside 0 to {max_periods} ({MAX_ECL_YEARS} years)",
                    periods[i]
                )));
            }
            let k = if s == 3 { 1 } else { periods[i].round() as usize };
            let input = EclInput {
                stage: s,
                cumulative_pd: curves[i].cumulative_periods(k, ppy),
                lgd: vec![lgd[i]],
                ead: repayment.profile(balance[i], k, eir[i], ppy),
                eir: if discount { eir[i] } else { 0.0 },
            };
            rows.push(quantparity::ifrs9::ecl(&input, ppy).map_err(|m| e(format!("facility {i}: {m}")))?);
        }
        // Totals by group: facilities, exposure, ECL (12 months, lifetime, stage), reported, realized.
        #[derive(Default, Clone)]
        struct Sum {
            n: usize,
            exposure: f64,
            ecl_12m: f64,
            ecl_lifetime: f64,
            ecl: f64,
            reported: f64,
            real_12m: f64,
            real_life: f64,
        }
        let add = |g: &mut Sum, i: usize| {
            g.n += 1;
            g.exposure += balance[i];
            g.ecl_12m += rows[i].ecl_12m;
            g.ecl_lifetime += rows[i].ecl_lifetime;
            g.ecl += rows[i].ecl;
            g.reported += reported.as_ref().map_or(0.0, |r| r[i]);
            g.real_12m += real_12m.as_ref().map_or(0.0, |r| r[i]);
            g.real_life += real_life.as_ref().map_or(0.0, |r| r[i]);
        };
        let show = |g: &Sum| {
            let mut o = json!({"facilities": g.n, "exposure": g.exposure, "ecl": g.ecl, "ecl_12m": g.ecl_12m,
                               "ecl_lifetime": g.ecl_lifetime, "coverage": g.ecl / g.exposure});
            if reported.is_some() {
                o["reported"] = json!(g.reported);
                o["difference"] = json!(g.ecl - g.reported);
            }
            if real_12m.is_some() {
                o["realized_12m"] = json!(g.real_12m);
                o["realized_to_ecl_12m"] = json!(g.real_12m / g.ecl_12m);
            }
            if real_life.is_some() {
                o["realized_lifetime"] = json!(g.real_life);
                o["realized_to_ecl_lifetime"] = json!(g.real_life / g.ecl_lifetime);
            }
            o
        };
        let mut total = Sum::default();
        let mut by_stage: BTreeMap<u8, Sum> = BTreeMap::new();
        for (i, r) in rows.iter().enumerate() {
            add(&mut total, i);
            add(by_stage.entry(r.stage).or_default(), i);
        }
        let mut out = json!({
            "facilities": n, "periods_per_year": ppy, "repayment": repayment, "discount": discount,
            "total": show(&total),
            "by_stage": by_stage.iter().map(|(s, g)| { let mut o = show(g); o["stage"] = json!(s); o }).collect::<Vec<_>>(),
        });
        let segments: Vec<String> = match self.params.get("segments") {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => vec![],
        };
        for name in segments {
            let seg = series(t, Some(&name), &self.name)?;
            let mut groups: BTreeMap<i64, Sum> = BTreeMap::new();
            for (i, v) in seg.iter().enumerate().take(n) {
                add(groups.entry(v.round() as i64).or_default(), i);
            }
            out["by"][&name] = json!(groups
                .iter()
                .map(|(k, g)| {
                    let mut o = show(g);
                    o["segment"] = json!(k);
                    o
                })
                .collect::<Vec<_>>());
        }
        let mut status = "pass";
        if let Some(rep) = &reported {
            let rel_tol = param_f64(&self.params, "tolerance", 0.01);
            let abs_tol = param_f64(&self.params, "abs_tolerance", 1.0);
            let mut diffs: Vec<(usize, f64)> = (0..n).map(|i| (i, rows[i].ecl - rep[i])).collect();
            let outside = diffs
                .iter()
                .filter(|(i, d)| d.abs() > abs_tol.max(rel_tol * rep[*i].abs()))
                .count();
            diffs.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
            let rel = (total.ecl - total.reported) / total.reported.abs();
            status = if rel.abs() > rel_tol || !rel.is_finite() {
                "fail"
            } else if outside > 0 {
                "warn"
            } else {
                "pass"
            };
            out["reconciliation"] = json!({
                "reported": total.reported, "recomputed": total.ecl, "difference": total.ecl - total.reported,
                "relative_difference": rel, "tolerance": rel_tol, "abs_tolerance": abs_tol,
                "facilities_outside_tolerance": outside,
                "largest": diffs.iter().take(20).map(|&(i, d)| json!({"index": i, "reported": rep[i],
                    "recomputed": rows[i].ecl, "difference": d, "stage": rows[i].stage})).collect::<Vec<_>>(),
                "status": status,
            });
        }
        out["status"] = json!(status);
        Ok(out)
    }

    fn staging(&self, inputs: &Inputs) -> Result<Value, Error> {
        let t = self.dep(inputs, "table", 0)?;
        let stage = self.table_col(t, "stage", "stage")?;
        let n = stage.len();
        let get = |k: &str| -> Result<Vec<f64>, Error> { Ok(self.opt_col(t, k, k)?.unwrap_or_else(|| vec![0.0; n])) };
        let (defaulted, dpd, flag) = (get("defaulted")?, get("dpd")?, get("flag")?);
        let (pd0, pd1) = (
            self.table_col(t, "pd_origination", "pd_origination")?,
            self.table_col(t, "pd_current", "pd_current")?,
        );
        let next = self.opt_col(t, "defaulted_next_12m", "defaulted_next_12m")?;
        if [defaulted.len(), dpd.len(), flag.len(), pd0.len(), pd1.len()]
            .iter()
            .any(|&l| l != n)
            || next.as_ref().is_some_and(|v| v.len() != n)
        {
            return Err(err(&self.name, "columns of different lengths"));
        }
        let records: Vec<StagingRecord> = (0..n)
            .map(|i| StagingRecord {
                reported_stage: stage[i].round().clamp(0.0, 255.0) as u8,
                defaulted: defaulted[i] != 0.0,
                dpd: dpd[i],
                pd_origination: pd0[i],
                pd_current: pd1[i],
                flag: flag[i] != 0.0,
                defaulted_next_12m: next.as_ref().map(|v| v[i] != 0.0),
            })
            .collect();
        let d = StagingRules::default();
        let p = |k: &str, x: f64| param_f64(&self.params, k, x);
        let rules = StagingRules {
            dpd_stage2: p("dpd_stage2", d.dpd_stage2),
            dpd_stage3: p("dpd_stage3", d.dpd_stage3),
            relative_threshold: p("relative_threshold", d.relative_threshold),
            absolute_threshold: p("absolute_threshold", d.absolute_threshold),
            low_credit_risk_pd: self.params.get("low_credit_risk_pd").and_then(Value::as_f64),
            tolerance: p("tolerance", d.tolerance),
        };
        let c = staging_check(&records, &rules).map_err(|m| err(&self.name, m))?;
        Ok(serde_json::to_value(c).unwrap())
    }
}

impl Step for BuiltinStep {
    /// The engine version, the step kind and its parameters: a parameter change is a model change.
    fn code_hash(&self) -> Option<String> {
        let params = serde_json::to_string(&self.params).unwrap();
        engine_hash(&format!("{} {params}", self.kind))
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
                if w == 0 || r.len() <= w {
                    return Err(err(&self.name, "series too short"));
                }
                // Same estimator as quantparity's rolling_backtest, kept to report each forecast.
                let var: Vec<f64> = (w..r.len())
                    .map(|t| historical_var(&r[t - w..t], alpha).ok_or_else(|| err(&self.name, "invalid series")))
                    .collect::<Result<_, _>>()?;
                let mut out = self.json(var_backtest(&r[w..], &var, alpha), "series too short")?;
                self.add_details(&mut out, &r[w..], &var, alpha, w, inputs)?;
                Ok(out)
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
                let (start, var) = self.evaluated_from(start, &var)?;
                let realized = &r[start..start + var.len()];
                let mut out = self.json(var_backtest(realized, &var, alpha), "backtest not possible")?;
                self.add_details(&mut out, realized, &var, alpha, start, inputs)?;
                Ok(out)
            }
            // VaR / ES forecasts: EWMA, GARCH or GJR volatility; normal, Student-t or filtered
            // historical simulation innovations.
            "var_model" => {
                let r = series(self.dep(inputs, "returns", 0)?, self.column(), &self.name)?;
                let p = &self.params;
                let innovations = match param_str(p, "distribution", "normal") {
                    "normal" => Innovations::Normal,
                    "t" | "student_t" => Innovations::StudentT {
                        nu: p.get("nu").and_then(Value::as_f64).ok_or_else(|| {
                            err(
                                &self.name,
                                "`nu` (degrees of freedom) is required for `distribution = \"t\"`",
                            )
                        })?,
                    },
                    "fhs" => Innovations::Filtered {
                        window: p.get("window").and_then(Value::as_u64).map(|w| w as usize),
                        min_residuals: param_f64(p, "min_residuals", 250.0) as usize,
                    },
                    d => return Err(err(&self.name, format!("unknown distribution `{d}` (normal, t, fhs)"))),
                };
                let (volatility, warmup) = match param_str(p, "volatility", "ewma") {
                    "ewma" => (
                        Volatility::Ewma {
                            lambda: param_f64(p, "lambda", 0.94),
                        },
                        30.0,
                    ),
                    v @ ("garch" | "gjr") => (
                        Volatility::Garch {
                            asymmetric: v == "gjr",
                            refit: param_f64(p, "refit", 250.0) as usize,
                            window: p.get("fit_window").and_then(Value::as_u64).map(|w| w as usize),
                        },
                        500.0,
                    ),
                    v => return Err(err(&self.name, format!("unknown volatility `{v}` (ewma, garch, gjr)"))),
                };
                let f = var_forecasts_with(
                    &r,
                    volatility,
                    param_f64(p, "warmup", warmup) as usize,
                    alpha,
                    param_f64(p, "es_alpha", 0.975),
                    innovations,
                )
                .map_err(|e| err(&self.name, e))?;
                let mut out = serde_json::to_value(&f).unwrap();
                // A GARCH fit at the stationarity bound has no long-run variance: warn.
                if let Some(fits) = &f.fits {
                    let max = param_f64(p, "max_persistence", 0.999);
                    let dates = self.dates(inputs)?;
                    let at_bound: Vec<Value> = fits
                        .iter()
                        .filter(|x| x.fit.persistence >= max)
                        .map(|x| {
                            let mut e = json!({"from": x.from, "persistence": x.fit.persistence});
                            if let Some(d) = dates.as_ref().and_then(|d| d.get(x.from)) {
                                e["date"] = json!(date_label(*d));
                            }
                            e
                        })
                        .collect();
                    let status = if at_bound.is_empty() { "pass" } else { "warn" };
                    out["persistence_check"] = json!({
                        "max_persistence": max,
                        "highest": fits.iter().map(|x| x.fit.persistence).fold(f64::NEG_INFINITY, f64::max),
                        "fits": fits.len(),
                        "at_bound": at_bound,
                        "status": status,
                    });
                    out["status"] = json!(status);
                }
                Ok(out)
            }
            // Acerbi-Székely ES backtest of forecasts {"start", "es", "es_var"} (`var_model`, or an
            // external model), over the whole sample and on consecutive 250-day windows.
            "es_backtest" => {
                let r = series(self.dep(inputs, "returns", 0)?, self.column(), &self.name)?;
                let f = self.dep(inputs, "forecasts", 1)?;
                let start = f.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
                let field = |k: &str| {
                    series(f.get(k).unwrap_or(&Value::Null), None, &self.name).map_err(|_| {
                        err(
                            &self.name,
                            format!("forecasts: `{k}` is missing (ES and VaR at the ES level)"),
                        )
                    })
                };
                let (es, es_var) = (field("es")?, field("es_var")?);
                if es.is_empty() || es.len() != es_var.len() || start + es.len() > r.len() {
                    return Err(err(
                        &self.name,
                        "forecasts are empty, of different lengths or longer than the series",
                    ));
                }
                let (_, es_var) = self.evaluated_from(start, &es_var)?;
                let (start, es) = self.evaluated_from(start, &es)?;
                let conf = self
                    .params
                    .get("alpha")
                    .and_then(Value::as_f64)
                    .or_else(|| f.get("es_confidence").and_then(Value::as_f64))
                    .unwrap_or(0.975);
                let realized = &r[start..start + es.len()];
                let mut out = self.json(es_backtest(realized, &es_var, &es, conf), "invalid forecasts")?;
                // Z2 p-values by simulation from the forecast distribution of a `var_model` step.
                // `simulations` absent: automatic for `var_model` forecasts (10,000 samples, 100,000
                // when p is close to a zone threshold: below 0.1%, or between 4% and 6%); 0: off; n:
                // exactly n samples.
                let requested = match self.params.get("simulations") {
                    None => None,
                    Some(v) => Some(
                        v.as_u64()
                            .ok_or_else(|| err(&self.name, "`simulations`: a number of samples"))?
                            as usize,
                    ),
                };
                let seed = param_f64(&self.params, "seed", 1.0) as u64;
                let model = serde_json::from_value::<VarForecasts>(f.clone()).ok();
                let null = match (requested, model) {
                    (Some(0), _) | (None, None) => None,
                    (Some(_), None) => {
                        return Err(err(
                            &self.name,
                            "`simulations` needs the forecasts of a `var_model` step: their model is simulated",
                        ))
                    }
                    (_, Some(m)) if m.es_confidence != conf => {
                        if requested.is_some() {
                            return Err(err(
                                &self.name,
                                "`simulations`: `alpha` must be the forecasts' ES level",
                            ));
                        }
                        None
                    }
                    (_, Some(m)) => Some(Z2Null::new(&r, &m).map_err(|e| err(&self.name, e))?),
                };
                let simulate = |z2: f64, a: usize, b: usize| -> Result<Option<Value>, Error> {
                    let Some(null) = &null else { return Ok(None) };
                    let run = |k: usize| {
                        null.test(z2, start + a, start + b, k, seed)
                            .map_err(|e| err(&self.name, e))
                    };
                    let s = match requested {
                        Some(k) => run(k)?,
                        None => match run(10_000)? {
                            s if s.p_value < 1e-3 || (0.04..=0.06).contains(&s.p_value) => run(100_000)?,
                            s => s,
                        },
                    };
                    Ok(Some(serde_json::to_value(s).unwrap()))
                };
                let n = realized.len();
                let simulated = simulate(out["z2"].as_f64().unwrap_or(f64::NAN), 0, n)?;
                let dates = self.dates(inputs)?;
                let date = |i: usize| dates.as_ref().and_then(|d| d.get(start + i)).map(|d| date_label(*d));
                let windows = (0..n / BASEL_WINDOW)
                    .rev()
                    .map(|k| n - (k + 1) * BASEL_WINDOW)
                    .map(|a| {
                        let b = a + BASEL_WINDOW;
                        let t = es_backtest(&realized[a..b], &es_var[a..b], &es[a..b], conf).unwrap();
                        let mut w = json!({"exceptions": t.exceptions, "z2": t.z2, "z1": t.z1, "zone": t.zone,
                                           "status": t.status});
                        if let Some(s) = simulate(t.z2, a, b)? {
                            w["p_value"] = s["p_value"].clone();
                            w["simulated_zone"] = s["zone"].clone();
                            w["status"] = s["status"].clone();
                        }
                        match (date(a), date(b - 1)) {
                            (Some(x), Some(y)) => {
                                w["from"] = x;
                                w["to"] = y;
                            }
                            _ => {
                                w["from"] = json!(start + a);
                                w["to"] = json!(start + b - 1);
                            }
                        }
                        Ok(w)
                    })
                    .collect::<Result<Vec<Value>, Error>>()?;
                // Whole-sample verdict: the simulated one; without simulation, the fixed thresholds
                // hold for 250 days only, so a longer sample takes the zone of its latest window.
                let basis = match (simulated, windows.last()) {
                    (Some(s), _) => {
                        out["zone"] = s["zone"].clone();
                        out["status"] = s["status"].clone();
                        out["simulated"] = s;
                        "simulation"
                    }
                    (None, Some(last)) if n > BASEL_WINDOW => {
                        out["zone"] = last["zone"].clone();
                        out["status"] = last["status"].clone();
                        "latest 250-day window"
                    }
                    _ => "fixed thresholds",
                };
                out["zone_basis"] = json!(basis);
                if let Some(last) = windows.last() {
                    out["latest_window"] = last.clone();
                }
                out["windows"] = json!(windows);
                Ok(out)
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
            "ecl" => self.ecl(inputs),
            "lgd_backtest" => {
                let t = self.dep(inputs, "table", 0)?;
                let est = self.table_col(t, "estimated", "lgd_estimate")?;
                let real = self.table_col(t, "realized", "lgd_realized")?;
                let seg = self.opt_col(t, "segment", "segment")?;
                let min_c = param_f64(&self.params, "min_concordance", 0.6);
                self.json(
                    parameter_backtest("lgd", &est, &real, seg.as_deref(), min_c),
                    "invalid data (equal-length finite columns are required)",
                )
            }
            "ccf_backtest" => {
                let t = self.dep(inputs, "table", 0)?;
                let limit = self.table_col(t, "limit", "limit")?;
                let drawn = self.table_col(t, "drawn", "drawn")?;
                let ead = self.table_col(t, "ead", "ead")?;
                let est = self.table_col(t, "estimated", "ccf_estimate")?;
                let seg = self.opt_col(t, "segment", "segment")?;
                let r = realized_ccf(&limit, &drawn, &ead, param_f64(&self.params, "min_undrawn", 0.0))
                    .filter(|_| est.len() == limit.len())
                    .ok_or_else(|| err(&self.name, "invalid data (equal-length finite columns are required)"))?;
                let pick = |v: &[f64]| r.kept.iter().map(|&i| v[i]).collect::<Vec<f64>>();
                let seg = seg.map(|s| pick(&s));
                let min_c = param_f64(&self.params, "min_concordance", 0.6);
                let b = parameter_backtest("ccf", &pick(&est), &r.ccf, seg.as_deref(), min_c)
                    .ok_or_else(|| err(&self.name, "no facility with an undrawn amount above `min_undrawn`"))?;
                let mut out = serde_json::to_value(b).unwrap();
                out["excluded"] = json!(r.excluded);
                out["outside_unit_interval"] = json!(r.outside_unit_interval);
                Ok(out)
            }
            "staging_check" => self.staging(inputs),
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
    /// As-of date of a dated pipeline, sent to the process with its inputs.
    pub as_of: Option<String>,
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
        // Parameters are part of the model.
        buf.extend(serde_json::to_vec(&self.params).unwrap());
        Some(hash_bytes(&buf))
    }

    fn run(&self, inputs: &Inputs) -> Result<Value, Error> {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let mut payload = json!({ "inputs": inputs, "params": self.params });
        if let Some(d) = &self.as_of {
            payload["as_of"] = json!(d);
        }
        let payload = serde_json::to_vec(&payload).unwrap();
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
            as_of: None,
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
    fn builtin_code_hash_tracks_engine_and_params() {
        let mut b = BuiltinStep {
            name: "b".into(),
            version: "1".into(),
            deps: vec![],
            kind: "psi".into(),
            params: Value::Null,
        };
        assert_eq!(b.code_hash(), engine_hash("psi null"));
        assert_ne!(engine_hash("psi null"), engine_hash("pd_calibration null"));
        let before = b.code_hash();
        b.params = json!({"bins": 10});
        assert_ne!(b.code_hash(), before);
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
    fn var_model_feeds_var_and_es_backtests() {
        let r: Vec<f64> = (0..900)
            .map(|k| {
                if k % 37 == 0 {
                    -0.05
                } else {
                    0.004 * ((k * 7919 % 13) as f64 - 6.0) / 6.0
                }
            })
            .collect();
        // 28 days a month, 12 months a year, from 2020-01-01.
        let days: Vec<f64> = (0..900)
            .map(|k| ((2020 + k / 336) * 10_000 + (k % 336 / 28 + 1) * 100 + k % 28 + 1) as f64)
            .collect();
        let mut i = BTreeMap::new();
        i.insert("r".to_string(), json!(r));
        i.insert("d".to_string(), json!(days));
        for (params, model) in [
            (json!({}), "ewma-normal"),
            (json!({"distribution": "t", "nu": 4}), "ewma-t"),
            (json!({"distribution": "fhs", "min_residuals": 250}), "fhs"),
        ] {
            let f = builtin("var_model", &["r"], params).run(&i).unwrap();
            assert_eq!(f["model"], model);
            i.insert("f".to_string(), f);
            let var = builtin("backtest_forecasts", &["r", "f"], json!({})).run(&i).unwrap();
            assert!(var["traffic_light"]["zone"].is_string());
            let es = builtin("es_backtest", &["r", "f", "d"], json!({"dates": "d", "simulations": 0}))
                .run(&i)
                .unwrap();
            assert_eq!(es["confidence"], 0.975);
            let w = es["windows"].as_array().unwrap();
            assert_eq!(es["latest_window"], *w.last().unwrap());
            assert_eq!(es["latest_window"]["to"], "2022-09-04");
            assert!(es["z2"].as_f64().unwrap() <= 1.0);
        }
        // GJR volatility, refitted every 100 days on the latest 300 returns.
        let f = builtin(
            "var_model",
            &["r"],
            json!({"volatility": "gjr", "warmup": 300, "refit": 100, "fit_window": 300, "distribution": "fhs"}),
        )
        .run(&i)
        .unwrap();
        assert_eq!((f["model"].as_str(), f["start"].as_u64()), (Some("gjr-fhs"), Some(550)));
        assert_eq!(f["fits"].as_array().unwrap().len(), 6);
        assert_eq!(f["volatility"]["window"], 300);
        let check = &f["persistence_check"];
        assert_eq!(check["fits"], 6);
        let highest = check["highest"].as_f64().unwrap();
        // Every fit at or above the bound is listed, with its date; the status follows.
        let strict = builtin(
            "var_model",
            &["r", "d"],
            json!({"volatility": "gjr", "warmup": 300, "refit": 100, "fit_window": 300,
                   "max_persistence": highest, "dates": "d"}),
        )
        .run(&i)
        .unwrap();
        let bound = strict["persistence_check"]["at_bound"].as_array().unwrap();
        assert!(!bound.is_empty() && bound.iter().all(|b| b["date"].is_string()));
        assert_eq!(
            (
                strict["status"].as_str(),
                strict["persistence_check"]["status"].as_str()
            ),
            (Some("warn"), Some("warn"))
        );
        let loose = builtin(
            "var_model",
            &["r"],
            json!({"volatility": "garch", "warmup": 300, "max_persistence": 1.0}),
        )
        .run(&i)
        .unwrap();
        assert_eq!(loose["status"], "pass");
        // EWMA has no fit, hence no check and no status.
        let ewma = builtin("var_model", &["r"], json!({})).run(&i).unwrap();
        assert!(ewma.get("status").is_none() && ewma.get("persistence_check").is_none());
        let e = builtin("var_model", &["r"], json!({"volatility": "egarch"}))
            .run(&i)
            .unwrap_err();
        assert!(e.to_string().contains("ewma, garch, gjr"));
        // Evaluated from the same day as the filtered simulation (index 280).
        let f = builtin("var_model", &["r"], json!({})).run(&i).unwrap();
        i.insert("f".to_string(), f);
        let late = builtin("backtest_forecasts", &["r", "f"], json!({"from": 280}))
            .run(&i)
            .unwrap();
        assert_eq!(late["observations"], 620);
        let es = builtin("es_backtest", &["r", "f"], json!({"from": 280, "simulations": 0}))
            .run(&i)
            .unwrap();
        assert_eq!(es["observations"], 620);
        assert!(builtin("es_backtest", &["r", "f"], json!({"from": 10}))
            .run(&i)
            .is_err());
        let e = builtin("var_model", &["r"], json!({"distribution": "t"}))
            .run(&i)
            .unwrap_err();
        assert!(e.to_string().contains("nu"));
        // Z2 p-values by simulation: whole sample and windows; the status follows the simulated zone.
        for params in [
            json!({}),
            json!({"distribution": "t", "nu": 4}),
            json!({"distribution": "fhs"}),
        ] {
            let f = builtin("var_model", &["r"], params).run(&i).unwrap();
            i.insert("f".to_string(), f);
            let es = builtin("es_backtest", &["r", "f"], json!({"simulations": 2000, "seed": 7}))
                .run(&i)
                .unwrap();
            let s = &es["simulated"];
            let p = s["p_value"].as_f64().unwrap();
            assert!(p > 0.0 && p <= 1.0 && s["simulations"] == 2000 && s["seed"] == 7);
            assert_eq!(es["status"], s["status"]);
            let w = es["windows"].as_array().unwrap();
            assert!(w.iter().all(|x| x["p_value"].is_f64() && x["status"].is_string()));
            assert_eq!(
                es["latest_window"]["simulated_zone"],
                w.last().unwrap()["simulated_zone"]
            );
            let again = builtin("es_backtest", &["r", "f"], json!({"simulations": 2000, "seed": 7}))
                .run(&i)
                .unwrap();
            assert_eq!(again, es);
            assert!(
                builtin("es_backtest", &["r", "f"], json!({"simulations": 100, "alpha": 0.99}))
                    .run(&i)
                    .is_err()
            );
        }
        // By default, `var_model` forecasts are simulated (10,000 samples, 100,000 near the red zone).
        let f = builtin("var_model", &["r"], json!({})).run(&i).unwrap();
        i.insert("f".to_string(), f);
        let auto = builtin("es_backtest", &["r", "f"], json!({})).run(&i).unwrap();
        let k = auto["simulated"]["simulations"].as_u64().unwrap();
        let p = auto["simulated"]["p_value"].as_f64().unwrap();
        let near = p < 1e-3 || (0.04..=0.06).contains(&p);
        assert!((k == 10_000 && !near) || k == 100_000, "{k} {p}");
        assert_eq!(
            (auto["zone_basis"].as_str(), &auto["zone"]),
            (Some("simulation"), &auto["simulated"]["zone"])
        );
        // Off: the whole sample (more than 250 days) takes the zone of its latest window.
        let off = builtin("es_backtest", &["r", "f"], json!({"simulations": 0}))
            .run(&i)
            .unwrap();
        assert!(off.get("simulated").is_none());
        assert_eq!(off["zone_basis"], "latest 250-day window");
        assert_eq!(
            (&off["zone"], &off["status"]),
            (&off["latest_window"]["zone"], &off["latest_window"]["status"])
        );
        // Forecasts without an ES (a VaR-only external model) cannot be ES-backtested.
        i.insert("f".to_string(), json!({"start": 1, "var": [0.02]}));
        assert!(builtin("es_backtest", &["r", "f"], json!({})).run(&i).is_err());
        // An external model's ES forecasts are backtested, but cannot be simulated.
        i.insert(
            "f".to_string(),
            json!({"start": 800, "es": vec![0.03; 100], "es_var": vec![0.02; 100]}),
        );
        let ext = builtin("es_backtest", &["r", "f"], json!({})).run(&i).unwrap();
        assert_eq!(ext["zone_basis"], "fixed thresholds");
        assert!(ext.get("simulated").is_none());
        let e = builtin("es_backtest", &["r", "f"], json!({"simulations": 100}))
            .run(&i)
            .unwrap_err();
        assert!(e.to_string().contains("var_model"));
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

    #[test]
    fn ifrs9_steps() {
        let mut i = BTreeMap::new();
        // Two bullet facilities, annual periods, flat 12-month PD: closed-form ECLs.
        i.insert(
            "book".to_string(),
            json!({"stage": [1, 2], "balance": [1000, 1000], "lgd": [0.5, 0.5], "eir": [0.0, 0.0],
                   "periods": [3, 3], "pd_12m": [0.1, 0.1], "grade": [1, 2], "reported": [50, 135.5]}),
        );
        let p = json!({"periods_per_year": 1, "repayment": "bullet", "segments": ["grade"]});
        let o = builtin("ecl", &["book"], p).run(&i).unwrap();
        let lifetime = 500.0 * (1.0 - 0.9f64.powi(3));
        assert!((o["total"]["ecl"].as_f64().unwrap() - (50.0 + lifetime)).abs() < 1e-9);
        assert_eq!(o["by"]["grade"].as_array().unwrap().len(), 2);
        // 135.5 reported against 135.5: within tolerance.
        assert_eq!(o["status"], "pass");
        // Curves by segment from a JSON document, and a migration matrix.
        i.insert("curves".to_string(), json!({"curves": {"1": [0.1], "2": [0.2, 0.3]}}));
        let p = json!({"periods_per_year": 1, "repayment": "bullet", "pd_curves": "curves"});
        let o = builtin("ecl", &["book", "curves"], p).run(&i).unwrap();
        assert!((o["by_stage"][1]["ecl"].as_f64().unwrap() - 500.0 * (1.0 - 0.7 * 0.7 / 0.8)).abs() < 1e-9);
        assert_eq!(o["status"], "fail");
        i.insert(
            "m".to_string(),
            json!({"states": ["A", "B", "D"], "matrix": [[0.9, 0.0, 0.1], [0.0, 0.9, 0.1], [0, 0, 1]]}),
        );
        let p = json!({"periods_per_year": 1, "repayment": "bullet", "pd_curves": "m"});
        let o = builtin("ecl", &["book", "m"], p).run(&i).unwrap();
        assert!((o["total"]["ecl"].as_f64().unwrap() - (50.0 + lifetime)).abs() < 1e-9);
        // Out-of-range sizes are refused before anything is allocated.
        let mut huge = i.clone();
        huge.get_mut("book").unwrap()["periods"] = json!([3, 1e18]);
        let e = builtin("ecl", &["book"], json!({})).run(&huge).unwrap_err();
        assert!(e.to_string().contains("periods, outside"));
        for p in [
            json!({"periods_per_year": 1e12}),
            json!({"pd_curves": "m", "years": 1e15}),
        ] {
            assert!(builtin("ecl", &["book", "m"], p).run(&i).is_err());
        }

        i.insert(
            "d".to_string(),
            json!({"lgd_estimate": [0.4, 0.4, 0.6, 0.6], "lgd_realized": [0.3, 0.5, 0.7, 0.5], "segment": [1, 1, 2, 2]}),
        );
        let o = builtin("lgd_backtest", &["d"], Value::Null).run(&i).unwrap();
        assert_eq!(o["segments"].as_array().unwrap().len(), 2);
        i.insert(
            "c".to_string(),
            json!({"limit": [100, 100, 100], "drawn": [50, 100, 20], "ead": [75, 100, 60], "ccf_estimate": [0.5, 0.5, 0.5]}),
        );
        let o = builtin("ccf_backtest", &["c"], Value::Null).run(&i).unwrap();
        assert_eq!(
            (o["excluded"].clone(), o["overall"]["observations"].clone()),
            (json!(1), json!(2))
        );
        i.insert(
            "s".to_string(),
            json!({"stage": [1, 1, 2], "dpd": [0, 45, 0], "pd_origination": [0.01, 0.01, 0.01],
                   "pd_current": [0.01, 0.01, 0.05]}),
        );
        let o = builtin("staging_check", &["s"], Value::Null).run(&i).unwrap();
        assert_eq!(
            (o["understaged"].clone(), o["status"].clone()),
            (json!(1), json!("fail"))
        );
    }

    #[test]
    fn date_labels() {
        assert_eq!(date_label(20200316.0), json!("2020-03-16"));
        assert_eq!(date_label(20201399.0), json!(20201399.0));
        assert_eq!(date_label(42.0), json!(42.0));
    }

    #[test]
    fn backtest_details_dates_windows_and_years() {
        // 600 days over 2019-2021 (200 per year), exceptions on days 10, 210, 211 and 599.
        let dates: Vec<f64> = (0..600)
            .map(|i| (2019 + i / 200) as f64 * 10_000.0 + 101.0 + (i % 200 / 28 * 100 + i % 200 % 28) as f64)
            .collect();
        let mut realized = vec![0.0; 600];
        for i in [10, 210, 211, 599] {
            realized[i] = -0.05;
        }
        let var = vec![0.02; 600];
        let d = backtest_details(&realized, &var, 0.99, 0, Some(&dates));
        let ex = d["exception_list"].as_array().unwrap();
        assert_eq!(ex.len(), 4);
        assert_eq!(
            (ex[1]["index"].clone(), ex[1]["date"].clone()),
            (json!(210), json!("2020-01-11"))
        );
        assert!((ex[0]["excess_loss"].as_f64().unwrap() - 0.03).abs() < 1e-12);
        // Two full windows, the last one ends at the latest observation: [100, 350) and [350, 600).
        let w = d["basel_windows"].as_array().unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0]["exceptions"].clone(), w[1]["exceptions"].clone()),
            (json!(2), json!(1))
        );
        assert_eq!(w[1]["to"], json!("2021-08-04"));
        assert_eq!(d["latest_basel_window"]["plus_factor"], json!(0.0));
        let y = d["by_year"].as_array().unwrap();
        assert_eq!(
            y.iter().map(|x| x["exceptions"].as_u64().unwrap()).collect::<Vec<_>>(),
            [1, 2, 1]
        );
        // Without dates: indices instead of dates, no yearly breakdown.
        let d = backtest_details(&realized, &var, 0.99, 7, None);
        assert_eq!(d["exception_list"][0]["index"], json!(17));
        assert!(d.get("by_year").is_none());
    }

    #[test]
    fn rolling_backtest_summary_is_unchanged() {
        let r: Vec<f64> = (0..700)
            .map(|i| ((i * 7919 % 1000) as f64 / 1000.0 - 0.5) * 0.04)
            .collect();
        let reference =
            serde_json::to_value(quantparity::validation::rolling_backtest(&r, 250, 0.99).unwrap()).unwrap();
        let mut i = Inputs::new();
        i.insert("r".to_string(), json!(r));
        let out = builtin("kupiec_backtest", &["r"], json!({"alpha": 0.99, "window": 250}))
            .run(&i)
            .unwrap();
        for (k, v) in reference.as_object().unwrap() {
            assert_eq!(&out[k], v, "{k}");
        }
        assert_eq!(out["basel_windows"].as_array().unwrap().len(), 1);
    }
}
