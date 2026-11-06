//! Sensitivity analysis: reruns another step of the pipeline with a shocked input or parameter
//! and measures how chosen outputs respond: the sensitivity of a model's outputs to its inputs
//! and assumptions that model risk guidance (SR 26-2, SS1/23) asks validators to test. With an
//! expected direction, the outputs must move monotonically with the shock; with a bound, a
//! relative change beyond it is flagged.

use crate::spec::{build_step, StepSpec};
use crate::steps::engine_hash;
use mf_core::diff::{glob_match, leaves};
use mf_core::{hash_bytes, Error, Inputs, Step};
use quantparity::Status;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Most shocked runs of the target a `sensitivity` step may ask for.
pub const MAX_POINTS: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Shocks {
    One(f64),
    Many(Vec<f64>),
}

impl Shocks {
    fn values(&self) -> Vec<f64> {
        match self {
            Shocks::One(x) => vec![*x],
            Shocks::Many(v) => v.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Expect {
    Increase,
    Decrease,
}

/// The expected direction of every output, or by output (`mf diff` paths, wildcards; first match).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Expectation {
    All(Expect),
    By(BTreeMap<String, Expect>),
}

impl Expectation {
    fn of(&self, path: &str) -> Option<Expect> {
        match self {
            Expectation::All(e) => Some(*e),
            Expectation::By(m) => m.iter().find(|(pat, _)| glob_match(pat, path)).map(|(_, e)| *e),
        }
    }
}

/// One shocked quantity: a dependency of the target (`input`, at `column` or JSON `pointer`), or
/// a parameter of the target (`param`, a key or a JSON pointer), multiplied, shifted or set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    pub input: Option<String>,
    pub column: Option<String>,
    pub pointer: Option<String>,
    pub param: Option<String>,
    pub multiply: Option<Shocks>,
    pub add: Option<Shocks>,
    pub set: Option<Shocks>,
    /// How the outputs should move when the shocked quantity increases.
    pub expect: Option<Expectation>,
    /// Largest acceptable relative change of an output (warns beyond).
    pub max_relative_change: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Multiply,
    Add,
    Set,
}

impl Scenario {
    fn op(&self) -> Result<(Op, Vec<f64>), String> {
        let ops = [
            (Op::Multiply, &self.multiply),
            (Op::Add, &self.add),
            (Op::Set, &self.set),
        ];
        let mut given = ops.iter().filter_map(|(o, s)| s.as_ref().map(|s| (*o, s.values())));
        match (given.next(), given.next()) {
            (Some((o, v)), None) if !v.is_empty() && v.iter().all(|x| x.is_finite()) => Ok((o, v)),
            (Some(_), None) => Err(format!("scenario {}: shocks must be finite numbers", self.name)),
            _ => Err(format!(
                "scenario {}: give one of `multiply`, `add` or `set`",
                self.name
            )),
        }
    }

    fn pointer(&self) -> Result<String, String> {
        let p = match (&self.input, &self.param, &self.column, &self.pointer) {
            (Some(_), None, Some(c), None) => format!("/{}", c.replace('~', "~0").replace('/', "~1")),
            (Some(_), None, None, Some(p)) => p.clone(),
            (Some(_), None, None, None) => String::new(),
            (None, Some(k), None, None) if k.starts_with('/') => k.clone(),
            (None, Some(k), None, None) => format!("/{}", k.replace('~', "~0").replace('/', "~1")),
            _ => {
                return Err(format!(
                    "scenario {}: give `input` (with `column` or `pointer`) or `param`",
                    self.name
                ))
            }
        };
        if !p.is_empty() && !p.starts_with('/') {
            return Err(format!("scenario {}: a JSON pointer starts with `/`", self.name));
        }
        Ok(p)
    }

