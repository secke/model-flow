//! Comparison of two run manifests: step by step, value by value, with tolerances.
//!
//! Steps are matched by name. Outputs are compared down to their leaves; arrays of objects
//! that carry an identifier (`id`, `name`, `date`…) are matched by that identifier, so that
//! a reordered or extended portfolio is still compared trade by trade. Each changed step is
//! classified as the origin of a change (its inputs are identical on both sides) or as a
//! downstream effect of an upstream change.

use crate::{Manifest, StepRecord};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Fields used, in this order, to match the elements of an array of objects.
const KEY_FIELDS: [&str; 11] = [
    "id", "name", "trade", "factor", "tenor", "maturity", "date", "key", "label", "index", "t",
];
/// Leaves whose change is reported as a change of verdict.
const STATUS_FIELDS: [&str; 3] = ["status", "zone", "decision"];

/// Tolerance applied to the numeric leaves whose path matches `path` (`*` and `?` wildcards).
/// A difference is within tolerance if it is within `abs` (or `bp` basis points, 1 bp = 1e-4)
/// **or** within `rel` of the larger magnitude. Without any bound, values must be identical.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tolerance {
    #[serde(default = "any_path")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bp: Option<f64>,
    /// Leaves under this path are not compared at all.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignore: bool,
}

fn any_path() -> String {
    "*".into()
}

impl Tolerance {
    pub fn check(&self) -> Result<(), String> {
        if self.abs.is_some() && self.bp.is_some() {
            return Err(format!("tolerance `{}`: give `abs` or `bp`, not both", self.path));
        }
        for (k, v) in [("abs", self.abs), ("rel", self.rel), ("bp", self.bp)] {
            if v.is_some_and(|x| !x.is_finite() || x < 0.0) {
                return Err(format!(
                    "tolerance `{}`: `{k}` must be a non-negative number",
                    self.path
                ));
            }
        }
        Ok(())
    }

    fn abs_bound(&self) -> Option<f64> {
        self.abs.or(self.bp.map(|b| b * 1e-4))
    }

    fn within(&self, delta: f64, scale: f64) -> bool {
        self.abs_bound().is_some_and(|a| delta <= a) || self.rel.is_some_and(|r| delta <= r * scale)
    }

    /// Short description, e.g. `abs 1e-6`, `0.5 bp or rel 1e-4`.
    pub fn describe(&self) -> String {
        let mut parts = vec![];
        if let Some(a) = self.abs {
            parts.push(format!("abs {a:e}"));
        }
        if let Some(b) = self.bp {
            parts.push(format!("{b} bp"));
        }
        if let Some(r) = self.rel {
            parts.push(format!("rel {r:e}"));
        }
        if parts.is_empty() {
            "exact".into()
        } else {
            parts.join(" or ")
        }
    }
}

/// Wildcard match: `*` any sequence (including empty), `?` any single character.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Numeric value changed.
    Number,
    /// Text, boolean or null changed, or the type changed.
    Value,
    /// Present on the right only.
    Added,
    /// Present on the left only.
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    /// `step.field[3].sub` or `step.trades[id=T1].npv`.
    pub path: String,
    pub kind: ChangeKind,
    pub left: Option<Value>,
    pub right: Option<Value>,
    /// `right - left`, numeric changes only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<f64>,
    /// `|right - left| / max(|left|, |right|)`, numeric changes only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel: Option<f64>,
    pub within_tolerance: bool,
    /// Tolerance that applied (`exact` when none).
    pub tolerance: String,
    /// The tolerance is in basis points: the value is a rate, shown in bp.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_bp: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Identical,
    /// Code, version or inputs changed but the output is identical.
    SameOutput,
    Changed,
    Added,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDiff {
    pub name: String,
    pub state: StepState,
    pub left_version: Option<String>,
    pub right_version: Option<String>,
    pub code_changed: bool,
    pub inputs_changed: bool,
    /// The output changed although the inputs are identical: the change starts here.
    pub origin: bool,
    /// Why the step changed: version, code, source data, upstream steps.
    pub cause: String,
    pub leaves_compared: usize,
    pub leaves_ignored: usize,
    pub within_tolerance: usize,
    pub breaches: usize,
    pub max_abs_delta: Option<f64>,
    pub max_rel_delta: Option<f64>,
    /// Changes grouped by field, array positions replaced by `[*]`.
    pub by_field: Vec<FieldSummary>,
    pub changes: Vec<Change>,
}

