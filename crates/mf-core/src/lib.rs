//! Deterministic orchestration engine: DAG of versioned steps,
//! SHA-256 fingerprints of inputs/outputs, run manifest and verified replay.

pub mod diff;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unknown dependency: {step} -> {dep}")]
    UnknownDep { step: String, dep: String },
    #[error("cycle detected in the pipeline")]
    Cycle,
    #[error("duplicate step: {0}")]
    Duplicate(String),
    #[error("step {step} failed: {msg}")]
    Step { step: String, msg: String },
}

/// Canonical fingerprint of a JSON value (object keys are sorted).
pub fn hash_value(v: &Value) -> String {
    let bytes = serde_json::to_vec(v).expect("JSON serialization");
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 fingerprint of raw bytes (code files, data).
pub fn hash_bytes(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

pub type Inputs = BTreeMap<String, Value>;

pub trait Step {
    fn name(&self) -> &str;
    /// Code/model version: any change must bump it.
    fn version(&self) -> &str;
    fn deps(&self) -> &[String];
    fn run(&self, inputs: &Inputs) -> Result<Value, Error>;
    /// Fingerprint of the executed code (scripts, binaries), if known.
    fn code_hash(&self) -> Option<String> {
        None
    }
}

type StepFn = Box<dyn Fn(&Inputs) -> Result<Value, String>>;

/// Step defined by a function.
pub struct FnStep {
    name: String,
    version: String,
    deps: Vec<String>,
    f: StepFn,
}

impl FnStep {
    pub fn new(
        name: &str,
        version: &str,
        deps: &[&str],
        f: impl Fn(&Inputs) -> Result<Value, String> + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            deps: deps.iter().map(|d| d.to_string()).collect(),
            f: Box::new(f),
        }
    }
}

impl Step for FnStep {
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
        (self.f)(inputs).map_err(|msg| Error::Step {
            step: self.name.clone(),
            msg,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepRecord {
    pub name: String,
    pub version: String,
    pub input_hash: String,
    pub output_hash: String,
    #[serde(default)]
    pub code_hash: Option<String>,
    pub output: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub steps: Vec<StepRecord>,
    /// Fingerprint of the whole run (names, versions, inputs, outputs).
    pub run_hash: String,
    /// Engine that produced the run, e.g. `modelflow 0.13.0` (absent before 0.13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
}

/// Engine name and version recorded in every manifest.
pub fn engine() -> String {
    format!("modelflow {}", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, PartialEq)]
pub struct Mismatch {
    pub step: String,
    pub reason: String,
}

#[derive(Default)]
pub struct Pipeline {
    steps: Vec<Box<dyn Step>>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn step(mut self, s: impl Step + 'static) -> Self {
        self.steps.push(Box::new(s));
        self
    }

    /// Stable topological order (Kahn, insertion order breaks ties).
    fn order(&self) -> Result<Vec<usize>, Error> {
        let mut idx: HashMap<&str, usize> = HashMap::new();
        for (i, s) in self.steps.iter().enumerate() {
            if idx.insert(s.name(), i).is_some() {
                return Err(Error::Duplicate(s.name().into()));
            }
        }
        let mut indeg = vec![0usize; self.steps.len()];
        let mut out: Vec<Vec<usize>> = vec![vec![]; self.steps.len()];
        for (i, s) in self.steps.iter().enumerate() {
            for d in s.deps() {
                let j = *idx.get(d.as_str()).ok_or_else(|| Error::UnknownDep {
                    step: s.name().into(),
                    dep: d.clone(),
                })?;
                indeg[i] += 1;
                out[j].push(i);
            }
        }
        let mut q: VecDeque<usize> = (0..self.steps.len()).filter(|&i| indeg[i] == 0).collect();
        let mut order = vec![];
        while let Some(i) = q.pop_front() {
            order.push(i);
            for &k in &out[i] {
                indeg[k] -= 1;
                if indeg[k] == 0 {
                    q.push_back(k);
                }
            }
        }
        if order.len() != self.steps.len() {
            return Err(Error::Cycle);
        }
        Ok(order)
    }

    pub fn execute(&self) -> Result<Manifest, Error> {
        let mut outputs: BTreeMap<String, Value> = BTreeMap::new();
        let mut records = vec![];
        for i in self.order()? {
            let s = &self.steps[i];
            let inputs: Inputs = s.deps().iter().map(|d| (d.clone(), outputs[d].clone())).collect();
            let input_hash = hash_value(&serde_json::to_value(&inputs).unwrap());
            let output = s.run(&inputs)?;
            records.push(StepRecord {
                name: s.name().into(),
                version: s.version().into(),
                input_hash,
                output_hash: hash_value(&output),
                code_hash: s.code_hash(),
                output: output.clone(),
            });
            outputs.insert(s.name().into(), output);
        }
        let run_hash = hash_value(
            &serde_json::to_value(
                records
                    .iter()
                    .map(|r| (&r.name, &r.version, &r.input_hash, &r.output_hash, &r.code_hash))
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        );
        Ok(Manifest {
            steps: records,
            run_hash,
            engine: Some(engine()),
        })
    }

    /// Re-runs the pipeline and compares it with a reference manifest.
    pub fn verify(&self, reference: &Manifest) -> Result<Vec<Mismatch>, Error> {
        let fresh = self.execute()?;
        let mut mm = vec![];
        if fresh.steps.len() != reference.steps.len() {
            mm.push(Mismatch {
                step: "*".into(),
                reason: "different number of steps".into(),
            });
        }
        for (a, b) in fresh.steps.iter().zip(&reference.steps) {
            if a.name != b.name {
                mm.push(Mismatch {
                    step: b.name.clone(),
                    reason: "different step".into(),
                });
                continue;
            }
            if a.version != b.version {
                mm.push(Mismatch {
                    step: a.name.clone(),
                    reason: format!("version {} -> {}", b.version, a.version),
                });
            }
            if a.code_hash != b.code_hash && a.version == b.version {
                mm.push(Mismatch {
                    step: a.name.clone(),
                    reason: "code or parameters changed without a version bump".into(),
                });
            }
            if a.input_hash != b.input_hash {
                mm.push(Mismatch {
                    step: a.name.clone(),
                    reason: "different inputs".into(),
                });
            }
            if a.output_hash != b.output_hash {
                mm.push(Mismatch {
                    step: a.name.clone(),
                    reason: "non-reproducible result".into(),
                });
            }
        }
        Ok(mm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pipe(k: i64) -> Pipeline {
        Pipeline::new()
            .step(FnStep::new("b", "1", &["a"], |i| {
                Ok(json!(i["a"].as_i64().unwrap() * 2))
            }))
            .step(FnStep::new("a", "1", &[], move |_| Ok(json!(k))))
    }

    #[test]
    fn executes_in_dependency_order() {
        let m = pipe(21).execute().unwrap();
        assert_eq!(m.steps[0].name, "a");
        assert_eq!(m.steps[1].output, json!(42));
    }

    #[test]
    fn replay_is_identical() {
        let m = pipe(21).execute().unwrap();
        assert!(pipe(21).verify(&m).unwrap().is_empty());
    }

    #[test]
    fn detects_changed_inputs() {
        let m = pipe(21).execute().unwrap();
        let mm = pipe(22).verify(&m).unwrap();
        assert!(mm.iter().any(|x| x.step == "a" && x.reason.contains("result")));
        assert!(mm.iter().any(|x| x.step == "b" && x.reason.contains("inputs")));
    }

    #[test]
    fn detects_cycle_and_unknown_dep() {
        let c = Pipeline::new()
            .step(FnStep::new("x", "1", &["y"], |_| Ok(json!(0))))
            .step(FnStep::new("y", "1", &["x"], |_| Ok(json!(0))));
        assert!(matches!(c.execute(), Err(Error::Cycle)));
        let u = Pipeline::new().step(FnStep::new("x", "1", &["zz"], |_| Ok(json!(0))));
        assert!(matches!(u.execute(), Err(Error::UnknownDep { .. })));
    }
}
