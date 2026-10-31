//! Python bindings for ModelFlow. The engine (DAG, fingerprints, built-in steps, history, replay,
//! diff) runs in Rust; Python functions can be registered as steps and are fingerprinted with
//! their source code and parameters. The public API is the `modelflow` package (python/).

use mf_core::diff::{DiffReport, Tolerance};
use mf_core::{hash_bytes, Error as StepFailure, Inputs, Manifest, Step};
use mf_runtime::spec::{self, PipelineSpec, StepSpec};
use mf_runtime::store::{overall_status, Store};
use mf_runtime::{diff as rdiff, report, steps::BuiltinStep};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple, PyType};
use pythonize::{depythonize, pythonize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

create_exception!(
    _core,
    ModelFlowError,
    PyException,
    "Error raised by the ModelFlow engine."
);
create_exception!(_core, StepError, ModelFlowError, "A pipeline step failed.");

fn mf_err(e: impl std::fmt::Display) -> PyErr {
    ModelFlowError::new_err(e.to_string())
}

fn to_py(py: Python<'_>, v: &Value) -> PyResult<PyObject> {
    Ok(pythonize(py, v).map_err(mf_err)?.unbind())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Converts a Python object into JSON: built-in types, plus NumPy arrays and scalars, pandas
/// DataFrames (dict of columns), Series and Index (lists) and dates (ISO 8601), by duck typing.
fn to_value(obj: &Bound<'_, PyAny>, path: &str) -> PyResult<Value> {
    if obj.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(b) = obj.downcast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    if obj.is_instance_of::<PyInt>() {
        if let Ok(i) = obj.extract::<i64>() {
            return Ok(Value::from(i));
        }
        if let Ok(u) = obj.extract::<u64>() {
            return Ok(Value::from(u));
        }
        return float_value(obj.extract::<f64>()?, path);
    }
    if let Ok(f) = obj.downcast::<PyFloat>() {
        return float_value(f.value(), path);
    }
    if let Ok(s) = obj.downcast::<PyString>() {
        return Ok(Value::String(s.to_cow()?.into_owned()));
    }
    if let Ok(d) = obj.downcast::<PyDict>() {
        let mut m = Map::new();
        for (k, v) in d.iter() {
            let key = match k.downcast::<PyString>() {
                Ok(s) => s.to_cow()?.into_owned(),
                Err(_) => k.str()?.to_cow()?.into_owned(),
            };
            let value = to_value(&v, &format!("{path}.{key}"))?;
            m.insert(key, value);
        }
        return Ok(Value::Object(m));
    }
    if obj.is_instance_of::<PyList>() || obj.is_instance_of::<PyTuple>() {
        let mut out = vec![];
        for (i, v) in obj.try_iter()?.enumerate() {
            out.push(to_value(&v?, &format!("{path}[{i}]"))?);
        }
        return Ok(Value::Array(out));
    }
    // pandas DataFrame: one list per column.
    if obj.hasattr("columns")? && obj.hasattr("to_dict")? {
        let kwargs = PyDict::new(obj.py());
        kwargs.set_item("orient", "list")?;
        return to_value(&obj.call_method("to_dict", (), Some(&kwargs))?, path);
    }
    // NumPy arrays and scalars, pandas Series and Index.
    if obj.hasattr("tolist")? {
        return to_value(&obj.call_method0("tolist")?, path);
    }
    if obj.hasattr("isoformat")? {
        return to_value(&obj.call_method0("isoformat")?, path);
    }
    if obj.hasattr("item")? {
        return to_value(&obj.call_method0("item")?, path);
    }
    Err(PyTypeError::new_err(format!(
        "{path}: cannot convert {} to JSON (use numbers, strings, lists, dicts, NumPy or pandas)",
        obj.get_type().name()?
    )))
}

fn float_value(x: f64, path: &str) -> PyResult<Value> {
    serde_json::Number::from_f64(x)
        .map(Value::Number)
        .ok_or_else(|| PyValueError::new_err(format!("{path}: non-finite number ({x}) cannot be recorded")))
}

fn opt_value(obj: Option<&Bound<'_, PyAny>>, what: &str) -> PyResult<Value> {
    match obj {
        Some(o) if !o.is_none() => to_value(o, what),
        _ => Ok(Value::Null),
    }
}

/// A Python function registered as a step.
struct PyStepDef {
    name: String,
    version: String,
    deps: Vec<String>,
    func: Py<PyAny>,
    params: Value,
    /// Fingerprint of the function's source code and parameters.
    code_hash: String,
}

struct PyFnStep {
    name: String,
    version: String,
    deps: Vec<String>,
    func: Py<PyAny>,
    params: Value,
    code_hash: String,
}

impl Step for PyFnStep {
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
        Some(self.code_hash.clone())
    }
    fn run(&self, inputs: &Inputs) -> Result<Value, StepFailure> {
        Python::with_gil(|py| -> PyResult<Value> {
            let kwargs = PyDict::new(py);
            for (k, v) in inputs {
                kwargs.set_item(k, pythonize(py, v).map_err(mf_err)?)?;
            }
            if let Value::Object(p) = &self.params {
                for (k, v) in p {
                    kwargs.set_item(k, pythonize(py, v).map_err(mf_err)?)?;
                }
            }
            let out = self.func.bind(py).call((), Some(&kwargs))?;
            to_value(&out, &self.name)
        })
        .map_err(|e| StepFailure::Step {
            step: self.name.clone(),
            msg: Python::with_gil(|py| match e.traceback(py).and_then(|t| t.format().ok()) {
                Some(tb) => format!("{e}\n{tb}"),
                None => e.to_string(),
            }),
        })
    }
}

/// A pipeline: steps declared as in a TOML file (data, built-in checks, external processes)
/// and Python functions. History is kept next to `workdir` (`.modelflow/runs.db`), or in
/// PostgreSQL when `MODELFLOW_DATABASE_URL` is set.
#[pyclass(subclass, module = "modelflow._core", name = "Pipeline")]
struct PyPipeline {
    name: String,
    workdir: PathBuf,
    model: Option<String>,
    steps: Vec<StepSpec>,
    functions: Vec<PyStepDef>,
    tolerances: Vec<Tolerance>,
}

impl PyPipeline {
    fn spec(&self) -> PipelineSpec {
        PipelineSpec {
            name: self.name.clone(),
            model: self.model.clone(),
            schedule: None,
            steps: self.steps.clone(),
            tolerances: self.tolerances.clone(),
        }
    }

    fn engine(&self, py: Python<'_>) -> PyResult<mf_core::Pipeline> {
        let mut p = spec::build(&self.spec(), &self.workdir).map_err(mf_err)?;
        for f in &self.functions {
            p = p.step(PyFnStep {
                name: f.name.clone(),
                version: f.version.clone(),
                deps: f.deps.clone(),
                func: f.func.clone_ref(py),
                params: f.params.clone(),
                code_hash: f.code_hash.clone(),
            });
        }
        Ok(p)
    }

    fn store(&self) -> PyResult<Store> {
        Store::for_pipeline(&self.workdir).map_err(mf_err)
    }

    fn check_new(&self, name: &str) -> PyResult<()> {
        if self.steps.iter().any(|s| s.name == name) || self.functions.iter().any(|f| f.name == name) {
            return Err(PyValueError::new_err(format!("duplicate step `{name}`")));
        }
        Ok(())
    }

    fn get_run(&self, run_id: Option<i64>) -> PyResult<PyRun> {
        let (id, manifest) = self.store()?.get(&self.name, run_id).map_err(mf_err)?;
        Ok(PyRun {
            pipeline: self.name.clone(),
            id: Some(id),
            manifest,
        })
    }
}

fn exec_error(e: mf_core::Error) -> PyErr {
    match e {
        StepFailure::Step { step, msg } => StepError::new_err(format!("step {step} failed: {msg}")),
        other => mf_err(other),
    }
}

#[pymethods]
impl PyPipeline {
    #[new]
    #[pyo3(signature = (name, workdir = None, model = None))]
    fn new(name: &str, workdir: Option<PathBuf>, model: Option<String>) -> Self {
        Self {
            name: name.to_string(),
            workdir: workdir.unwrap_or_else(|| PathBuf::from(".")),
            model,
            steps: vec![],
            functions: vec![],
            tolerances: vec![],
        }
    }

    /// Loads a TOML pipeline; relative paths start from its directory, which is also `workdir`.
    #[classmethod]
    fn load<'py>(cls: &Bound<'py, PyType>, path: PathBuf) -> PyResult<Bound<'py, PyAny>> {
        let (sp, base) = spec::load(&path).map_err(mf_err)?;
        let obj = cls.call1((sp.name.clone(), base, sp.model.clone()))?;
        {
            let mut p = obj.downcast::<PyPipeline>()?.borrow_mut();
            p.steps = sp.steps;
            p.tolerances = sp.tolerances;
        }
        Ok(obj)
    }

    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    #[getter]
    fn workdir(&self) -> PathBuf {
        self.workdir.clone()
    }

    /// Step names, in declaration order (declared steps, then Python functions).
    #[getter]
    fn steps(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(|s| s.name.clone())
            .chain(self.functions.iter().map(|f| f.name.clone()))
            .collect()
    }

    /// Diff tolerances, first matching rule wins.
    #[getter]
    fn tolerances(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_py(py, &serde_json::to_value(&self.tolerances).unwrap())
    }

    /// Adds a step declared as in a TOML `[[step]]` table: `kind` is `csv`, `parquet`, `json`,
    /// `process` or a built-in kind (see `modelflow.builtin_kinds()`); other fields as keywords.
    #[pyo3(signature = (name, kind, version = "1", deps = None, params = None, **fields))]
    fn add(
        &mut self,
        name: &str,
        kind: &str,
        version: &str,
        deps: Option<Vec<String>>,
        params: Option<&Bound<'_, PyAny>>,
        fields: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        self.check_new(name)?;
        let mut m = match fields {
            Some(f) => match to_value(f.as_any(), name)? {
                Value::Object(m) => m,
                _ => Map::new(),
            },
            None => Map::new(),
        };
        m.insert("name".into(), name.into());
        m.insert("kind".into(), kind.into());
        m.insert("version".into(), version.into());
        m.insert("deps".into(), serde_json::json!(deps.unwrap_or_default()));
        m.insert("params".into(), opt_value(params, "params")?);
        let s: StepSpec =
            serde_json::from_value(Value::Object(m)).map_err(|e| PyValueError::new_err(format!("step {name}: {e}")))?;
        // Validates the step (required fields, known kind) now rather than at run time.
        let probe = PipelineSpec {
            name: self.name.clone(),
            model: None,
            schedule: None,
            steps: vec![s.clone()],
            tolerances: vec![],
        };
        spec::build(&probe, &self.workdir).map_err(PyValueError::new_err)?;
        self.steps.push(s);
        Ok(())
    }

    /// Registers a Python function as a step (use the `Pipeline.step` decorator). `source` is the
    /// function's source code: with `params` and `version`, it is the step's code fingerprint.
    #[pyo3(signature = (func, name, version, deps, params = None, source = ""))]
    fn add_function(
        &mut self,
        func: Py<PyAny>,
        name: &str,
        version: &str,
        deps: Vec<String>,
        params: Option<&Bound<'_, PyAny>>,
        source: &str,
    ) -> PyResult<()> {
        self.check_new(name)?;
        let params = opt_value(params, "params")?;
        if !(params.is_null() || params.is_object()) {
            return Err(PyValueError::new_err("params must be a dict"));
        }
        let mut buf = b"python\x1f".to_vec();
        buf.extend(source.as_bytes());
        buf.push(0x1f);
        buf.extend(serde_json::to_vec(&params).unwrap());
        self.functions.push(PyStepDef {
            name: name.to_string(),
            version: version.to_string(),
            deps,
            func,
            params,
            code_hash: hash_bytes(&buf),
        });
        Ok(())
    }

    /// Adds a diff tolerance on the paths matching `path` (`*`, `?`): `abs`, `bp` (basis points)
    /// or `rel`; `ignore=True` excludes the paths from the comparison.
    #[pyo3(signature = (path = "*", abs = None, rel = None, bp = None, ignore = false))]
    fn tolerance(
        &mut self,
        path: &str,
        abs: Option<f64>,
        rel: Option<f64>,
        bp: Option<f64>,
        ignore: bool,
    ) -> PyResult<()> {
        let t = Tolerance {
            path: path.to_string(),
            abs,
            rel,
            bp,
            ignore,
        };
        t.check().map_err(PyValueError::new_err)?;
        self.tolerances.push(t);
        Ok(())
    }

    /// Executes the pipeline. With `record=True` (default) the run is stored in the history.
    #[pyo3(signature = (record = true, triggered_by = None))]
    fn run(&self, py: Python<'_>, record: bool, triggered_by: Option<String>) -> PyResult<PyRun> {
        let manifest = self.engine(py)?.execute().map_err(exec_error)?;
        let id = if record {
            let who = triggered_by
                .unwrap_or_else(|| format!("python:{}", std::env::var("USER").unwrap_or_else(|_| "unknown".into())));
            Some(self.store()?.save(&self.name, &manifest, &who).map_err(mf_err)?)
        } else {
            None
        };
        Ok(PyRun {
            pipeline: self.name.clone(),
            id,
            manifest,
        })
    }

    /// Recorded run `run_id`, or the latest one.
    #[pyo3(signature = (run_id = None))]
    fn get(&self, run_id: Option<i64>) -> PyResult<PyRun> {
        self.get_run(run_id)
    }

    /// Recorded runs, oldest first: id, run_hash, created_at, triggered_by, status.
    fn runs(&self, py: Python<'_>) -> PyResult<PyObject> {
        let rows = self.store()?.list(&self.name).map_err(mf_err)?;
        to_py(py, &serde_json::to_value(rows).unwrap())
    }

    /// Replays a recorded run (default: the latest) and returns the mismatches: an empty list
    /// means the run is reproduced identically.
    #[pyo3(signature = (run_id = None))]
    fn verify(&self, py: Python<'_>, run_id: Option<i64>) -> PyResult<PyObject> {
        let reference = self.get_run(run_id)?.manifest;
        let mm = self.engine(py)?.verify(&reference).map_err(exec_error)?;
        let v: Vec<Value> = mm
            .iter()
            .map(|m| serde_json::json!({"step": m.step, "reason": m.reason}))
            .collect();
        to_py(py, &Value::Array(v))
    }

    /// Compares two recorded runs with this pipeline's tolerances (default: previous vs latest).
    #[pyo3(signature = (left = None, right = None))]
    fn diff(&self, left: Option<i64>, right: Option<i64>) -> PyResult<PyDiff> {
        let ids: Vec<i64> = self
            .store()?
            .list(&self.name)
            .map_err(mf_err)?
            .iter()
            .map(|r| r.id)
            .collect();
        let pick = |given: Option<i64>, back: usize| -> PyResult<i64> {
            given
                .or_else(|| ids.len().checked_sub(back).map(|i| ids[i]))
                .ok_or_else(|| mf_err("not enough recorded runs to compare"))
        };
        let (l, r) = (pick(left, 2)?, pick(right, 1)?);
        let (a, b) = (self.get_run(Some(l))?, self.get_run(Some(r))?);
        Ok(PyDiff::new(
            mf_core::diff::diff(&a.manifest, &b.manifest, &self.tolerances),
            format!("{}#{l}", self.name),
            format!("{}#{r}", self.name),
        ))
    }

    /// Writes the HTML validation report of a recorded run (default: the latest, replayed first).
    #[pyo3(signature = (path, run_id = None, verify = true))]
    fn report(&self, py: Python<'_>, path: PathBuf, run_id: Option<i64>, verify: bool) -> PyResult<PathBuf> {
        let run = self.get_run(run_id)?;
        let id = run.id.unwrap();
        let replay = if verify {
            Some(self.engine(py)?.verify(&run.manifest).map_err(exec_error)?)
        } else {
            None
        };
        let created_at = self
            .store()?
            .info(&self.name, id)
            .map_err(mf_err)?
            .map(|r| r.created_at)
            .unwrap_or_default();
        let meta = report::ReportMeta {
            pipeline: &self.name,
            run_id: id,
            created_at,
            generated_at: now(),
            replay: replay.as_deref(),
            decisions: &[],
        };
        std::fs::write(&path, report::render_html(&meta, &run.manifest)).map_err(mf_err)?;
        Ok(path)
    }

    fn __repr__(&self) -> String {
        format!(
            "Pipeline({:?}, {} steps, workdir={:?})",
            self.name,
            self.steps.len() + self.functions.len(),
            self.workdir
        )
    }
}