/// Changes of one field across the elements of its arrays (e.g. `ewma.var[*]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSummary {
    pub field: String,
    pub changes: usize,
    pub breaches: usize,
    pub max_abs_delta: Option<f64>,
    pub max_rel_delta: Option<f64>,
    /// Mean of `right - left` over the numeric changes (sign shows a systematic shift).
    pub mean_delta: Option<f64>,
    /// Every change of the field has a tolerance in basis points.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_bp: bool,
}

/// `a.b[3].c[id=T1]` → `a.b[*].c[*]`; quoted object keys (`["a b"]`) are kept.
pub fn field_of(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let (end, keep) = if let Some(key) = tail.strip_prefix("[\"") {
            // JSON string key: ends at the first `"]` that is not escaped.
            let close = key
                .match_indices("\"]")
                .find(|(j, _)| !key[..*j].ends_with('\\'))
                .map(|(j, _)| j + 4)
                .unwrap_or(tail.len());
            (close, true)
        } else {
            (tail.find(']').map(|j| j + 1).unwrap_or(tail.len()), false)
        };
        out.push_str(if keep { &tail[..end] } else { "[*]" });
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn summarize(changes: &[Change]) -> Vec<FieldSummary> {
    let mut groups: BTreeMap<String, Vec<&Change>> = BTreeMap::new();
    for c in changes {
        groups.entry(field_of(&c.path)).or_default().push(c);
    }
    let mut v: Vec<FieldSummary> = groups
        .into_iter()
        .map(|(field, cs)| {
            let deltas: Vec<f64> = cs.iter().filter_map(|c| c.delta).collect();
            FieldSummary {
                field,
                changes: cs.len(),
                breaches: cs.iter().filter(|c| !c.within_tolerance).count(),
                max_abs_delta: deltas.iter().map(|d| d.abs()).reduce(f64::max),
                max_rel_delta: cs.iter().filter_map(|c| c.rel).reduce(f64::max),
                mean_delta: (!deltas.is_empty()).then(|| deltas.iter().sum::<f64>() / deltas.len() as f64),
                in_bp: cs.iter().all(|c| c.in_bp),
            }
        })
        .collect();
    v.sort_by(|a, b| b.breaches.cmp(&a.breaches).then(b.changes.cmp(&a.changes)));
    v
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusChange {
    pub path: String,
    pub left: Value,
    pub right: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffReport {
    pub left_run_hash: String,
    pub right_run_hash: String,
    #[serde(default)]
    pub left_engine: Option<String>,
    #[serde(default)]
    pub right_engine: Option<String>,
    pub identical: bool,
    pub steps: Vec<StepDiff>,
    pub status_changes: Vec<StatusChange>,
    pub leaves_compared: usize,
    pub changes: usize,
    pub breaches: usize,
    /// Steps where a change starts, with their cause.
    pub origins: Vec<String>,
    /// No breach beyond tolerance and no added or removed step.
    pub ok: bool,
}

struct Walker<'a> {
    rules: &'a [Tolerance],
    changes: Vec<Change>,
    compared: usize,
    ignored: usize,
}

impl Walker<'_> {
    fn rule(&self, path: &str) -> Option<&Tolerance> {
        self.rules.iter().find(|r| glob_match(&r.path, path))
    }

    fn push(&mut self, path: String, kind: ChangeKind, left: Option<&Value>, right: Option<&Value>) {
        if self.rule(&path).is_some_and(|r| r.ignore) {
            self.ignored += 1;
            return;
        }
        self.compared += 1;
        self.changes.push(Change {
            path,
            kind,
            left: left.cloned(),
            right: right.cloned(),
            delta: None,
            rel: None,
            within_tolerance: false,
            tolerance: "exact".into(),
            in_bp: false,
        });
    }

    fn walk(&mut self, path: &str, a: &Value, b: &Value) {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => self.objects(path, x, y),
            (Value::Array(x), Value::Array(y)) => self.arrays(path, x, y),
            (Value::Number(x), Value::Number(y)) => {
                let rule = self.rule(path).cloned();
                if rule.as_ref().is_some_and(|r| r.ignore) {
                    self.ignored += 1;
                    return;
                }
                self.compared += 1;
                if x == y {
                    return;
                }
                let (l, r) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
                let delta = r - l;
                let scale = l.abs().max(r.abs());
                let rel = if scale > 0.0 { delta.abs() / scale } else { 0.0 };
                if delta == 0.0 {
                    return; // same number written differently (1 vs 1.0)
                }
                self.changes.push(Change {
                    path: path.to_string(),
                    kind: ChangeKind::Number,
                    left: Some(a.clone()),
                    right: Some(b.clone()),
                    delta: Some(delta),
                    rel: Some(rel),
                    within_tolerance: rule.as_ref().is_some_and(|t| t.within(delta.abs(), scale)),
                    in_bp: rule.as_ref().is_some_and(|t| t.bp.is_some()),
                    tolerance: rule.map(|t| t.describe()).unwrap_or_else(|| "exact".into()),
                });
            }
            _ if a == b => {
                if self.rule(path).is_some_and(|r| r.ignore) {
                    self.ignored += 1;
                } else {
                    self.compared += 1;
                }
            }
            _ => self.push(path.to_string(), ChangeKind::Value, Some(a), Some(b)),
        }
    }

    fn objects(&mut self, path: &str, x: &Map<String, Value>, y: &Map<String, Value>) {
        let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
        for k in keys {
            let p = join_key(path, k);
            match (x.get(k), y.get(k)) {
                (Some(a), Some(b)) => self.walk(&p, a, b),
                (Some(a), None) => self.missing(&p, a, ChangeKind::Removed),
                (None, Some(b)) => self.missing(&p, b, ChangeKind::Added),
                (None, None) => unreachable!(),
            }
        }
    }

    /// A subtree present on one side only counts as a single change.
    fn missing(&mut self, path: &str, v: &Value, kind: ChangeKind) {
        match kind {
            ChangeKind::Removed => self.push(path.to_string(), kind, Some(v), None),
            _ => self.push(path.to_string(), kind, None, Some(v)),
        }
    }

    fn arrays(&mut self, path: &str, x: &[Value], y: &[Value]) {
        if let Some(field) = key_field(x, y) {
            let index = |v: &[Value]| -> BTreeMap<String, usize> {
                v.iter().enumerate().map(|(i, e)| (key_text(&e[field]), i)).collect()
            };
            let (ix, iy) = (index(x), index(y));
            // Left order first, then the elements present on the right only.
            let mut seen = BTreeSet::new();
            let order = x.iter().chain(y).map(|e| key_text(&e[field]));
            for k in order {
                if !seen.insert(k.clone()) {
                    continue;
                }
                let p = format!("{path}[{field}={k}]");
                match (ix.get(&k), iy.get(&k)) {
                    (Some(&i), Some(&j)) => self.walk(&p, &x[i], &y[j]),
                    (Some(&i), None) => self.missing(&p, &x[i], ChangeKind::Removed),
                    (None, Some(&j)) => self.missing(&p, &y[j], ChangeKind::Added),
                    (None, None) => unreachable!(),
                }
            }
            return;
        }
        for i in 0..x.len().max(y.len()) {
            let p = format!("{path}[{i}]");
            match (x.get(i), y.get(i)) {
                (Some(a), Some(b)) => self.walk(&p, a, b),
                (Some(a), None) => self.missing(&p, a, ChangeKind::Removed),
                (None, Some(b)) => self.missing(&p, b, ChangeKind::Added),
                (None, None) => unreachable!(),
            }
        }
    }
}

