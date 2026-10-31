"""The equity VaR desk of examples/var-equity, written with the Python SDK.

The pipeline is declared in Python; the EWMA model is a plain Python function executed in-process
(no subprocess, no JSON protocol), while the data loading, the historical VaR and the backtests
are built-in steps executed by the Rust engine. The script then:

1. runs and records the pipeline, and replays it;
2. checks with `modelflow.diff` that it gives exactly the same numbers as the TOML pipeline of
   examples/var-equity, whose EWMA model is an external process;
3. compares a challenger (Student-t quantile) with the champion, as a validator would.

    pip install maturin && maturin develop --release     # from the repository root
    python examples/python-sdk/var_desk.py
"""
import math
import pathlib
import sys

import modelflow as mf

HERE = pathlib.Path(__file__).resolve().parent
DATA = HERE.parent / "var-equity"

Z99 = 2.3263478740408408
T5_99 = 3.364930 * math.sqrt(3 / 5)  # Student-t (5 degrees of freedom) quantile, unit variance


def desk(quantile, version):
    p = mf.Pipeline("var-equity-desk-python", workdir=HERE)
    p.add("returns", "csv", version="1.0.0", path="../var-equity/data/returns.csv", column="ret")
    p.add("var_hist_99", "var_historical", version="1.0.0", deps=["returns"], params={"alpha": 0.99})
    p.add("backtest_hist", "kupiec_backtest", version="1.0.0", deps=["returns"], params={"alpha": 0.99, "window": 250})

    @p.step(name="ewma_model", version=version, params={"lam": 0.94, "q": quantile, "warmup": 30})
    def ewma(returns, lam, q, warmup):
        """RiskMetrics EWMA VaR: forecast for r[t] with the information up to t-1."""
        s2 = sum(x * x for x in returns[:warmup]) / warmup
        var = []
        for x in returns[warmup:]:
            var.append(q * math.sqrt(s2))
            s2 = lam * s2 + (1 - lam) * x * x
        return {"start": warmup, "var": var, "model": "ewma", "lambda": lam}

    p.add(
        "backtest_ewma",
        "backtest_forecasts",
        version="1.0.0",
        deps=["returns", "ewma_model"],
        params={"alpha": 0.99, "returns": "returns", "forecasts": "ewma_model"},
    )
    p.tolerance("returns*", ignore=True)
    return p


champion = desk(Z99, "0.1.0")
run = champion.run()
print(run)
assert champion.verify() == [], "the run must replay identically"

# Same model, two implementations: the TOML pipeline runs the EWMA model as an external process,
# which squares returns with `x ** 2` where this one uses `x * x`. An exact comparison sees the
# rounding differences (about 1e-17); a relative tolerance of 1e-12 accepts them.
toml_run = mf.load(DATA / "pipeline.toml").run(record=False)
exact = mf.diff(toml_run, run)
print("exact comparison:", exact.verdict)
same = mf.diff(toml_run, run, tolerances=[{"path": "*", "rel": 1e-12}])
print(same.to_text(limit=3))
if not same.ok:
    sys.exit("the Python pipeline does not reproduce the TOML pipeline")

challenger = desk(T5_99, "1.0.0-t5").run(record=False)
d = mf.diff(run, challenger)
print(d.to_text(limit=5))
for c in d.status_changes:
    print(f"verdict change: {c['path']}: {c['left']} -> {c['right']}")
champion.report(HERE / "report.html")
print("report written:", HERE / "report.html")