/// A run: its manifest (steps, fingerprints, outputs) and, if recorded, its id.
#[pyclass(module = "modelflow._core", name = "Run", frozen)]
struct PyRun {
    pipeline: String,
    id: Option<i64>,
    manifest: Manifest,
}

#[pymethods]
impl PyRun {
    #[getter]
    fn pipeline(&self) -> &str {
        &self.pipeline
    }

    /// Id in the history, `None` if the run was not recorded.
    #[getter]
    fn id(&self) -> Option<i64> {
        self.id
    }

    #[getter]
    fn run_hash(&self) -> &str {
        &self.manifest.run_hash
    }

    #[getter]
    fn engine(&self) -> Option<String> {
        self.manifest.engine.clone()
    }

    /// Worst status of the check steps (`pass`, `warn`, `fail`), `None` if there are none.
    #[getter]
    fn status(&self) -> Option<String> {
        overall_status(&self.manifest)
    }

    #[getter]
    fn steps(&self) -> Vec<String> {
        self.manifest.steps.iter().map(|s| s.name.clone()).collect()
    }

    /// Output of every step, by name.
    #[getter]
    fn outputs(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = PyDict::new(py);
        for s in &self.manifest.steps {
            d.set_item(&s.name, to_py(py, &s.output)?)?;
        }
        Ok(d.into_any().unbind())
    }