fn join_key(path: &str, k: &str) -> String {
    let plain = !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    if plain {
        format!("{path}.{k}")
    } else {
        format!("{path}[{}]", Value::String(k.to_string()))
    }
}

fn key_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// First identifier field that is a unique scalar in every element of both arrays.
fn key_field(x: &[Value], y: &[Value]) -> Option<&'static str> {
    if x.is_empty() && y.is_empty() {
        return None;
    }
    KEY_FIELDS.into_iter().find(|f| {
        [x, y].iter().all(|arr| {
            let mut seen = BTreeSet::new();
            arr.iter().all(|e| {
                matches!(e.get(*f), Some(Value::String(_) | Value::Number(_))) && seen.insert(key_text(&e[*f]))
            })
        })
    })
}

fn record_cause(a: &StepRecord, b: &StepRecord, origin: bool, is_source: bool, engines: Option<&str>) -> String {
    let mut causes = vec![];
    if a.version != b.version {
        causes.push(format!("version {} → {}", a.version, b.version));
    } else if a.code_hash != b.code_hash {
        causes.push(match engines {
            // Built-in steps are fingerprinted with the engine version.
            Some(e) => format!("code or parameters changed without a version bump (engine {e})"),
            None => "code or parameters changed without a version bump".to_string(),
        });
    }
    if a.input_hash != b.input_hash {
        causes.push(if is_source {
            "inputs changed".into()
        } else {
            "upstream steps changed".into()
        });
    }
    if origin && causes.is_empty() {
        causes.push(if is_source {
            "source data changed".into()
        } else {
            "non-reproducible: same code and inputs".into()
        });
    }
    causes.join(", ")
}