    fn describe(&self) -> String {
        match (&self.input, &self.param) {
            (Some(i), _) => match (&self.column, &self.pointer) {
                (Some(c), _) => format!("input {i}, column {c}"),
                (_, Some(p)) => format!("input {i} at {p}"),
                _ => format!("input {i}"),
            },
            (_, Some(p)) => format!("parameter {p}"),
            _ => String::new(),
        }
    }
}

/// Applies the shock to every number under `v`; returns how many there were and their mean.
fn shock(v: &mut Value, op: Op, x: f64) -> (usize, f64) {
    match v {
        Value::Number(n) => {
            let a = n.as_f64().unwrap_or(f64::NAN);
            let b = match op {
                Op::Multiply => a * x,
                Op::Add => a + x,
                Op::Set => x,
            };
            *v = json!(b);
            (1, a)
        }
        Value::Array(items) => fold(items.iter_mut(), op, x),
        Value::Object(m) => fold(m.values_mut(), op, x),
        _ => (0, 0.0),
    }
}

fn fold<'a>(items: impl Iterator<Item = &'a mut Value>, op: Op, x: f64) -> (usize, f64) {
    let (mut n, mut sum) = (0, 0.0);
    for e in items {
        let (k, m) = shock(e, op, x);
        n += k;
        sum += m * k as f64;
    }
    (n, if n == 0 { 0.0 } else { sum / n as f64 })
}

pub struct SensitivityStep {
    pub name: String,
    pub version: String,
    /// The target's dependencies, then the target itself (its output is the base case).
    pub deps: Vec<String>,
    pub target: StepSpec,
    pub base: PathBuf,
    pub as_of: Option<String>,
    pub scenarios: Vec<Scenario>,
    /// Paths in the target's output, in the notation of `mf diff`, with `*` and `?` wildcards.
    pub outputs: Vec<String>,
    pub target_hash: Option<String>,
}

impl SensitivityStep {
    fn err(&self, m: impl Into<String>) -> Error {
        Error::Step {
            step: self.name.clone(),
            msg: m.into(),
        }
    }

    /// Numeric outputs of the target matching `outputs`, keyed by their path in its output.
    fn select(&self, out: &Value) -> BTreeMap<String, f64> {
        let mut flat = vec![];
        leaves("", out, &mut flat);
        flat.into_iter()
            .filter_map(|(p, v)| {
                let p = p.trim_start_matches('.').to_string();
                let keep = self.outputs.iter().any(|pat| glob_match(pat, &p));
                v.as_f64().filter(|_| keep).map(|x| (p, x))
            })
            .collect()
    }

    fn run_point(&self, inputs: &Inputs, s: &Scenario, op: Op, x: f64) -> Result<(Value, f64), String> {
        let pointer = s.pointer()?;
        let mut target = self.target.clone();
        let mut ins: Inputs = target
            .deps
            .iter()
            .filter_map(|d| inputs.get(d).map(|v| (d.clone(), v.clone())))
            .collect();
        let node = match (&s.input, &s.param) {
            (Some(i), _) => ins
                .get_mut(i)
                .ok_or(format!(
                    "scenario {}: `{i}` is not a dependency of {}",
                    s.name, target.name
                ))?
                .pointer_mut(&pointer),
            _ => target.params.pointer_mut(&pointer),
        }
        .ok_or(format!("scenario {}: nothing at {}", s.name, s.describe()))?;
        let (n, mean) = shock(node, op, x);
        if n == 0 {
            return Err(format!("scenario {}: no number at {}", s.name, s.describe()));
        }
        let step = build_step(&[], &target, &self.base, self.as_of.as_deref())?;
        let out = step
            .run(&ins)
            .map_err(|e| format!("scenario {}, shock {x}: {e}", s.name))?;
        // Direction of the shock: the quantity goes up when it is positive.
        let dir = match op {
            Op::Multiply => (x - 1.0) * mean.signum(),
            Op::Add => x,
            Op::Set => x - mean,
        };
        Ok((out, dir))
    }
}