    /// Output of one step.
    fn output(&self, py: Python<'_>, step: &str) -> PyResult<PyObject> {
        let s = self
            .manifest
            .steps
            .iter()
            .find(|s| s.name == step)
            .ok_or_else(|| PyValueError::new_err(format!("unknown step `{step}`")))?;
        to_py(py, &s.output)
    }

    /// The full manifest, as a dict (`json.dump` it to keep a reference for `modelflow.diff`).
    #[getter]
    fn manifest(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_py(py, &serde_json::to_value(&self.manifest).unwrap())
    }

    /// Writes the manifest as JSON (a reference for `mf diff` or `modelflow.diff`).
    fn save(&self, path: PathBuf) -> PyResult<PathBuf> {
        std::fs::write(&path, serde_json::to_string_pretty(&self.manifest).unwrap()).map_err(mf_err)?;
        Ok(path)
    }

    fn __repr__(&self) -> String {
        format!(
            "Run({:?}, id={}, run_hash={}…, status={})",
            self.pipeline,
            self.id.map(|i| i.to_string()).unwrap_or_else(|| "None".into()),
            &self.manifest.run_hash[..12],
            overall_status(&self.manifest).unwrap_or_else(|| "-".into())
        )
    }
}

/// Comparison of two runs (see `modelflow.diff`).
#[pyclass(module = "modelflow._core", name = "Diff", frozen)]
struct PyDiff {
    report: DiffReport,
    left: String,
    right: String,
}