/// Compares `left` (reference) with `right` (candidate). `rules`: first match wins.
pub fn diff(left: &Manifest, right: &Manifest, rules: &[Tolerance]) -> DiffReport {
    let engines = (left.engine != right.engine).then(|| {
        let name = |e: &Option<String>| e.clone().unwrap_or_else(|| "before 0.13".into());
        format!("{} → {}", name(&left.engine), name(&right.engine))
    });
    let rindex: BTreeMap<&str, &StepRecord> = right.steps.iter().map(|s| (s.name.as_str(), s)).collect();
    let lnames: BTreeSet<&str> = left.steps.iter().map(|s| s.name.as_str()).collect();
    let mut steps = vec![];
    for a in &left.steps {
        let Some(b) = rindex.get(a.name.as_str()) else {
            steps.push(one_sided(a, StepState::Removed));
            continue;
        };
        let mut w = Walker {
            rules,
            changes: vec![],
            compared: 0,
            ignored: 0,
        };
        w.walk(&a.name, &a.output, &b.output);
        let output_changed = a.output_hash != b.output_hash && !w.changes.is_empty();
        let code_changed = a.version != b.version || a.code_hash != b.code_hash;
        let inputs_changed = a.input_hash != b.input_hash;
        // A source step has no dependencies: its "input" is the data it reads.
        let is_source = a.input_hash == crate::hash_value(&serde_json::json!({}));
        let origin = output_changed && !inputs_changed;
        let state = if output_changed {
            StepState::Changed
        } else if code_changed || inputs_changed {
            StepState::SameOutput
        } else {
            StepState::Identical
        };
        let cause = if state == StepState::Identical {
            String::new()
        } else {
            record_cause(a, b, origin, is_source, engines.as_deref())
        };
        let numeric = w.changes.iter().filter(|c| c.kind == ChangeKind::Number);
        let max_abs = numeric.clone().filter_map(|c| c.delta.map(f64::abs)).reduce(f64::max);
        let max_rel = numeric.filter_map(|c| c.rel).reduce(f64::max);
        let within = w.changes.iter().filter(|c| c.within_tolerance).count();
        steps.push(StepDiff {
            name: a.name.clone(),
            state,
            left_version: Some(a.version.clone()),
            right_version: Some(b.version.clone()),
            code_changed,
            inputs_changed,
            origin,
            cause,
            leaves_compared: w.compared,
            leaves_ignored: w.ignored,
            within_tolerance: within,
            breaches: w.changes.len() - within,
            max_abs_delta: max_abs,
            max_rel_delta: max_rel,
            by_field: summarize(&w.changes),
            changes: w.changes,
        });
    }
    for b in right.steps.iter().filter(|s| !lnames.contains(s.name.as_str())) {
        steps.push(one_sided(b, StepState::Added));
    }

    let status_changes = steps
        .iter()
        .flat_map(|s| &s.changes)
        .filter(|c| {
            let leaf = c.path.rsplit(['.', '[']).next().unwrap_or("");
            STATUS_FIELDS.contains(&leaf) && c.left.is_some() && c.right.is_some()
        })
        .map(|c| StatusChange {
            path: c.path.clone(),
            left: c.left.clone().unwrap(),
            right: c.right.clone().unwrap(),
        })
        .collect();
    let origins = steps
        .iter()
        .filter(|s| s.origin || matches!(s.state, StepState::Added | StepState::Removed))
        .map(|s| format!("{} ({})", s.name, s.cause))
        .collect();
    let breaches = steps.iter().map(|s| s.breaches).sum();
    let structural = steps
        .iter()
        .any(|s| matches!(s.state, StepState::Added | StepState::Removed));
    DiffReport {
        left_run_hash: left.run_hash.clone(),
        right_run_hash: right.run_hash.clone(),
        left_engine: left.engine.clone(),
        right_engine: right.engine.clone(),
        identical: left.run_hash == right.run_hash,
        leaves_compared: steps.iter().map(|s| s.leaves_compared).sum(),
        changes: steps.iter().map(|s| s.changes.len()).sum(),
        breaches,
        ok: breaches == 0 && !structural,
        origins,
        status_changes,
        steps,
    }
}

