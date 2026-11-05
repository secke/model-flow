"""Summary of the latest recorded run of challengers.toml: one row per model.

    python3 summarize.py            # reads `mf show challengers.toml`, writes results/summary.md and .json
"""
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
MF = sys.argv[1] if len(sys.argv) > 1 else str(HERE / "../../target/release/mf")
run = json.loads(subprocess.run([MF, "show", str(HERE / "challengers.toml")], capture_output=True, text=True, check=True).stdout)
out = {s["name"]: s["output"] for s in run["steps"]}
FROM = 750
MODELS = [
    ("ewma_normal", "EWMA, normal (champion)"),
    ("ewma_fhs", "EWMA, filtered HS (experiment 05)"),
    ("garch_normal", "GARCH, normal"),
    ("gjr_normal", "GJR, normal"),
    ("gjr_t5", "GJR, Student-t 5"),
    ("garch_fhs", "GARCH, filtered HS"),
    ("gjr_fhs", "GJR, filtered HS"),
    ("gjr_fhs_4y", "GJR, filtered HS, fit on latest 1,000"),
]
ret = out["returns"]


def moments(key):
    """Standardized returns r_t / sigma_t on the evaluated days: std, excess kurtosis, 1% and 99% quantiles."""
    f = out[key]
    z = sorted(ret[t] / f["sigma"][t - f["start"]] for t in range(FROM, len(ret)))
    n = len(z)
    m2 = sum(x * x for x in z) / n
    return {"std": m2 ** 0.5, "kurtosis": sum(x ** 4 for x in z) / n / m2 ** 2 - 3,
            "q01": z[int(0.01 * n)], "q99": z[int(0.99 * n)]}


rows = []
for key, label in MODELS:
    v, e, f = out[f"var_{key}"], out[f"es_{key}"], out[key]
    zones = [w["zone"] for w in v["basel_windows"]]
    es_zones = [w["zone"] for w in e["windows"]]
    exc = v["exception_list"]
    pairs = [(a["date"], b["date"]) for a, b in zip(exc, exc[1:]) if b["index"] == a["index"] + 1]
    rows.append({
        "model": key, "label": label,
        "observations": v["observations"], "exceptions": v["exceptions"],
        "expected": v["observations"] * 0.01, "rate": v["exceptions"] / v["observations"],
        "kupiec_p": v["kupiec"]["p_value"], "independence_p": v["christoffersen"]["p_ind"],
        "consecutive": pairs,
        "basel": {z_: zones.count(z_) for z_ in ("red", "yellow", "green")},
        "es_exceptions": e["exceptions"], "es_expected": e["expected_exceptions"],
        "severity": e["severity"], "z2": e["z2"], "es_zone": e["zone"],
        "es_windows": {z_: es_zones.count(z_) for z_ in ("red", "yellow", "green")},
        "mean_var": sum(f["var"][FROM - f["start"]:]) / v["observations"],
        "mean_es": sum(f["es"][FROM - f["start"]:]) / e["observations"],
        "residuals": moments(key),
        "exception_dates": [x["date"] for x in exc],
        "persistence_check": f.get("persistence_check"),
    })

r0 = rows[0]
lines = [
    f"Run `{run['run_hash'][:16]}`, {r0['observations']} days evaluated ({out['var_ewma_normal']['basel_windows'][0]['from']} onwards), "
    "every model on the same days. Expected exceptions: "
    f"{r0['expected']:.1f} at 99%, {out['es_ewma_normal']['expected_exceptions']:.1f} at 97.5%.",
    "",
    "| Model | VaR 99% exceptions | Kupiec p | Christoffersen p (independence) | Consecutive exception days | Basel windows red / yellow / green | ES 97.5% exceptions | Severity | Z2 | ES windows red / yellow / green | Mean VaR | Mean ES |",
    "|---|---:|---:|---:|---|---|---:|---:|---:|---|---:|---:|",
]
for r in rows:
    b, w = r["basel"], r["es_windows"]
    pairs = ", ".join(f"{a} / {b_[5:]}" for a, b_ in r["consecutive"]) or "none"
    lines.append(
        f"| {r['label']} | {r['exceptions']} ({r['rate']:.2%}) | {r['kupiec_p']:.1e} | {r['independence_p']:.2f} | {pairs} | "
        f"{b['red']} / {b['yellow']} / {b['green']} | {r['es_exceptions']} | {r['severity']:.2f} | "
        f"{r['z2']:+.2f} ({r['es_zone']}) | {w['red']} / {w['yellow']} / {w['green']} | {r['mean_var']:.2%} | {r['mean_es']:.2%} |"
    )
lines += ["", "Persistence check of the GARCH model steps (warn at α + γ/2 + β ≥ 0.999):", "",
          "| Model | Fits | Highest persistence | Fits at the bound | Status |", "|---|---:|---:|---|---|"]
for r in rows:
    c = r["persistence_check"]
    if c:
        at = ", ".join(f"{b['date']} ({b['persistence']:.6f})" for b in c["at_bound"]) or "none"
        lines.append(f"| {r['label']} | {c['fits']} | {c['highest']:.6f} | {at} | {c['status']} |")
lines += ["", "Standardized returns r_t / σ_t on the evaluated days (what the volatility filter leaves to the distribution):", "",
          "| Filter | Std | Excess kurtosis | 1% quantile | 99% quantile |", "|---|---:|---:|---:|---:|"]
for key, label in [("ewma_normal", "EWMA"), ("garch_normal", "GARCH"), ("gjr_normal", "GJR"), ("gjr_fhs_4y", "GJR, fit on latest 1,000")]:
    m = moments(key)
    lines.append(f"| {label} | {m['std']:.3f} | {m['kurtosis']:.2f} | {m['q01']:+.2f} | {m['q99']:+.2f} |")
win = out["var_ewma_normal"]["basel_windows"]
lines += ["", "Basel zone of the 99% VaR (exceptions) / Z2 zone of the 97.5% ES, by 250-day window:", "",
          "| Window | " + " | ".join(label for _, label in MODELS) + " |", "|---|" + "---|" * len(MODELS)]
for i, w in enumerate(win):
    cells = []
    for key, _ in MODELS:
        vw, ew = out[f"var_{key}"]["basel_windows"][i], out[f"es_{key}"]["windows"][i]
        cells.append(f"{vw['zone']} ({vw['exceptions']}) / {ew['zone']} ({ew['z2']:+.2f})")
    lines.append(f"| {w['from']} → {w['to']} | " + " | ".join(cells) + " |")
# Exception days of each model, side by side
days = sorted({d for r in rows for d in r["exception_dates"]})
lines += ["", "99% VaR exception days (x = exception):", "", "| Day | Return | " + " | ".join(k for k, _ in MODELS) + " |",
          "|---|---:|" + "---|" * len(MODELS)]
idx = {x["date"]: x["return"] for r in MODELS for x in out[f"var_{r[0]}"]["exception_list"]}
for d in days:
    lines.append(f"| {d} | {idx[d]:+.2%} | " + " | ".join("x" if d in r["exception_dates"] else "" for r in rows) + " |")
(HERE / "results").mkdir(exist_ok=True)
(HERE / "results/summary.md").write_text("\n".join(lines) + "\n")
(HERE / "results/summary.json").write_text(json.dumps({"run_hash": run["run_hash"], "from": FROM, "models": rows}, indent=2) + "\n")
print("\n".join(lines))
