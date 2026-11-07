//! Data quality: checks a data set before the models read it. The raw cells of a CSV or Parquet
//! file (or a table produced by another step) are tested against rules on completeness, type,
//! range, allowed values, uniqueness and consistency between columns, and profiled. BCBS 239
//! (principles 3 and 4) asks for complete, accurate risk data whose quality is measured; the
//! loading steps stop at the first bad cell, this one counts them all and says where they are.

use crate::steps::engine_hash;
use mf_core::{hash_bytes, Error, Inputs, Step};
use quantparity::Status;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

/// Rows listed as examples for each failed check.
const EXAMPLES: usize = 5;

/// Cells read as missing in a CSV file (besides the empty cell), whatever their case.
const MISSING: &[&str] = &["na", "n/a", "nan", "null", "none"];

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warn,
    #[default]
    Fail,
}

/// Checks on one or several columns (`[[step.rule]]`). Value checks skip missing cells, which
/// only `required` and `max_missing` test.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub name: Option<String>,
    pub column: Option<String>,
    #[serde(default)]
    pub columns: Vec<String>,
    /// No missing cell.
    #[serde(default)]
    pub required: bool,
    /// Largest acceptable share of missing cells.
    pub max_missing: Option<f64>,
    /// Finite numbers only.
    #[serde(default)]
    pub numeric: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Allowed values (numbers, strings or booleans).
    pub one_of: Option<Vec<Value>>,
    /// The value must not exceed (`not_above`) or fall below (`not_below`) this other column.
    pub not_above: Option<String>,
    pub not_below: Option<String>,
    /// The combination of the columns identifies each row.
    #[serde(default)]
    pub unique: bool,
    /// Share of rows that may break a value check (default 0).
    #[serde(default)]
    pub max_rate: f64,
    /// Absolute slack of `min`, `max`, `not_above` and `not_below` (rounding).
    #[serde(default)]
    pub tolerance: f64,
    /// Values that stand for a missing cell in these columns (sentinels such as -1 or 999).
    #[serde(default)]
    pub missing_values: Vec<Value>,
    /// Only the rows where this column takes one of these values (`{column, one_of}`).
    pub when: Option<When>,
    #[serde(default)]
    pub severity: Severity,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct When {
    pub column: String,
    pub one_of: Vec<Value>,
}

impl Rule {
    fn columns(&self) -> Vec<String> {
        self.column.iter().chain(&self.columns).cloned().collect()
    }

    fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.columns().join(", "))
    }

    /// Every column the rule reads.
    fn reads(&self) -> Vec<String> {
        let mut c = self.columns();
        c.extend(self.not_above.iter().chain(&self.not_below).cloned());
        c.extend(self.when.iter().map(|w| w.column.clone()));
        c
    }

    pub fn check(&self) -> Result<(), String> {
        let l = self.label();
        if self.columns().is_empty() {
            return Err(format!("rule {l}: give `column` or `columns`"));
        }
        let rate = |x: f64| (0.0..=1.0).contains(&x);
        if !rate(self.max_rate) || self.max_missing.is_some_and(|x| !rate(x)) {
            return Err(format!(
                "rule {l}: `max_rate` and `max_missing` are shares between 0 and 1"
            ));
        }
        if !(self.tolerance >= 0.0 && self.tolerance.is_finite()) {
            return Err(format!("rule {l}: `tolerance` is a non-negative number"));
        }
        if self.required && self.max_missing.is_some() {
            return Err(format!("rule {l}: `required` and `max_missing` exclude each other"));
        }
        if let (Some(a), Some(b)) = (self.min, self.max) {
            if a > b {
                return Err(format!("rule {l}: `min` is above `max`"));
            }
        }
        let checks = self.required
            || self.max_missing.is_some()
            || self.numeric
            || self.min.is_some()
            || self.max.is_some()
            || self.one_of.is_some()
            || self.not_above.is_some()
            || self.not_below.is_some()
            || self.unique;
        if !checks {
            return Err(format!("rule {l}: no check"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Missing,
    Num(f64),
    Text(Box<str>),
    Bool(bool),
}

impl Cell {
    fn of_text(s: &str) -> Cell {
        let s = s.trim();
        if s.is_empty() || MISSING.contains(&s.to_ascii_lowercase().as_str()) {
            Cell::Missing
        } else if let Ok(x) = s.parse::<f64>() {
            Cell::Num(x)
        } else {
            Cell::Text(s.into())
        }
    }

    fn of_json(v: &Value) -> Cell {
        match v {
            Value::Null => Cell::Missing,
            Value::Number(n) => Cell::Num(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => Cell::Text(s.as_str().into()),
            Value::Bool(b) => Cell::Bool(*b),
            other => Cell::Text(other.to_string().into()),
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            Cell::Num(x) if x.is_finite() => Some(*x),
            _ => None,
        }
    }

    fn matches(&self, v: &Value) -> bool {
        match (self, v) {
            (Cell::Num(x), Value::Number(n)) => n.as_f64() == Some(*x),
            (Cell::Text(s), Value::String(t)) => **s == **t,
            (Cell::Bool(b), Value::Bool(c)) => b == c,
            _ => false,
        }
    }

    fn key(&self) -> String {
        match self {
            Cell::Missing => "\u{0}".into(),
            Cell::Num(x) => format!("n{x}"),
            Cell::Text(s) => format!("s{s}"),
            Cell::Bool(b) => format!("b{b}"),
        }
    }
}

/// Columns of a data set, with the line of each record of a CSV file (a quoted cell may span
/// several lines); elsewhere rows are numbered from 1.
struct Data {
    columns: BTreeMap<String, Vec<Cell>>,
    absent: Vec<String>,
    rows: usize,
    lines: Option<Vec<u64>>,
}

impl Data {
    fn row(&self, i: usize) -> u64 {
        self.lines.as_ref().map_or(i as u64 + 1, |l| l[i])
    }
}

fn read_csv(path: &std::path::Path, wanted: &[String], delimiter: u8) -> Result<Data, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_path(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let header = rdr.headers().map_err(|e| e.to_string())?.clone();
    let (idx, absent): (Vec<_>, Vec<_>) = wanted
        .iter()
        .map(|c| (c, header.iter().position(|h| h.trim() == c)))
        .partition(|(_, i)| i.is_some());
    let mut columns: BTreeMap<String, Vec<Cell>> = idx.iter().map(|(c, _)| ((*c).clone(), vec![])).collect();
    // Behind a CRLF terminator, a record starts on the `\n` the reader has not counted yet.
    let crlf = {
        use std::io::Read;
        let mut head = vec![0u8; 1 << 16];
        let k = std::fs::File::open(path)
            .and_then(|mut f| f.read(&mut head))
            .unwrap_or(0);
        head[..k]
            .iter()
            .position(|&b| b == b'\n')
            .is_some_and(|i| i > 0 && head[i - 1] == b'\r')
    };
    let mut lines = vec![];
    let mut rec = csv::StringRecord::new();
    loop {
        let line = rdr.position().line() + 1;
        if !rdr.read_record(&mut rec).map_err(|e| format!("line {line}: {e}"))? {
            break;
        }
        for (c, i) in &idx {
            let cell = rec.get(i.unwrap()).map_or(Cell::Missing, Cell::of_text);
            columns.get_mut(*c).unwrap().push(cell);
        }
        lines.push(rec.position().map_or(line, |p| p.line() + u64::from(crlf)));
    }
    Ok(Data {
        columns,
        absent: absent.into_iter().map(|(c, _)| c.clone()).collect(),
        rows: lines.len(),
        lines: Some(lines),
    })
}

fn read_parquet(path: &std::path::Path, wanted: &[String]) -> Result<Data, String> {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use parquet::record::Field;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let reader = SerializedFileReader::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
    let schema = reader.metadata().file_metadata().schema_descr_ptr();
    let (present, absent): (Vec<_>, Vec<_>) = wanted
        .iter()
        .cloned()
        .partition(|c| schema.columns().iter().any(|d| d.name() == c));
    let mut columns: BTreeMap<String, Vec<Cell>> = present.iter().map(|c| (c.clone(), vec![])).collect();
    let mut rows = 0;
    for row in reader.get_row_iter(None).map_err(|e| e.to_string())? {
        let row = row.map_err(|e| e.to_string())?;
        for (name, field) in row.get_column_iter() {
            if let Some(col) = columns.get_mut(name) {
                col.push(match field {
                    Field::Null => Cell::Missing,
                    Field::Double(x) => Cell::Num(*x),
                    Field::Float(x) => Cell::Num(*x as f64),
                    Field::Long(x) => Cell::Num(*x as f64),
                    Field::Int(x) => Cell::Num(*x as f64),
                    Field::Short(x) => Cell::Num(*x as f64),
                    Field::Byte(x) => Cell::Num(*x as f64),
                    Field::UInt(x) => Cell::Num(*x as f64),
                    Field::ULong(x) => Cell::Num(*x as f64),
                    Field::Bool(b) => Cell::Bool(*b),
                    Field::Str(s) => Cell::Text(s.as_str().into()),
                    other => Cell::Text(other.to_string().into()),
                });
            }
        }
        rows += 1;
    }
    Ok(Data {
        columns,
        absent,
        rows,
        lines: None,
    })
}

/// A `{column: [...]}` table or an array of records.
fn read_value(v: &Value, wanted: &[String]) -> Result<Data, String> {
    let mut columns = BTreeMap::new();
    let mut absent = vec![];
    let rows = match v {
        Value::Object(t) => {
            let mut rows = None;
            for c in wanted {
                match t.get(c).map(|x| x.as_array()) {
                    Some(Some(a)) => {
                        if rows.is_some_and(|n| n != a.len()) {
                            return Err(format!("column `{c}` differs in length from the others"));
                        }
                        rows = Some(a.len());
                        columns.insert(c.clone(), a.iter().map(Cell::of_json).collect());
                    }
                    Some(None) => return Err(format!("`{c}` is not a column (array)")),
                    None => absent.push(c.clone()),
                }
            }
            rows.unwrap_or(0)
        }
        Value::Array(recs) => {
            for c in wanted {
                if recs.iter().any(|r| r.get(c).is_some()) {
                    let col = recs.iter().map(|r| r.get(c).map_or(Cell::Missing, Cell::of_json));
                    columns.insert(c.clone(), col.collect());
                } else {
                    absent.push(c.clone());
                }
            }
            recs.len()
        }
        _ => return Err("expected a table (`{column: [...]}`) or an array of records".into()),
    };
    Ok(Data {
        columns,
        absent,
        rows,
        lines: None,
    })
}

pub struct DataQualityStep {
    pub name: String,
    pub version: String,
    pub deps: Vec<String>,
    /// File checked (otherwise the table of the only dependency).
    pub path: Option<PathBuf>,
    pub delimiter: u8,
    pub rules: Vec<Rule>,
    pub params: Value,
}

impl DataQualityStep {
    fn err(&self, m: impl Into<String>) -> Error {
        Error::Step {
            step: self.name.clone(),
            msg: m.into(),
        }
    }

    fn wanted(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.rules
            .iter()
            .flat_map(Rule::reads)
            .filter(|c| seen.insert(c.clone()))
            .collect()
    }

    fn load(&self, inputs: &Inputs) -> Result<(Data, Value), Error> {
        let wanted = self.wanted();
        match &self.path {
            Some(p) => {
                let sha256 = file_hash(p).map_err(|e| self.err(format!("{}: {e}", p.display())))?;
                let parquet = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("parquet"));
                let data = if parquet {
                    read_parquet(p, &wanted)
                } else {
                    read_csv(p, &wanted, self.delimiter)
                }
                .map_err(|e| self.err(e))?;
                let file = p.file_name().map(|f| f.to_string_lossy().into_owned());
                Ok((data, json!({"file": file, "sha256": sha256})))
            }
            None => {
                let d = &self.deps[0];
                let v = inputs
                    .get(d)
                    .ok_or_else(|| self.err(format!("`{d}` is not declared in deps")))?;
                Ok((read_value(v, &wanted).map_err(|e| self.err(e))?, json!({"step": d})))
            }
        }
    }
}

/// SHA-256 of a file, read by blocks: the data a run checked, for lineage.
fn file_hash(p: &std::path::Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let (mut h, mut buf) = (Sha256::new(), vec![0u8; 1 << 20]);
    loop {
        match f.read(&mut buf)? {
            0 => break,
            k => h.update(&buf[..k]),
        }
    }
    Ok(h.finalize().iter().map(|x| format!("{x:02x}")).collect())
}

/// One check of a rule: the rows that break it, against the share it tolerates.
fn check(name: &str, bad: &[usize], n: usize, data: &Data, limit: f64, sev: Severity) -> (Value, Status) {
    let rate = if n == 0 { 0.0 } else { bad.len() as f64 / n as f64 };
    let status = if rate <= limit {
        Status::Pass
    } else if sev == Severity::Warn {
        Status::Warn
    } else {
        Status::Fail
    };
    let rows: Vec<u64> = bad.iter().take(EXAMPLES).map(|&i| data.row(i)).collect();
    let mut v = json!({"check": name, "count": bad.len(), "rate": rate, "limit": limit, "status": status});
    if !rows.is_empty() {
        v["rows"] = json!(rows);
    }
    (v, status)
}

/// Missing share, and range and mean of the numbers, of one column.
fn profile(col: &[&Cell], miss: impl Fn(&Cell) -> bool) -> Value {
    let missing = col.iter().filter(|c| miss(c)).count();
    let nums: Vec<f64> = col.iter().filter(|c| !miss(c)).filter_map(|c| c.num()).collect();
    let mut p = json!({"missing": missing, "missing_rate": if col.is_empty() { 0.0 } else { missing as f64 / col.len() as f64 }});
    if !nums.is_empty() {
        p["min"] = json!(nums.iter().cloned().fold(f64::INFINITY, f64::min));
        p["max"] = json!(nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        p["mean"] = json!(nums.iter().sum::<f64>() / nums.len() as f64);
    }
    p
}

impl Step for DataQualityStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn deps(&self) -> &[String] {
        &self.deps
    }
    /// The engine version, the rules and the parameters: a rule change is a change of the check.
    fn code_hash(&self) -> Option<String> {
        let rules = serde_json::to_string(&(&self.rules, &self.params)).unwrap();
        engine_hash(&format!("data_quality {}", hash_bytes(rules.as_bytes())))
    }

    fn run(&self, inputs: &Inputs) -> Result<Value, Error> {
        let (data, source) = self.load(inputs)?;
        let n = data.rows;
        let mut overall = Status::Pass;
        let mut out_rules = vec![];

        let min_rows = self.params.get("min_rows").and_then(Value::as_u64);
        let max_rows = self.params.get("max_rows").and_then(Value::as_u64);
        let mut rows_status = Status::Pass;
        if min_rows.is_some_and(|m| (n as u64) < m) || max_rows.is_some_and(|m| (n as u64) > m) {
            rows_status = Status::Fail;
        }
        overall = overall.max(rows_status);
        if !data.absent.is_empty() {
            overall = Status::Fail;
        }

        for r in &self.rules {
            let mut checks = vec![];
            let mut status = Status::Pass;
            let reads = r.reads();
            let gone: Vec<&String> = reads.iter().filter(|c| data.absent.contains(c)).collect();
            if !gone.is_empty() {
                out_rules.push(
                    json!({"rule": r.label(), "columns": r.columns(), "missing_columns": gone, "status": Status::Fail}),
                );
                continue;
            }
            let rows: Vec<usize> = match &r.when {
                Some(w) => {
                    let wc = &data.columns[&w.column];
                    (0..n).filter(|&i| w.one_of.iter().any(|v| wc[i].matches(v))).collect()
                }
                None => (0..n).collect(),
            };
            let m = rows.len();
            let mut add = |name: &str, bad: Vec<usize>, limit: f64, sev: Severity| {
                let (v, s) = check(name, &bad, m, &data, limit, sev);
                checks.push(v);
                status = status.max(s);
            };
            let cols = r.columns();
            let miss = |c: &Cell| *c == Cell::Missing || r.missing_values.iter().any(|v| c.matches(v));
            let tol = r.tolerance;
            if r.unique {
                let mut seen = HashSet::new();
                let bad = rows
                    .iter()
                    .copied()
                    .filter(|&i| {
                        let k: Vec<String> = cols.iter().map(|c| data.columns[c][i].key()).collect();
                        !seen.insert(k.join("\u{1}"))
                    })
                    .collect();
                add("unique", bad, 0.0, r.severity);
            }
            for c in &cols {
                let col = &data.columns[c];
                let label = |k: &str| {
                    if cols.len() > 1 {
                        format!("{c}: {k}")
                    } else {
                        k.to_string()
                    }
                };
                let missing: Vec<usize> = rows.iter().copied().filter(|&i| miss(&col[i])).collect();
                if r.required || r.max_missing.is_some() {
                    add(&label("missing"), missing, r.max_missing.unwrap_or(0.0), r.severity);
                }
                let present = || rows.iter().copied().filter(|&i| !miss(&col[i]));
                if r.numeric || r.min.is_some() || r.max.is_some() {
                    add(
                        &label("numeric"),
                        present().filter(|&i| col[i].num().is_none()).collect(),
                        r.max_rate,
                        r.severity,
                    );
                }
                if let Some(m) = r.min {
                    add(
                        &label("min"),
                        present()
                            .filter(|&i| col[i].num().is_some_and(|x| x < m - tol))
                            .collect(),
                        r.max_rate,
                        r.severity,
                    );
                }
                if let Some(m) = r.max {
                    add(
                        &label("max"),
                        present()
                            .filter(|&i| col[i].num().is_some_and(|x| x > m + tol))
                            .collect(),
                        r.max_rate,
                        r.severity,
                    );
                }
                if let Some(allowed) = &r.one_of {
                    add(
                        &label("one_of"),
                        present()
                            .filter(|&i| !allowed.iter().any(|v| col[i].matches(v)))
                            .collect(),
                        r.max_rate,
                        r.severity,
                    );
                }
                for (other, name, above) in [(&r.not_above, "not_above", true), (&r.not_below, "not_below", false)] {
                    if let Some(o) = other {
                        let oc = &data.columns[o];
                        let bad = present()
                            .filter(|&i| match (col[i].num(), oc[i].num()) {
                                (Some(x), Some(y)) => {
                                    if above {
                                        x > y + tol
                                    } else {
                                        x < y - tol
                                    }
                                }
                                _ => false,
                            })
                            .collect();
                        add(&label(&format!("{name} {o}")), bad, r.max_rate, r.severity);
                    }
                }
            }
            let profiles: BTreeMap<&String, Value> = cols
                .iter()
                .map(|c| {
                    (
                        c,
                        profile(&rows.iter().map(|&i| &data.columns[c][i]).collect::<Vec<_>>(), miss),
                    )
                })
                .collect();
            overall = overall.max(status);
            let mut out =
                json!({"rule": r.label(), "columns": cols, "checks": checks, "profile": profiles, "status": status});
            if r.when.is_some() {
                out["rows"] = json!(m);
            }
            out_rules.push(out);
        }

        let mut src = source;
        src["rows"] = json!(n);
        let mut out = json!({"source": src, "rules": out_rules, "status": overall});
        if min_rows.is_some() || max_rows.is_some() {
            out["row_count"] = json!({"rows": n, "min": min_rows, "max": max_rows, "status": rows_status});
        }
        if !data.absent.is_empty() {
            out["missing_columns"] = json!(data.absent);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mf-quality-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn checks_a_csv_file() {
        let dir = tmp("csv");
        std::fs::write(
            dir.join("loans.csv"),
            "id,pd,stage,limit,drawn,grade,note\n\
             1,0.02,1,100,50,A,\n\
             2,,2,100,120,B,\"two\nlines\"\n\
             2,1.5,4,100,80,Z,\n\
             4,NA,3,,10,A,\n\
             5,abc,1,100,100.004,C,\n",
        )
        .unwrap();
        let toml = "name='q'\n[[step]]\nname='dq'\nkind='data_quality'\nversion='1'\npath='loans.csv'\n\
                    params={min_rows=3}\n\
                    [[step.rule]]\ncolumn='id'\nrequired=true\nunique=true\n\
                    [[step.rule]]\ncolumn='pd'\nmin=0\nmax=1\nmax_missing=0.5\n\
                    [[step.rule]]\ncolumn='stage'\none_of=[1,2,3]\nseverity='warn'\n\
                    [[step.rule]]\ncolumn='drawn'\nnot_above='limit'\nmax_rate=0.25\ntolerance=0.01\n\
                    [[step.rule]]\ncolumn='grade'\none_of=['A','B','C']\n\
                    [[step.rule]]\ncolumn='stage'\nmissing_values=[4]\nmax_missing=0.2\n";
        let s = spec::parse(toml).unwrap();
        let m = spec::run(&s, &dir, None).unwrap();
        let o = &m.steps[0].output;
        assert_eq!(o["source"]["rows"], 5);
        assert_eq!(o["source"]["sha256"].as_str().unwrap().len(), 64);
        let rule = |k: usize| &o["rules"][k];
        let chk = |k: usize, c: &str| {
            rule(k)["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["check"] == c)
                .unwrap()
                .clone()
        };
        // Duplicate id on line 5 (the third record: the second spans lines 3 and 4).
        assert_eq!(chk(0, "unique")["rows"], json!([5]));
        assert_eq!(rule(0)["status"], "fail");
        // Two missing PDs (empty, NA): 40% within the 50% allowed; one text and one above 1.
        assert_eq!(chk(1, "missing")["count"], 2);
        assert_eq!(chk(1, "missing")["status"], "pass");
        assert_eq!(chk(1, "numeric")["rows"], json!([7]));
        assert_eq!(chk(1, "max")["rows"], json!([5]));
        assert_eq!(rule(1)["profile"]["pd"]["max"], 1.5);
        // Stage 4 only warns.
        assert_eq!(rule(2)["status"], "warn");
        // One drawn amount above its limit (120 > 100; 100.004 is within the rounding tolerance):
        // 20% within the 25% tolerated.
        assert_eq!(chk(3, "not_above limit")["count"], 1);
        assert_eq!(rule(3)["status"], "pass");
        assert_eq!(chk(4, "one_of")["rows"], json!([5]));
        // Stage 4 as a sentinel: missing, and out of the profile.
        assert_eq!(chk(5, "missing")["count"], 1);
        assert_eq!(rule(5)["profile"]["stage"]["max"], 3.0);
        assert_eq!(o["row_count"]["status"], "pass");
        assert_eq!(o["status"], "fail");
        assert!(spec::verify(&s, &dir, &m).unwrap().is_empty());
        let text = std::fs::read_to_string(dir.join("loans.csv")).unwrap();
        std::fs::write(dir.join("loans.csv"), text.replace('\n', "\r\n")).unwrap();
        let crlf = spec::run(&s, &dir, None).unwrap().steps[0].output.clone();
        assert_eq!(crlf["rules"], o["rules"]);
        std::fs::write(dir.join("loans.csv"), text).unwrap();

        // A changed rule is a changed check; an absent column fails the rule, not the run.
        let s2 = spec::parse(&toml.replace("max_rate=0.25", "max_rate=0.1")).unwrap();
        assert!(!spec::verify(&s2, &dir, &m).unwrap().is_empty());
        let s3 = spec::parse(&format!("{toml}[[step.rule]]\ncolumn='ead'\nrequired=true\n")).unwrap();
        let o3 = spec::run(&s3, &dir, None).unwrap().steps[0].output.clone();
        assert_eq!(o3["missing_columns"], json!(["ead"]));
        assert_eq!(o3["rules"][6]["status"], "fail");

        for bad in [
            "[[step.rule]]\ncolumn='x'\n",
            "[[step.rule]]\ncolumn='x'\nmin=2\nmax=1\n",
            "[[step.rule]]\ncolumn='x'\nrequired=true\nmax_missing=0.1\n",
            "[[step.rule]]\ncolumn='x'\nnumeric=true\nmax_rate=2\n",
            "[[step.rule]]\nnumeric=true\n",
            "[[step.rule]]\ncolumn='x'\nmin=0\ntolerance=-1\n",
        ] {
            let t = format!("name='q'\n[[step]]\nname='dq'\nkind='data_quality'\nversion='1'\npath='loans.csv'\n{bad}");
            assert!(spec::build(&spec::parse(&t).unwrap(), &dir).is_err(), "{bad}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checks_a_step_output() {
        let dir = tmp("json");
        std::fs::write(
            dir.join("book.json"),
            r#"[{"id": "a", "notional": 1e6, "ccy": "EUR"}, {"id": "b", "notional": null, "ccy": "USD"},
                {"id": "c", "notional": -5, "ccy": "XXX"}]"#,
        )
        .unwrap();
        let s = spec::parse(
            "name='q'\n[[step]]\nname='book'\nkind='json'\nversion='1'\npath='book.json'\n\
             [[step]]\nname='dq'\nkind='data_quality'\nversion='1'\ndeps=['book']\n\
             [[step.rule]]\ncolumns=['id','notional']\nrequired=true\n\
             [[step.rule]]\ncolumn='notional'\nmin=0\n\
             [[step.rule]]\ncolumn='ccy'\none_of=['EUR','USD']\nmax_rate=0.5\n\
             [[step.rule]]\ncolumn='notional'\nmin=0\nwhen={column='ccy',one_of=['EUR','USD']}\n",
        )
        .unwrap();
        let o = spec::run(&s, &dir, None).unwrap().steps[1].output.clone();
        assert_eq!(o["source"], json!({"step": "book", "rows": 3}));
        assert_eq!(o["rules"][0]["checks"][1]["check"], "notional: missing");
        assert_eq!(o["rules"][0]["checks"][1]["rows"], json!([2]));
        assert_eq!(o["rules"][1]["checks"][1]["rows"], json!([3]));
        assert_eq!(o["rules"][2]["status"], "pass");
        // Only the EUR and USD trades: the negative notional (XXX) is left out.
        assert_eq!(o["rules"][3]["rows"], 2);
        assert_eq!(o["rules"][3]["status"], "pass");
        assert_eq!(o["rules"][3]["profile"]["notional"]["missing"], 1);
        assert_eq!(o["status"], "fail");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
