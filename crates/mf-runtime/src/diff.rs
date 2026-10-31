//! Rendering of a run comparison (`mf diff`): terminal text and Markdown (pull request comments,
//! CI job summaries). The comparison itself is in `mf_core::diff`.

use crate::{spec, store::Store};
use mf_core::diff::{Change, ChangeKind, DiffReport, StepDiff, StepState, Tolerance};
use mf_core::Manifest;
use serde_json::Value;
use std::path::Path;

/// A run designated by a reference, with its label and its pipeline (if any).
pub struct Resolved {
    pub label: String,
    pub manifest: Manifest,
    pub spec: Option<spec::PipelineSpec>,
}

/// Resolves `pipeline.toml` (latest run), `pipeline.toml#ID`, `#latest`, `#previous`, `#fresh`
/// (executed now, not recorded) or a manifest file `run.json` (`mf show`, `mf run -o`, or a run
/// returned by the API).
pub fn resolve(reference: &str) -> Result<Resolved, String> {
    let (path, sel) = reference.split_once('#').unwrap_or((reference, "latest"));
    if path.ends_with(".json") {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        return Ok(Resolved {
            label: path.to_string(),
            manifest: manifest_from_value(v).map_err(|e| format!("{path}: {e}"))?,
            spec: None,
        });
    }
    let (sp, base) = spec::load(Path::new(path))?;
    if sel == "fresh" {
        let manifest = spec::build(&sp, &base)?.execute().map_err(|e| e.to_string())?;
        return Ok(Resolved {
            label: format!("{path}#fresh"),
            manifest,
            spec: Some(sp),
        });
    }
    let store = Store::for_pipeline(&base)?;
    let id = match sel {
        "latest" => None,
        "previous" => {
            let runs = store.list(&sp.name)?;
            match runs.len() {
                n if n >= 2 => Some(runs[n - 2].id),
                _ => return Err(format!("{path}: fewer than two runs recorded")),
            }
        }
        n => Some(
            n.parse()
                .map_err(|_| format!("{reference}: expected #ID, #previous, #latest or #fresh"))?,
        ),
    };
    let (id, manifest) = store.get(&sp.name, id)?;
    Ok(Resolved {
        label: format!("{path}#{id}"),
        manifest,
        spec: Some(sp),
    })
}

/// A manifest, or a run as returned by the API (which embeds its manifest).
pub fn manifest_from_value(mut v: Value) -> Result<Manifest, String> {
    if v.get("steps").is_none() {
        if let Some(m) = v.get_mut("manifest") {
            v = m.take();
        }
    }
    serde_json::from_value(v).map_err(|e| format!("not a run manifest ({e})"))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ToleranceFile {
    #[serde(default, rename = "tolerance")]
    tolerances: Vec<Tolerance>,
}

/// `[[tolerance]]` rules of a TOML file.
pub fn load_tolerances(path: &Path) -> Result<Vec<Tolerance>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let t: ToleranceFile = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    for r in &t.tolerances {
        r.check()?;
    }
    Ok(t.tolerances)
}

/// Compact number: 6 significant digits, scientific notation outside [1e-3, 1e7).
pub fn num(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    let a = x.abs();
    if (1e-3..1e7).contains(&a) {
        let decimals = (5 - a.log10().floor() as i32).max(0) as usize;
        let s = format!("{x:.decimals$}");
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    } else {
        format!("{x:.3e}")
    }
}

fn value(v: &Option<Value>) -> String {
    match v {
        None => "—".into(),
        Some(Value::Number(n)) => n.as_f64().map(num).unwrap_or_else(|| n.to_string()),
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            let s = other.to_string();
            if s.chars().count() > 48 {
                format!("{}…", s.chars().take(47).collect::<String>())
            } else {
                s
            }
        }
    }
}

fn state(s: &StepDiff) -> &'static str {
    match s.state {
        StepState::Identical => "identical",
        StepState::SameOutput => "same output",
        StepState::Changed if s.breaches == 0 => "within tol.",
        StepState::Changed => "changed",
        StepState::Added => "added",
        StepState::Removed => "removed",
    }
}

/// Absolute change, in basis points for rates (tolerance in bp).
fn amount(x: f64, in_bp: bool, sign: bool) -> String {
    if !in_bp {
        return if sign { signed(x) } else { num(x) };
    }
    let v = x * 1e4;
    let s = if v.abs() >= 0.01 { format!("{v:.2}") } else { num(v) };
    format!("{}{s} bp", if sign && v > 0.0 { "+" } else { "" })
}