impl Step for SensitivityStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn deps(&self) -> &[String] {
        &self.deps
    }

    /// The target's code, the scenarios and the outputs: changing any of them changes the analysis.
    fn code_hash(&self) -> Option<String> {
        let own = serde_json::to_string(&(&self.target_hash, &self.scenarios, &self.outputs)).unwrap();
        engine_hash(&format!("sensitivity {}", hash_bytes(own.as_bytes())))
    }

    fn run(&self, inputs: &Inputs) -> Result<Value, Error> {
        let base_out = inputs
            .get(&self.target.name)
            .ok_or_else(|| self.err(format!("missing output of {}", self.target.name)))?;
        let base = self.select(base_out);
        if base.is_empty() {
            return Err(self.err(format!("no numeric output of {} matches `outputs`", self.target.name)));
        }
        let mut points = vec![];
        for (k, s) in self.scenarios.iter().enumerate() {
            let (op, xs) = s.op().map_err(|e| self.err(e))?;
            s.pointer().map_err(|e| self.err(e))?;
            points.extend(xs.into_iter().map(|x| (k, op, x)));
        }
        if points.len() > MAX_POINTS {
            return Err(self.err(format!("{} shocked runs, at most {MAX_POINTS}", points.len())));
        }
        let jobs = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(points.len())
            .max(1);
        let results: Vec<Result<(Value, f64), String>> = std::thread::scope(|sc| {
            let chunks: Vec<_> = (0..jobs)
                .map(|j| {
                    let pts: Vec<_> = points.iter().skip(j).step_by(jobs).collect();
                    sc.spawn(move || {
                        pts.into_iter()
                            .map(|&(k, op, x)| self.run_point(inputs, &self.scenarios[k], op, x))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            let mut by_chunk: Vec<std::vec::IntoIter<_>> =
                chunks.into_iter().map(|h| h.join().unwrap().into_iter()).collect();
            (0..points.len()).map(|i| by_chunk[i % jobs].next().unwrap()).collect()
        });

        let mut scenarios = vec![];
        let mut overall = Status::Pass;
        let mut results = results.into_iter();
        for s in &self.scenarios {
            let (op, xs) = s.op().map_err(|e| self.err(e))?;
            let mut pts = vec![];
            let mut curve: Vec<(f64, BTreeMap<String, f64>)> = vec![(0.0, base.clone())];
            for x in xs {
                let (out, dir) = results.next().unwrap().map_err(|e| self.err(e))?;
                let got = self.select(&out);
                let mut outs = serde_json::Map::new();
                for (p, b) in &base {
                    let v = *got
                        .get(p)
                        .ok_or_else(|| self.err(format!("scenario {}: output {p} missing after the shock", s.name)))?;
                    let change = v - b;
                    let rel = (*b != 0.0).then(|| change / b.abs());
                    let mut o = json!({ "value": v, "change": change, "relative_change": rel });
                    if op == Op::Multiply && x != 1.0 {
                        o["elasticity"] = json!(rel.map(|r| r / (x - 1.0)));
                    }
                    outs.insert(p.clone(), o);
                }
                pts.push(json!({ "shock": x, "outputs": outs }));
                curve.push((dir, got));
            }
            let mut violations = vec![];
            let mut status = Status::Pass;
            curve.sort_by(|a, b| a.0.total_cmp(&b.0));
            for p in base.keys() {
                if let Some(e) = s.expect.as_ref().and_then(|x| x.of(p)) {
                    let sign = if e == Expect::Increase { 1.0 } else { -1.0 };
                    let ys: Vec<f64> = curve.iter().map(|(_, m)| m[p] * sign).collect();
                    if ys.windows(2).any(|w| w[1] < w[0]) {
                        violations.push(format!(
                            "{p} does not {} with the shock",
                            if sign > 0.0 { "increase" } else { "decrease" }
                        ));
                        status = Status::Fail;
                    } else if ys.windows(2).all(|w| w[1] == w[0]) {
                        violations.push(format!("{p} does not respond to the shock"));
                        status = status.max(Status::Warn);
                    }
                }
            }
            if let Some(m) = s.max_relative_change {
                for (p, b) in &base {
                    let worst = curve
                        .iter()
                        .map(|(_, c)| ((c[p] - b) / b.abs()).abs())
                        .fold(0.0, f64::max);
                    if *b != 0.0 && worst > m {
                        violations.push(format!("{p} changes by {:.1}%, above {:.1}%", 100.0 * worst, 100.0 * m));
                        status = status.max(Status::Warn);
                    }
                }
            }
            overall = overall.max(status);
            scenarios.push(json!({
                "name": s.name,
                "shocked": s.describe(),
                "operation": match op { Op::Multiply => "multiply", Op::Add => "add", Op::Set => "set" },
                "expect": s.expect,
                "points": pts,
                "violations": violations,
                "status": status,
            }));
        }
        Ok(json!({
            "target": self.target.name,
            "base": base,
            "scenarios": scenarios,
            "status": overall,
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::spec;
    use serde_json::json;

    const PIPELINE: &str = r#"
name = "s"

[[step]]
name = "book"
kind = "json"
version = "1"
path = "book.json"

[[step]]
name = "ecl"
kind = "ecl"
version = "1"
deps = ["book"]
params = { periods_per_year = 1, repayment = "bullet" }

[[step]]
name = "sens"
kind = "sensitivity"
version = "1"
target = "ecl"
outputs = ["total.ecl", "total.ecl_12m"]

[[step.scenario]]
name = "LGD"
input = "book"
column = "lgd"
multiply = [0.5, 0.8, 1.2]
expect = "increase"
max_relative_change = 0.1

[[step.scenario]]
name = "PD"
input = "book"
column = "pd_12m"
add = [-0.05, 0.05]
expect = "decrease"

[[step.scenario]]
name = "monthly periods"
param = "periods_per_year"
set = 12

[[step.scenario]]
name = "PD, by output"
input = "book"
column = "pd_12m"
add = 0.05
expect = { "total.ecl" = "increase", "total.ecl_12m" = "decrease" }

[[step.scenario]]
name = "EIR"
input = "book"
column = "eir"
multiply = 2
expect = "decrease"
"#;

    #[test]
    fn shocks_inputs_and_parameters() {
        let dir = std::env::temp_dir().join(format!("mf-sens-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("book.json"),
            r#"{"stage": [1, 2], "balance": [1000, 1000], "lgd": [0.5, 0.5], "eir": [0, 0],
                "periods": [3, 3], "pd_12m": [0.1, 0.1]}"#,
        )
        .unwrap();
        let s = spec::parse(PIPELINE).unwrap();
        let m = spec::run(&s, &dir, None).unwrap();
        let o = &m.steps.iter().find(|x| x.name == "sens").unwrap().output;
        let base = 50.0 + 500.0 * (1.0 - 0.9f64.powi(3));
        assert!((o["base"]["total.ecl"].as_f64().unwrap() - base).abs() < 1e-9);
        // The ECL is linear in the LGD: elasticity 1; +20% is beyond the 10% bound.
        let lgd = &o["scenarios"][0];
        for p in lgd["points"].as_array().unwrap() {
            assert!((p["outputs"]["total.ecl"]["elasticity"].as_f64().unwrap() - 1.0).abs() < 1e-12);
        }
        assert_eq!(lgd["status"], "warn");
        // The ECL increases with the PD: the expected decrease fails.
        assert_eq!(o["scenarios"][1]["status"], "fail");
        // A parameter: 3 periods are now 3 months, the lifetime ECL falls.
        let monthly = &o["scenarios"][2];
        assert!(monthly["points"][0]["outputs"]["total.ecl"]["change"].as_f64().unwrap() < 0.0);
        assert_eq!(monthly["status"], "pass");
        // Directions by output: only the 12-month ECL is expected to fall, and it does not.
        assert_eq!(
            o["scenarios"][3]["violations"],
            json!(["total.ecl_12m does not decrease with the shock"])
        );
        // Twice a zero rate is still zero: no response, a warning.
        assert_eq!(
            o["scenarios"][4]["violations"][0],
            "total.ecl does not respond to the shock"
        );
        assert_eq!(o["scenarios"][4]["status"], "warn");
        assert_eq!(o["status"], "fail");
        assert!(spec::verify(&s, &dir, &m).unwrap().is_empty());

        // A different scenario is a different analysis.
        let changed = spec::parse(&PIPELINE.replace("multiply = [0.5, 0.8, 1.2]", "multiply = [0.9]")).unwrap();
        let reasons = spec::verify(&changed, &dir, &m).unwrap();
        assert!(!reasons.is_empty() && reasons.iter().all(|r| r.step == "sens"));
        for bad in [
            PIPELINE.replace("target = \"ecl\"", "target = \"nope\""),
            PIPELINE.replace("column = \"lgd\"", "column = \"lgd\"\nparam = \"x\""),
            PIPELINE.replace("column = \"lgd\"", "column = \"none\""),
        ] {
            let s = spec::parse(&bad).unwrap();
            assert!(spec::run(&s, &dir, None).is_err());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