fn one_sided(s: &StepRecord, state: StepState) -> StepDiff {
    let (lv, rv) = match state {
        StepState::Removed => (Some(s.version.clone()), None),
        _ => (None, Some(s.version.clone())),
    };
    StepDiff {
        name: s.name.clone(),
        state,
        left_version: lv,
        right_version: rv,
        code_changed: false,
        inputs_changed: false,
        origin: false,
        cause: match state {
            StepState::Removed => "step removed".into(),
            _ => "step added".into(),
        },
        leaves_compared: 0,
        leaves_ignored: 0,
        within_tolerance: 0,
        breaches: 0,
        max_abs_delta: None,
        max_rel_delta: None,
        by_field: vec![],
        changes: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FnStep, Pipeline};
    use serde_json::json;

    fn run(rate: f64, version: &str, extra: bool) -> Manifest {
        let mut p = Pipeline::new()
            .step(FnStep::new("rate", "1", &[], move |_| Ok(json!(rate))))
            .step(FnStep::new("book", version, &["rate"], move |i| {
                let r = i["rate"].as_f64().unwrap();
                let mut trades = vec![
                    json!({"id": "T1", "npv": 100.0 * r, "status": if r > 0.03 { "fail" } else { "pass" }}),
                    json!({"id": "T2", "npv": 50.0}),
                ];
                if extra {
                    trades.insert(0, json!({"id": "T3", "npv": 1.0}));
                }
                Ok(json!({"trades": trades, "currency": "EUR"}))
            }));
        if extra {
            p = p.step(FnStep::new("limit", "1", &["book"], |_| Ok(json!({"status": "pass"}))));
        }
        p.execute().unwrap()
    }

    #[test]
    fn identical_runs() {
        let d = diff(&run(0.02, "1", false), &run(0.02, "1", false), &[]);
        assert!(d.identical && d.ok);
        assert_eq!(d.changes, 0);
        assert!(d.steps.iter().all(|s| s.state == StepState::Identical));
        // rate; currency; T1 id, npv, status; T2 id, npv
        assert_eq!(d.leaves_compared, 7);
    }

    #[test]
    fn origin_and_propagation() {
        let d = diff(&run(0.02, "1", false), &run(0.035, "1", false), &[]);
        let rate = &d.steps[0];
        assert!(rate.origin && rate.cause == "source data changed");
        let book = &d.steps[1];
        assert!(!book.origin && book.cause == "upstream steps changed");
        assert_eq!(book.changes[0].path, "book.trades[id=T1].npv");
        assert!((book.changes[0].delta.unwrap() - 1.5).abs() < 1e-12);
        assert_eq!(d.status_changes[0].path, "book.trades[id=T1].status");
        assert_eq!(d.origins, vec!["rate (source data changed)"]);
        assert!(!d.ok && d.breaches == 3);
    }

    #[test]
    fn tolerances_first_match_wins() {
        let rules = [
            Tolerance {
                path: "rate".into(),
                bp: Some(200.0),
                ..Default::default()
            },
            Tolerance {
                path: "book.*.npv".into(),
                rel: Some(0.5),
                ..Default::default()
            },
            Tolerance {
                path: "*.status".into(),
                ignore: true,
                ..Default::default()
            },
            Tolerance {
                path: "*".into(),
                abs: Some(0.0),
                ..Default::default()
            },
        ];
        let d = diff(&run(0.02, "1", false), &run(0.035, "1", false), &rules);
        assert!(d.ok, "{d:?}");
        assert_eq!(d.steps[0].changes[0].tolerance, "200 bp");
        assert!(d.steps[0].changes[0].in_bp && d.steps[0].by_field[0].in_bp);
        assert_eq!(d.steps[1].leaves_ignored, 1);
        assert!(d.status_changes.is_empty());
    }

    #[test]
    fn code_change_with_identical_results_is_ok() {
        let d = diff(&run(0.02, "1", false), &run(0.02, "2", false), &[]);
        assert!(d.ok && !d.identical);
        assert_eq!(d.steps[1].state, StepState::SameOutput);
        assert_eq!(d.steps[1].cause, "version 1 → 2");
    }

    #[test]
    fn engine_upgrade_is_named() {
        let mut old = run(0.02, "1", false);
        old.engine = None;
        for s in &mut old.steps {
            s.code_hash = Some("old engine".into());
        }
        let d = diff(&old, &run(0.02, "1", false), &[]);
        assert!(d.ok && d.changes == 0);
        assert_eq!(
            d.steps[0].cause,
            format!(
                "code or parameters changed without a version bump (engine before 0.13 → {})",
                crate::engine()
            )
        );
    }

    #[test]
    fn keyed_arrays_and_structural_changes() {
        let d = diff(&run(0.02, "1", false), &run(0.02, "1", true), &[]);
        let book = &d.steps[1];
        // T3 inserted at the front: matched by id, so T1 and T2 are unchanged.
        assert_eq!(book.changes.len(), 1);
        assert_eq!(book.changes[0].path, "book.trades[id=T3]");
        assert_eq!(book.changes[0].kind, ChangeKind::Added);
        assert_eq!(d.steps[2].state, StepState::Added);
        assert!(!d.ok);
    }

    #[test]
    fn plain_arrays_and_paths() {
        let mut w = Walker {
            rules: &[],
            changes: vec![],
            compared: 0,
            ignored: 0,
        };
        w.walk(
            "s",
            &json!({"a b": [1, 2], "x": 1}),
            &json!({"a b": [1, 3, 4], "x": 1.0}),
        );
        let paths: Vec<_> = w.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["s[\"a b\"][1]", "s[\"a b\"][2]"]);
        assert_eq!(w.compared, 4);
    }

    #[test]
    fn fields() {
        assert_eq!(field_of("a.b[3].c[id=T1]"), "a.b[*].c[*]");
        assert_eq!(field_of("s[\"a b\"][2]"), "s[\"a b\"][*]");
        let d = diff(&run(0.02, "1", false), &run(0.035, "1", false), &[]);
        let f = &d.steps[1].by_field;
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].field.as_str(), f[0].changes), ("book.trades[*].npv", 1));
        assert!((f[0].mean_delta.unwrap() - 1.5).abs() < 1e-12);
    }

    #[test]
    fn globs() {
        assert!(glob_match("*", "a.b[0]"));
        assert!(glob_match("curve.*.rate", "curve.pillars[3].rate"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("curve.*.rate", "curve.pillars[3].df"));
        assert!(glob_match("*[tenor=1?Y]*", "c.p[tenor=10Y].z"));
    }

    #[test]
    fn rejects_bad_tolerance() {
        let t = Tolerance {
            path: "*".into(),
            abs: Some(1.0),
            bp: Some(1.0),
            ..Default::default()
        };
        assert!(t.check().is_err());
        let t = Tolerance {
            abs: Some(-1.0),
            ..Default::default()
        };
        assert!(t.check().is_err());
    }
}