fn delta(c: &Change) -> String {
    match (c.delta, c.rel) {
        (Some(d), Some(r)) => format!("{} ({})", amount(d, c.in_bp, true), pct(r)),
        _ => match c.kind {
            ChangeKind::Added => "added".into(),
            ChangeKind::Removed => "removed".into(),
            _ => "changed".into(),
        },
    }
}

fn pct(r: f64) -> String {
    let p = r * 100.0;
    if p == 0.0 || p >= 0.01 {
        format!("{p:.2}%")
    } else {
        format!("{p:.1e}%")
    }
}

fn signed(x: f64) -> String {
    format!("{}{}", if x > 0.0 { "+" } else { "" }, num(x))
}

/// Changes beyond tolerance first (non-numeric first, then by relative size), then the others.
fn ranked(s: &StepDiff) -> Vec<&Change> {
    let mut v: Vec<&Change> = s.changes.iter().collect();
    v.sort_by(|a, b| {
        let key = |c: &Change| (c.within_tolerance, c.kind == ChangeKind::Number);
        key(a)
            .cmp(&key(b))
            .then(b.rel.unwrap_or(0.0).total_cmp(&a.rel.unwrap_or(0.0)))
    });
    v
}

/// One-line verdict.
pub fn verdict(d: &DiffReport) -> String {
    if d.identical {
        return "OK: identical runs (same run fingerprint)".into();
    }
    let changed_steps = d.steps.iter().filter(|s| s.breaches > 0).count();
    let origin = if d.origins.is_empty() {
        String::new()
    } else {
        format!("; origin: {}", d.origins.join(", "))
    };
    if !d.ok {
        let structural = d
            .steps
            .iter()
            .filter(|s| matches!(s.state, StepState::Added | StepState::Removed))
            .count();
        let mut what = vec![];
        if d.breaches > 0 {
            what.push(format!(
                "{} change{} beyond tolerance in {} step{}",
                d.breaches,
                plural(d.breaches),
                changed_steps,
                plural(changed_steps)
            ));
        }
        if structural > 0 {
            what.push(format!("{structural} step{} added or removed", plural(structural)));
        }
        return format!("FAIL: {}{origin}", what.join(", "));
    }
    if d.changes == 0 {
        let touched: Vec<&str> = d
            .steps
            .iter()
            .filter(|s| s.code_changed)
            .map(|s| s.name.as_str())
            .collect();
        return if touched.is_empty() {
            "OK: identical results".into()
        } else {
            format!("OK: identical results (code changed in: {})", touched.join(", "))
        };
    }
    format!(
        "OK: {} change{}, all within tolerance{origin}",
        d.changes,
        plural(d.changes)
    )
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn short_hash(h: &str) -> &str {
    &h[..h.len().min(16)]
}

/// Terminal rendering. `limit`: changes listed per step.
pub fn render_text(d: &DiffReport, left: &str, right: &str, limit: usize) -> String {
    let mut o = String::new();
    let engine = |e: &Option<String>| e.as_deref().map(|e| format!("  {e}")).unwrap_or_default();
    o += &format!(
        "left   {left}  run_hash {}{}\n",
        short_hash(&d.left_run_hash),
        engine(&d.left_engine)
    );
    o += &format!(
        "right  {right}  run_hash {}{}\n\n",
        short_hash(&d.right_run_hash),
        engine(&d.right_engine)
    );
    o += &format!(
        "{:<22} {:<12} {:>9} {:>8} {:>7} {:>9} {:>11}  {}\n",
        "step", "state", "compared", "changed", "within", "breaches", "max rel", "cause"
    );
    for s in &d.steps {
        o += &format!(
            "{:<22} {:<12} {:>9} {:>8} {:>7} {:>9} {:>11}  {}{}\n",
            s.name,
            state(s),
            s.leaves_compared,
            s.changes.len(),
            s.within_tolerance,
            s.breaches,
            s.max_rel_delta.map(pct).unwrap_or_default(),
            s.cause,
            if s.origin { "  ← origin" } else { "" }
        );
    }
    if !d.status_changes.is_empty() {
        o += "\nVerdict changes:\n";
        for c in &d.status_changes {
            o += &format!(
                "  {:<60} {} → {}\n",
                c.path,
                value(&Some(c.left.clone())),
                value(&Some(c.right.clone()))
            );
        }
    }
    for s in d.steps.iter().filter(|s| !s.changes.is_empty()) {
        o += &format!(
            "\n{} ({} changes, {} beyond tolerance)\n",
            s.name,
            s.changes.len(),
            s.breaches
        );
        o += &format!(
            "  {:<58} {:>8} {:>8} {:>12} {:>9} {:>12}\n",
            "field", "changed", "beyond", "max |Δ|", "max rel", "mean Δ"
        );
        for f in s.by_field.iter().take(limit) {
            o += &format!(
                "  {:<58} {:>8} {:>8} {:>12} {:>9} {:>12}\n",
                f.field,
                f.changes,
                f.breaches,
                f.max_abs_delta.map(|x| amount(x, f.in_bp, false)).unwrap_or_default(),
                f.max_rel_delta.map(pct).unwrap_or_default(),
                f.mean_delta.map(|x| amount(x, f.in_bp, true)).unwrap_or_default()
            );
        }
        if s.by_field.len() > limit {
            o += &format!("  … {} more fields\n", s.by_field.len() - limit);
        }
        o += "  largest changes:\n";
        for c in ranked(s).into_iter().take(limit) {
            o += &format!(
                "  {} {:<58} {:>14} → {:<14} {:<26} {}\n",
                if c.within_tolerance { " " } else { "!" },
                c.path,
                value(&c.left),
                value(&c.right),
                delta(c),
                c.tolerance
            );
        }
        if s.changes.len() > limit {
            o += &format!("  … {} more\n", s.changes.len() - limit);
        }
    }
    o += &format!("\n{}\n", verdict(d));
    o
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|")
}

/// Markdown rendering, for a pull request comment or `$GITHUB_STEP_SUMMARY`.
pub fn render_markdown(d: &DiffReport, left: &str, right: &str, limit: usize) -> String {
    let mut o = String::new();
    o += &format!("### ModelFlow diff — {}\n\n", verdict(d));
    o += &format!(
        "Reference `{}` (`{}`) · candidate `{}` (`{}`) · {} values compared, {} changed, {} beyond tolerance\n\n",
        md_cell(left),
        short_hash(&d.left_run_hash),
        md_cell(right),
        short_hash(&d.right_run_hash),
        d.leaves_compared,
        d.changes,
        d.breaches
    );
    o += "| Step | State | Compared | Changed | Beyond tol. | Max rel. | Cause |\n|---|---|---:|---:|---:|---:|---|\n";
    for s in &d.steps {
        o += &format!(
            "| `{}` | {} | {} | {} | {} | {} | {}{} |\n",
            s.name,
            state(s),
            s.leaves_compared,
            s.changes.len(),
            s.breaches,
            s.max_rel_delta.map(pct).unwrap_or_default(),
            md_cell(&s.cause),
            if s.origin { " **(origin)**" } else { "" }
        );
    }
    if !d.status_changes.is_empty() {
        o += "\n**Verdict changes**\n\n| Path | Reference | Candidate |\n|---|---|---|\n";
        for c in &d.status_changes {
            o += &format!(
                "| `{}` | {} | {} |\n",
                md_cell(&c.path),
                md_cell(&value(&Some(c.left.clone()))),
                md_cell(&value(&Some(c.right.clone())))
            );
        }
    }
    let changed: Vec<&StepDiff> = d.steps.iter().filter(|s| !s.changes.is_empty()).collect();
    if !changed.is_empty() {
        o += "\n**Changes by field**\n\n| Field | Changed | Beyond tol. | Max \\|Δ\\| | Max rel. | Mean Δ |\n|---|---:|---:|---:|---:|---:|\n";
        for s in &changed {
            for f in s.by_field.iter().take(limit) {
                o += &format!(
                    "| `{}` | {} | {} | {} | {} | {} |\n",
                    md_cell(&f.field),
                    f.changes,
                    f.breaches,
                    f.max_abs_delta.map(|x| amount(x, f.in_bp, false)).unwrap_or_default(),
                    f.max_rel_delta.map(pct).unwrap_or_default(),
                    f.mean_delta.map(|x| amount(x, f.in_bp, true)).unwrap_or_default()
                );
            }
        }
        o += "\n<details><summary>Largest changes</summary>\n\n| | Path | Reference | Candidate | Δ (rel.) | Tolerance |\n|---|---|---:|---:|---:|---|\n";
        for s in changed {
            for c in ranked(s).into_iter().take(limit) {
                o += &format!(
                    "| {} | `{}` | {} | {} | {} | {} |\n",
                    if c.within_tolerance { "" } else { "**!**" },
                    md_cell(&c.path),
                    md_cell(&value(&c.left)),
                    md_cell(&value(&c.right)),
                    delta(c),
                    c.tolerance
                );
            }
            if s.changes.len() > limit {
                o += &format!("| | `{}`: {} more | | | | |\n", s.name, s.changes.len() - limit);
            }
        }
        o += "\n</details>\n";
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(num(0.0), "0");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(-123.456789), "-123.457");
        assert_eq!(num(100.0), "100");
        assert_eq!(num(2.5e-12), "2.500e-12");
        assert_eq!(num(0.0123456789), "0.0123457");
    }
}
