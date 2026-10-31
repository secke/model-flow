"""Tests of the Python SDK (run with `pytest python/tests` after `maturin develop`)."""
import json
import math
import pathlib
import shutil

import numpy as np
import pandas as pd
import pytest

import modelflow as mf

ROOT = pathlib.Path(__file__).resolve().parents[2]


def returns_csv(path, n=600, shock=None):
    rows = ["day,ret"]
    for i in range(n):
        r = 0.01 * math.sin(i * 1.7) * (1.5 if i % 97 == 0 else 1.0)
        if shock is not None and i == shock:
            r = -0.2
        rows.append(f"{20200101 + i},{r:.6f}")
    path.write_text("\n".join(rows) + "\n")


def desk(tmp_path, lam=0.94, body=None):
    returns_csv(tmp_path / "returns.csv")
    p = mf.Pipeline("desk", workdir=tmp_path)
    p.add("returns", "csv", path="returns.csv", column="ret")
    p.add("var_99", "var_historical", deps=["returns"], params={"alpha": 0.99})

    def ewma(returns, lam):
        r = np.asarray(returns)
        s2 = np.mean(r[:30] ** 2)
        out = []
        for x in r[30:]:
            out.append(2.326 * np.sqrt(s2))
            s2 = lam * s2 + (1 - lam) * x * x
        return {"start": 30, "var": np.array(out)}

    p.step(body or ewma, name="ewma", version="1.0.0", params={"lam": lam})
    p.add(
        "backtest",
        "backtest_forecasts",
        deps=["returns", "ewma"],
        params={"alpha": 0.99, "returns": "returns", "forecasts": "ewma"},
    )
    p.tolerance("returns*", ignore=True)
    return p


def test_python_pipeline_runs_records_and_replays(tmp_path):
    p = desk(tmp_path)
    assert p.steps == ["returns", "var_99", "backtest", "ewma"]
    run = p.run()
    assert run.id == 1 and len(run.run_hash) == 64
    assert run.engine == mf.engine()
    assert run.status in ("pass", "warn", "fail")
    out = run.outputs
    assert out["ewma"]["start"] == 30 and len(out["ewma"]["var"]) == 570
    assert isinstance(out["ewma"]["var"][0], float)
    assert out["backtest"]["observations"] == 570
    assert p.verify() == []
    assert [r["id"] for r in p.runs()] == [1]
    assert p.get().run_hash == run.run_hash
    # Same pipeline, same inputs: identical run
    again = p.run(record=False)
    assert again.id is None and again.run_hash == run.run_hash


def test_parameter_change_is_detected_and_traced(tmp_path):
    p = desk(tmp_path)
    p.run()
    p2 = desk(tmp_path, lam=0.97)
    # Replaying run 1 with other parameters: a model change, not a reproducibility failure
    mismatches = {(m["step"], m["reason"]) for m in p2.verify()}
    assert ("ewma", "code or parameters changed without a version bump") in mismatches
    p2.run()
    d = p2.diff()
    assert not d.ok and d.left == "desk#1" and d.right == "desk#2"
    assert d.origins == ["ewma (code or parameters changed without a version bump)"]
    summary = {s["step"]: s for s in d.summary()}
    assert summary["var_99"]["state"] == "identical"
    assert summary["backtest"]["cause"] == "upstream steps changed"
    changes = pd.DataFrame(d.changes())
    assert "ewma" in set(changes["step"])
    assert (changes[changes.field == "ewma.var[*]"]["delta"] != 0).all()
    assert "origin" in d.to_text() and d.to_markdown().startswith("### ModelFlow diff — FAIL")


def test_source_change_is_detected(tmp_path):
    p = desk(tmp_path)
    run = p.run()

    def ewma(returns, lam):  # same name, different code
        return {"start": 30, "var": [0.02] * (len(returns) - 30)}

    p2 = desk(tmp_path, body=ewma)
    assert any(m["step"] == "ewma" and "code" in m["reason"] for m in p2.verify(run.id))


def test_diff_with_tolerances_and_references(tmp_path):
    p = desk(tmp_path)
    a = p.run()
    b = desk(tmp_path, lam=0.9400001).run()
    assert not mf.diff(a, b)
    loose = mf.diff(a, b, tolerances=[{"path": "ewma.var*", "rel": 1e-4}, {"path": "*", "abs": 1e-3}])
    assert loose.ok, loose.to_text()
    # References: manifest files and recorded runs, as with the command line
    a.save(tmp_path / "reference.json")
    d = mf.diff(str(tmp_path / "reference.json"), b)
    assert not d.ok and d.left.endswith("reference.json")
    toml = tmp_path / "tol.toml"
    toml.write_text('[[tolerance]]\npath = "*"\nrel = 1e-3\n')
    assert mf.diff(a.manifest, b, tolerances=str(toml)).ok
    assert mf.diff(a, a).identical


def test_pandas_and_numpy_outputs(tmp_path):
    p = mf.Pipeline("frames", workdir=tmp_path)

    @p.step(deps=[])
    def table():
        return pd.DataFrame({"id": ["T1", "T2"], "npv": np.array([1.5, -2.0]), "qty": np.int64(3)})

    @p.step
    def total(table):
        return float(pd.DataFrame(table)["npv"].sum())

    out = p.run().outputs
    assert out["table"] == {"id": ["T1", "T2"], "npv": [1.5, -2.0], "qty": [3, 3]}
    assert out["total"] == -0.5


def test_errors(tmp_path):
    p = mf.Pipeline("errors", workdir=tmp_path)

    @p.step(deps=[])
    def broken():
        raise ValueError("bad input data")

    with pytest.raises(mf.StepError, match="bad input data"):
        p.run()
    with pytest.raises(ValueError, match="duplicate step"):
        p.add("broken", "json", path="x.json")
    with pytest.raises(ValueError, match="unknown type"):
        p.add("x", "no_such_kind")
    with pytest.raises(ValueError):
        p.tolerance("*", abs=1.0, bp=1.0)

    q = mf.Pipeline("nan", workdir=tmp_path)
    q.step(lambda: {"x": float("nan")}, name="nan", deps=[])
    with pytest.raises(mf.StepError, match="non-finite"):
        q.run()


def test_toml_pipeline_and_command_line_interop(tmp_path):
    work = tmp_path / "var-equity"
    shutil.copytree(ROOT / "examples" / "var-equity", work, ignore=shutil.ignore_patterns(".modelflow"))
    p = mf.load(work / "pipeline.toml")
    assert p.name == "var-equity-desk" and "ewma_model" in p.steps
    run = p.run()
    assert p.verify() == []
    report = p.report(tmp_path / "report.html")
    assert "var-equity-desk" in pathlib.Path(report).read_text()
    # The same history is read by the command line's references
    d = mf.diff(f"{work / 'pipeline.toml'}#{run.id}", f"{work / 'pipeline.toml'}#fresh")
    assert d.ok and d.changes() == []
    manifest = json.loads(json.dumps(run.manifest))
    assert manifest["run_hash"] == run.run_hash
