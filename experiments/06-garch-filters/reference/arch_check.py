"""Refits every GARCH / GJR fit recorded by the latest run of challengers.toml with the Python
`arch` package, on the same returns, and compares the log-likelihoods and the parameters.

    pip install arch==8.0.0
    python3 reference/arch_check.py          # writes results/reference_arch.md
"""
import json
import math
import pathlib
import subprocess
import sys

import numpy as np
from arch import arch_model

HERE = pathlib.Path(__file__).resolve().parent.parent
MF = sys.argv[1] if len(sys.argv) > 1 else str(HERE / "../../target/release/mf")
run = json.loads(subprocess.run([MF, "show", str(HERE / "challengers.toml")], capture_output=True, text=True, check=True).stdout)
out = {s["name"]: s["output"] for s in run["steps"]}
ret, days = out["returns"], out["days"]
WINDOW = {"garch_normal": None, "gjr_normal": None, "gjr_fhs_4y": 1000}

lines = [f"Run `{run['run_hash'][:16]}`: every fit refitted with arch {__import__('arch').__version__} on 100 r "
         "(zero mean, normal likelihood, default backcast, SLSQP with ftol 1e-12).", "",
         "| Model | Fit used from | Returns | quantparity log-lik. − arch | Largest parameter gap (ω × 1e4, α, γ, β) | α | γ | β | Persistence |",
         "|---|---|---:|---:|---:|---:|---:|---:|---:|"]
worst_ll, worst_p, n_fits = 0.0, 0.0, 0
for key, window in WINDOW.items():
    for f in out[key]["fits"]:
        t0, fit = f["from"], f["fit"]
        lo = 0 if window is None else max(0, t0 - window)
        y = 100 * np.array(ret[lo:t0])
        o = int(fit["model"] == "gjr")
        res = arch_model(y, mean="Zero", vol="GARCH", p=1, o=o, q=1, dist="normal", rescale=False).fit(
            disp="off", options={"ftol": 1e-12, "maxiter": 2000})
        a_ll = res.loglikelihood + len(y) * math.log(100)
        p = fit["params"]
        mine = [p["omega"] * 1e4, p["alpha"]] + ([p["gamma"]] if o else []) + [p["beta"]]
        gap = max(abs(a - b) for a, b in zip(mine, res.params.values))
        d_ll = fit["loglik"] - a_ll
        worst_ll, worst_p, n_fits = min(worst_ll, d_ll), max(worst_p, gap), n_fits + 1
        day = str(int(days[t0]))
        lines.append(f"| {key} | {day[:4]}-{day[4:6]}-{day[6:]} | {len(y)} | {d_ll:+.1e} | {gap:.1e} | "
                     f"{p['alpha']:.4f} | {p['gamma']:.4f} | {p['beta']:.4f} | {fit['persistence']:.6f} |")
lines += ["", f"{n_fits} fits. Lowest quantparity − arch log-likelihood: {worst_ll:+.1e}; largest parameter gap: {worst_p:.1e}."]
(HERE / "results").mkdir(exist_ok=True)
(HERE / "results/reference_arch.md").write_text("\n".join(lines) + "\n")
print("\n".join(lines))
