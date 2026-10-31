"""ModelFlow: reproducible model validation, from Python.

The engine is written in Rust: the DAG, the SHA-256 fingerprints, the built-in steps (VaR,
backtests, PD validation, pricing against an independent reference, curves, portfolio risk), the
history, the replay and the diff. Python functions can be steps of a pipeline; they are
fingerprinted with their source code and parameters, like any other model.

    import modelflow as mf

    p = mf.Pipeline("desk-var")
    p.add("returns", "csv", path="data/returns.csv", column="ret")
    p.add("var_99", "var_historical", deps=["returns"], params={"alpha": 0.99})

    @p.step(version="1.0.0", params={"lam": 0.94})
    def ewma(returns, lam):
        ...
        return {"start": 30, "var": forecasts}

    run = p.run()                 # executed and recorded (.modelflow/runs.db)
    p.verify()                    # [] : replayed identically
    mf.diff(reference, run)       # what changed, where it started, beyond which tolerance

Dated pipelines (`${as_of}` in a data path) are monitored over time:

    p.run_dates("all")            # one run per as-of date, in parallel
    pandas.DataFrame(p.series(["backtest.latest_basel_window.zone", "drift.psi"]))
"""

import inspect
import textwrap

from . import _core
from ._core import Diff, ModelFlowError, Run, StepError, builtin_kinds, diff, engine

__version__ = _core.__version__
__all__ = [
    "Pipeline",
    "Run",
    "Diff",
    "diff",
    "load",
    "builtin_kinds",
    "engine",
    "ModelFlowError",
    "StepError",
]


def _source(func):
    """Source code of a function, dedented; its bytecode when the source is not available."""
    try:
        return textwrap.dedent(inspect.getsource(func))
    except (OSError, TypeError):
        code = getattr(func, "__code__", None)
        if code is None:
            raise TypeError(f"{func!r}: cannot fingerprint a step without source or bytecode")
        return repr((code.co_code, code.co_consts, code.co_names))


class Pipeline(_core.Pipeline):
    """A pipeline of steps executed by the ModelFlow engine.

    `Pipeline(name, workdir=".", model=None)` starts an empty pipeline; `Pipeline.load(path)`
    reads a TOML pipeline. Steps are added with `add` (data, built-in checks, external processes,
    as in TOML) and with the `step` decorator (Python functions).
    """

    def step(self, func=None, *, name=None, version="1", deps=None, params=None):
        """Registers a function as a step.

        Each dependency's output is passed as a keyword argument named after the dependency, and
        each entry of `params` as a keyword argument. By default the dependencies are the
        function's parameters that are not in `params`. The function returns JSON-compatible data:
        numbers, strings, lists, dicts, NumPy arrays or pandas objects.

        A parameter named `as_of` receives the run's as-of date (`run(as_of=...)`, `run_dates`).

        The step's fingerprint covers the function's source code and `params`: changing either
        without changing `version` is reported by the replay and by the diff.
        """
        params = dict(params or {})

        def register(f):
            sig = inspect.signature(f)
            wants_as_of = "as_of" in sig.parameters
            d = deps
            if d is None:
                d = [
                    n
                    for n, p in sig.parameters.items()
                    if n not in params
                    and n != "as_of"
                    and p.kind in (p.POSITIONAL_OR_KEYWORD, p.KEYWORD_ONLY)
                ]
            self.add_function(f, name or f.__name__, version, list(d), params, _source(f), wants_as_of)
            return f

        return register(func) if func is not None else register


def load(path):
    """Loads a TOML pipeline (`Pipeline.load`)."""
    return Pipeline.load(path)
