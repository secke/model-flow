//! Validation over time: a dated pipeline run for many as-of dates (in parallel), and the series
//! of its results, date by date.

use crate::spec::{self, PipelineSpec};
use crate::store::Store;
use mf_core::diff::{glob_match, leaves};
use mf_core::Manifest;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Dates designated on the command line: `2024-01-31`, `D1,D2`, `all` (every available date) or
/// `new` (available dates without a recorded run).
pub fn select_dates(spec: &PipelineSpec, base: &Path, store: &Store, sel: &str) -> Result<Vec<String>, String> {
    let dates = match sel {
        "all" => spec::available_dates(spec, base)?,
        "new" => {
            let done: std::collections::BTreeSet<String> =
                store.list(&spec.name)?.into_iter().filter_map(|r| r.as_of).collect();
            spec::available_dates(spec, base)?
                .into_iter()
                .filter(|d| !done.contains(d))
                .collect()
        }
        list => list
            .split(',')
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .collect(),
    };
    for d in &dates {
        spec::check_as_of(d)?;
    }
    Ok(dates)
}

/// Runs the pipeline for every date on `workers` threads (0: one per CPU). Results are returned
/// in the order of `dates`; a failed date does not stop the others.
pub fn run_dates(
    spec: &PipelineSpec,
    base: &Path,
    dates: &[String],
    workers: usize,
) -> Vec<(String, Result<Manifest, String>)> {
    let n = match workers {
        0 => std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        w => w,
    }
    .min(dates.len().max(1));
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Result<Manifest, String>>>> = Mutex::new(vec![None; dates.len()]);
    std::thread::scope(|scope| {
        for _ in 0..n {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= dates.len() {
                    break;
                }
                let r = spec::run(spec, base, Some(&dates[i]));
                results.lock().unwrap()[i] = Some(r);
            });
        }
    });
    dates
        .iter()
        .cloned()
        .zip(results.into_inner().unwrap().into_iter().map(|r| r.unwrap()))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct SeriesRow {
    pub as_of: String,
    pub run_id: i64,
    /// Worst status of the run's checks.
    pub status: Option<String>,
    /// One value per column, `None` where the path does not exist on that date.
    pub values: Vec<Option<Value>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Series {
    pub pipeline: String,
    pub columns: Vec<String>,
    pub rows: Vec<SeriesRow>,
}

/// Date of a row: its as-of date, or `#ID` for the runs of an undated pipeline.
pub fn label(r: &SeriesRow) -> String {
    if r.as_of.is_empty() {
        format!("#{}", r.run_id)
    } else {
        r.as_of.clone()
    }
}

/// The values at `paths` (diff notation, `*` and `?` wildcards) in the latest run of every as-of
/// date, or in every run of an undated pipeline. Without paths: the status of every check step.
pub fn series(store: &Store, pipeline: &str, paths: &[String]) -> Result<Series, String> {
    let mut runs = store.latest_by_as_of(pipeline)?;
    if runs.is_empty() {
        runs = store.list(pipeline)?;
    }
    let mut columns: Vec<String> = vec![];
    let mut tables = vec![];
    for r in &runs {
        let (_, m) = store.get(pipeline, Some(r.id))?;
        let mut flat: Vec<(String, Value)> = vec![];
        for s in &m.steps {
            leaves(&s.name, &s.output, &mut flat);
        }
        let flat: BTreeMap<String, Value> = flat.into_iter().collect();
        if paths.is_empty() {
            for s in &m.steps {
                let p = format!("{}.status", s.name);
                if flat.contains_key(&p) && !columns.contains(&p) {
                    columns.push(p);
                }
            }
        } else {
            for pat in paths {
                if pat.contains(['*', '?']) {
                    for p in flat.keys().filter(|p| glob_match(pat, p)) {
                        if !columns.contains(p) {
                            columns.push(p.clone());
                        }
                    }
                } else if !columns.contains(pat) {
                    columns.push(pat.clone());
                }
            }
        }
        tables.push((r, flat));
    }
    let rows = tables
        .into_iter()
        .map(|(r, flat)| SeriesRow {
            as_of: r.as_of.clone().unwrap_or_default(),
            run_id: r.id,
            status: r.status.clone(),
            values: columns.iter().map(|c| flat.get(c).cloned()).collect(),
        })
        .collect();
    Ok(Series {
        pipeline: pipeline.to_string(),
        columns,
        rows,
    })
}

fn cell(v: &Option<Value>) -> String {
    match v {
        None => String::new(),
        Some(Value::Number(n)) => n.as_f64().map(crate::diff::num).unwrap_or_else(|| n.to_string()),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// Verdict changes over time: `(column, date, from, to)` for every text column, by date.
pub fn transitions(s: &Series) -> Vec<(String, String, String, String)> {
    let mut out = vec![];
    for (j, c) in s.columns.iter().enumerate() {
        let mut prev: Option<&str> = None;
        for (i, r) in s.rows.iter().enumerate() {
            let Some(Value::String(v)) = &r.values[j] else { continue };
            if let Some(p) = prev {
                if p != v {
                    out.push((i, j, (c.clone(), label(r), p.to_string(), v.clone())));
                }
            }
            prev = Some(v);
        }
    }
    // Rows are in date (or run) order.
    out.sort_by_key(|(i, j, _)| (*i, *j));
    out.into_iter().map(|(_, _, t)| t).collect()
}

pub fn render_csv(s: &Series) -> String {
    let quote = |x: &str| {
        if x.contains([',', '"', '\n']) {
            format!("\"{}\"", x.replace('"', "\"\""))
        } else {
            x.to_string()
        }
    };
    let mut o = format!(
        "as_of,run_id,status{}\n",
        s.columns.iter().map(|c| format!(",{}", quote(c))).collect::<String>()
    );
    for r in &s.rows {
        o += &format!("{},{},{}", r.as_of, r.run_id, r.status.as_deref().unwrap_or(""));
        for v in &r.values {
            // Full precision in CSV: the file is meant for further analysis.
            let x = match v {
                None => String::new(),
                Some(Value::String(t)) => t.clone(),
                Some(other) => other.to_string(),
            };
            o += &format!(",{}", quote(&x));
        }
        o.push('\n');
    }
    o
}

pub fn render_text(s: &Series) -> String {
    // Long paths are replaced by [1], [2]... in the header, with a legend below the table.
    let legend = s.columns.iter().any(|c| c.chars().count() > 24);
    let headers: Vec<String> = if legend {
        (1..=s.columns.len()).map(|i| format!("[{i}]")).collect()
    } else {
        s.columns.clone()
    };
    let mut widths: Vec<usize> = headers.iter().map(|c| c.chars().count().max(8)).collect();
    let cells: Vec<Vec<String>> = s.rows.iter().map(|r| r.values.iter().map(cell).collect()).collect();
    for row in &cells {
        for (j, c) in row.iter().enumerate() {
            widths[j] = widths[j].max(c.chars().count());
        }
    }
    let mut o = format!("{:<12} {:>6} {:<7}", "as_of", "run", "status");
    for (c, w) in headers.iter().zip(&widths) {
        o += &format!("  {c:>w$}");
    }
    o.push('\n');
    for (r, row) in s.rows.iter().zip(&cells) {
        o += &format!(
            "{:<12} {:>6} {:<7}",
            label(r),
            format!("#{}", r.run_id),
            r.status.as_deref().unwrap_or("-")
        );
        for (c, w) in row.iter().zip(&widths) {
            o += &format!("  {c:>w$}");
        }
        o.push('\n');
    }
    if legend {
        o.push('\n');
        for (h, c) in headers.iter().zip(&s.columns) {
            o += &format!("{h} {c}\n");
        }
    }
    let t = transitions(s);
    if !t.is_empty() {
        o += "\nChanges over time:\n";
        for (c, d, a, b) in t {
            o += &format!("  {d}  {c}: {a} → {b}\n");
        }
    }
    o += &format!("\n{} dates, {} columns\n", s.rows.len(), s.columns.len());
    o
}

pub fn render_markdown(s: &Series) -> String {
    let mut o = format!(
        "| As of | Run | Status |{}\n|---|---:|---|{}\n",
        s.columns.iter().map(|c| format!(" `{c}` |")).collect::<String>(),
        "---:|".repeat(s.columns.len())
    );
    for r in &s.rows {
        o += &format!(
            "| {} | #{} | {} |",
            r.as_of,
            r.run_id,
            r.status.as_deref().unwrap_or("")
        );
        for v in &r.values {
            o += &format!(" {} |", cell(v).replace('|', "\\|"));
        }
        o.push('\n');
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_in_parallel_then_series() {
        let dir = std::env::temp_dir().join(format!("mf-series-{}", std::process::id()));
        let dates = ["2024-01-31", "2024-02-29", "2024-03-29", "2024-04-30"];
        for (i, d) in dates.iter().enumerate() {
            std::fs::create_dir_all(dir.join(d)).unwrap();
            // The fourth date has a crash: the VaR rises and the backtest status changes.
            let shock = if i == 3 { -0.2 } else { -0.01 };
            let rows: String = (0..300)
                .map(|k| {
                    format!(
                        "{}\n",
                        if k % 50 == 0 {
                            shock
                        } else {
                            0.001 * ((k % 7) as f64 - 3.0)
                        }
                    )
                })
                .collect();
            std::fs::write(dir.join(d).join("r.csv"), format!("ret\n{rows}")).unwrap();
        }
        let sp = spec::parse(
            "name='mon'\n[[step]]\nname='r'\nkind='csv'\nversion='1'\npath='${as_of}/r.csv'\ncolumn='ret'\n\
             [[step]]\nname='var'\nkind='var_historical'\nversion='1'\ndeps=['r']\nparams={alpha=0.99}\n",
        )
        .unwrap();
        let store = Store::open(&dir.join(".modelflow/runs.db")).unwrap();
        let all = select_dates(&sp, &dir, &store, "all").unwrap();
        assert_eq!(all, dates);
        let out = run_dates(&sp, &dir, &all[..3], 3);
        for (_, m) in &out {
            store.save("mon", m.as_ref().unwrap(), "test").unwrap();
        }
        assert_eq!(select_dates(&sp, &dir, &store, "new").unwrap(), ["2024-04-30"]);
        let (_, m) = &run_dates(&sp, &dir, &["2024-04-30".to_string()], 0)[0];
        store.save("mon", m.as_ref().unwrap(), "test").unwrap();
        let s = series(&store, "mon", &["var.var".into(), "var.obs*".into()]).unwrap();
        assert_eq!(s.columns, ["var.var", "var.observations"]);
        assert_eq!(s.rows.len(), 4);
        assert_eq!(s.rows[3].as_of, "2024-04-30");
        let v = |i: usize| s.rows[i].values[0].as_ref().unwrap().as_f64().unwrap();
        assert_eq!(v(0), v(1));
        assert!(v(3) > v(0));
        assert!(render_csv(&s).starts_with("as_of,run_id,status,var.var,var.observations\n2024-01-31,"));
        let (_, m) = store.get_as_of("mon", "2024-02-29").unwrap();
        assert_eq!(m.as_of.as_deref(), Some("2024-02-29"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