impl PyDiff {
    fn new(report: DiffReport, left: String, right: String) -> Self {
        Self { report, left, right }
    }
}

#[pymethods]
impl PyDiff {
    /// No change beyond tolerance and no step added or removed.
    #[getter]
    fn ok(&self) -> bool {
        self.report.ok
    }

    /// Same run fingerprint on both sides.
    #[getter]
    fn identical(&self) -> bool {
        self.report.identical
    }

    #[getter]
    fn verdict(&self) -> String {
        rdiff::verdict(&self.report)
    }

    #[getter]
    fn left(&self) -> &str {
        &self.left
    }

    #[getter]
    fn right(&self) -> &str {
        &self.right
    }

    /// Number of changes beyond tolerance.
    #[getter]
    fn breaches(&self) -> usize {
        self.report.breaches
    }

    /// Steps where a change starts, with their cause.
    #[getter]
    fn origins(&self) -> Vec<String> {
        self.report.origins.clone()
    }

    /// Changes of verdict (`status`, `zone`, `decision` fields).
    #[getter]
    fn status_changes(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_py(py, &serde_json::to_value(&self.report.status_changes).unwrap())
    }

    /// Full comparison, as a dict (steps, fields, changes).
    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_py(py, &serde_json::to_value(&self.report).unwrap())
    }

    /// One row per step: state, counts, largest relative change, cause.
    fn summary(&self, py: Python<'_>) -> PyResult<PyObject> {
        let rows: Vec<Value> = self
            .report
            .steps
            .iter()
            .map(|s| {
                serde_json::json!({
                    "step": s.name, "state": s.state, "compared": s.leaves_compared,
                    "changed": s.changes.len(), "within_tolerance": s.within_tolerance,
                    "breaches": s.breaches, "max_abs_delta": s.max_abs_delta,
                    "max_rel_delta": s.max_rel_delta, "origin": s.origin, "cause": s.cause,
                })
            })
            .collect();
        to_py(py, &Value::Array(rows))
    }

    /// One row per changed value: step, path, field, kind, left, right, delta, rel, tolerance.
    fn changes(&self, py: Python<'_>) -> PyResult<PyObject> {
        let rows: Vec<Value> = self
            .report
            .steps
            .iter()
            .flat_map(|s| {
                s.changes.iter().map(move |c| {
                    serde_json::json!({
                        "step": s.name, "path": c.path, "field": mf_core::diff::field_of(&c.path),
                        "kind": c.kind, "left": c.left, "right": c.right, "delta": c.delta,
                        "rel": c.rel, "within_tolerance": c.within_tolerance, "tolerance": c.tolerance,
                    })
                })
            })
            .collect();
        to_py(py, &Value::Array(rows))
    }

    #[pyo3(signature = (limit = 10))]
    fn to_text(&self, limit: usize) -> String {
        rdiff::render_text(&self.report, &self.left, &self.right, limit)
    }

    #[pyo3(signature = (limit = 10))]
    fn to_markdown(&self, limit: usize) -> String {
        rdiff::render_markdown(&self.report, &self.left, &self.right, limit)
    }

    /// Rendered as Markdown in Jupyter.
    fn _repr_markdown_(&self) -> String {
        self.to_markdown(10)
    }

    fn __bool__(&self) -> bool {
        self.report.ok
    }

    fn __repr__(&self) -> String {
        format!(
            "Diff({} vs {}: {})",
            self.left,
            self.right,
            rdiff::verdict(&self.report)
        )
    }
}

