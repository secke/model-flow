//! Moteur d'orchestration déterministe : DAG d'étapes versionnées,
//! empreintes SHA-256 des entrées/sorties, manifeste d'exécution et rejeu vérifié.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("dépendance inconnue: {step} -> {dep}")]
    UnknownDep { step: String, dep: String },
    #[error("cycle détecté dans le pipeline")]
    Cycle,
    #[error("étape dupliquée: {0}")]
    Duplicate(String),
    #[error("échec de l'étape {step}: {msg}")]
    Step { step: String, msg: String },
}

/// Empreinte canonique d'une valeur JSON (les clés d'objet sont triées).
pub fn hash_value(v: &Value) -> String {
    let bytes = serde_json::to_vec(v).expect("sérialisation JSON");
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Empreinte SHA-256 d'octets bruts (fichiers de code, données).
pub fn hash_bytes(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

pub type Inputs = BTreeMap<String, Value>;

pub trait Step {
    fn name(&self) -> &str;
    /// Version du code/modèle : toute modification doit l'incrémenter.
    fn version(&self) -> &str;
    fn deps(&self) -> &[String];
    fn run(&self, inputs: &Inputs) -> Result<Value, Error>;
    /// Empreinte du code exécuté (scripts, binaires) si connue.
    fn code_hash(&self) -> Option<String> { None }
}

type StepFn = Box<dyn Fn(&Inputs) -> Result<Value, String>>;

/// Étape définie par une fonction.
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
    fn name(&self) -> &str { &self.name }
    fn version(&self) -> &str { &self.version }
    fn deps(&self) -> &[String] { &self.deps }
    fn run(&self, inputs: &Inputs) -> Result<Value, Error> {
        (self.f)(inputs).map_err(|msg| Error::Step { step: self.name.clone(), msg })
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
    /// Empreinte de l'ensemble (noms, versions, entrées, sorties).
    pub run_hash: String,
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
    pub fn new() -> Self { Self::default() }

    pub fn add(mut self, s: impl Step + 'static) -> Self {
        self.steps.push(Box::new(s));
        self
    }

    /// Ordre topologique stable (Kahn, ordre d'insertion pour départager).
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
                if indeg[k] == 0 { q.push_back(k); }
            }
        }
        if order.len() != self.steps.len() { return Err(Error::Cycle); }
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
        let run_hash = hash_value(&serde_json::to_value(
            records.iter().map(|r| (&r.name, &r.version, &r.input_hash, &r.output_hash, &r.code_hash)).collect::<Vec<_>>(),
        ).unwrap());
        Ok(Manifest { steps: records, run_hash })
    }

    /// Ré-exécute le pipeline et compare avec un manifeste de référence.
    pub fn verify(&self, reference: &Manifest) -> Result<Vec<Mismatch>, Error> {
        let fresh = self.execute()?;
        let mut mm = vec![];
        if fresh.steps.len() != reference.steps.len() {
            mm.push(Mismatch { step: "*".into(), reason: "nombre d'étapes différent".into() });
        }
        for (a, b) in fresh.steps.iter().zip(&reference.steps) {
            if a.name != b.name { mm.push(Mismatch { step: b.name.clone(), reason: "étape différente".into() }); continue; }
            if a.version != b.version { mm.push(Mismatch { step: a.name.clone(), reason: format!("version {} -> {}", b.version, a.version) }); }
            if a.code_hash != b.code_hash && a.version == b.version {
                mm.push(Mismatch { step: a.name.clone(), reason: "code modifié sans changement de version".into() });
            }
            if a.input_hash != b.input_hash { mm.push(Mismatch { step: a.name.clone(), reason: "entrées différentes".into() }); }
            if a.output_hash != b.output_hash { mm.push(Mismatch { step: a.name.clone(), reason: "résultat non reproductible".into() }); }
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
            .add(FnStep::new("b", "1", &["a"], |i| Ok(json!(i["a"].as_i64().unwrap() * 2))))
            .add(FnStep::new("a", "1", &[], move |_| Ok(json!(k))))
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
        assert!(mm.iter().any(|x| x.step == "a" && x.reason.contains("résultat")));
        assert!(mm.iter().any(|x| x.step == "b" && x.reason.contains("entrées")));
    }

    #[test]
    fn detects_cycle_and_unknown_dep() {
        let c = Pipeline::new()
            .add(FnStep::new("x", "1", &["y"], |_| Ok(json!(0))))
            .add(FnStep::new("y", "1", &["x"], |_| Ok(json!(0))));
        assert!(matches!(c.execute(), Err(Error::Cycle)));
        let u = Pipeline::new().add(FnStep::new("x", "1", &["zz"], |_| Ok(json!(0))));
        assert!(matches!(u.execute(), Err(Error::UnknownDep { .. })));
    }
}