/// A run given as a `Run`, a manifest dict, or a reference string (`pipeline.toml#3`, `run.json`…).
fn manifest_of(obj: &Bound<'_, PyAny>) -> PyResult<(String, Manifest, Vec<Tolerance>)> {
    if let Ok(r) = obj.downcast::<PyRun>() {
        let r = r.get();
        let label = match r.id {
            Some(i) => format!("{}#{i}", r.pipeline),
            None => format!("{}#unrecorded", r.pipeline),
        };
        return Ok((label, r.manifest.clone(), vec![]));
    }
    if let Ok(s) = obj.extract::<PathBuf>() {
        let reference = s.to_string_lossy().into_owned();
        let r = rdiff::resolve(&reference).map_err(mf_err)?;
        return Ok((r.label, r.manifest, r.spec.map(|s| s.tolerances).unwrap_or_default()));
    }
    if obj.is_instance_of::<PyDict>() {
        let m = rdiff::manifest_from_value(to_value(obj, "manifest")?).map_err(mf_err)?;
        return Ok(("manifest".into(), m, vec![]));
    }
    Err(PyTypeError::new_err(
        "expected a Run, a manifest dict or a reference string",
    ))
}

/// Compares two runs. `left` is the reference, `right` the candidate: each a `Run`, a manifest
/// dict, or a reference (`pipeline.toml`, `pipeline.toml#ID`, `#previous`, `#fresh`, `run.json`).
/// `tolerances`: a list of dicts (`path`, `abs`, `rel`, `bp`, `ignore`) or a TOML file, applied
/// before the pipelines' own rules (candidate first).
#[pyfunction]
#[pyo3(signature = (left, right, tolerances = None))]
fn diff(left: &Bound<'_, PyAny>, right: &Bound<'_, PyAny>, tolerances: Option<&Bound<'_, PyAny>>) -> PyResult<PyDiff> {
    let (ll, lm, lrules) = manifest_of(left)?;
    let (rl, rm, rrules) = manifest_of(right)?;
    let mut rules: Vec<Tolerance> = match tolerances {
        None => vec![],
        Some(t) if t.is_none() => vec![],
        Some(t) => match t.extract::<PathBuf>() {
            Ok(p) if !t.is_instance_of::<PyList>() => rdiff::load_tolerances(Path::new(&p)).map_err(mf_err)?,
            _ => {
                let v: Vec<Tolerance> =
                    depythonize(t).map_err(|e| PyValueError::new_err(format!("tolerances: {e}")))?;
                for r in &v {
                    r.check().map_err(PyValueError::new_err)?;
                }
                v
            }
        },
    };
    rules.extend(rrules);
    rules.extend(lrules);
    Ok(PyDiff::new(mf_core::diff::diff(&lm, &rm, &rules), ll, rl))
}

/// Kinds of the built-in steps (VaR, backtests, PD validation, pricing, curves, portfolio risk).
#[pyfunction]
fn builtin_kinds() -> Vec<&'static str> {
    BuiltinStep::KINDS.to_vec()
}

/// Name and version of the engine, as recorded in every manifest.
#[pyfunction]
fn engine() -> String {
    mf_core::engine()
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPipeline>()?;
    m.add_class::<PyRun>()?;
    m.add_class::<PyDiff>()?;
    m.add_function(wrap_pyfunction!(diff, m)?)?;
    m.add_function(wrap_pyfunction!(builtin_kinds, m)?)?;
    m.add_function(wrap_pyfunction!(engine, m)?)?;
    m.add("ModelFlowError", m.py().get_type::<ModelFlowError>())?;
    m.add("StepError", m.py().get_type::<StepError>())?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
