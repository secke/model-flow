"""Summary of the latest recorded run of challengers.toml: one row per model.

    python3 summarize.py            # reads `mf show challengers.toml`, writes results/summary.md and .json
"""
import json
import math
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
MF = sys.argv[1] if len(sys.argv) > 1 else str(HERE / "../../target/release/mf")
run = json.loads(subprocess.run([MF, "show", str(HERE / "challengers.toml")], capture_output=True, text=True, check=True).stdout)
out = {s["name"]: s["output"] for s in run["steps"]}
MODELS = [
    ("ewma_normal", "EWMA, normal (champion)"),
    ("ewma_t5", "EWMA, Student-t 5"),
    ("ewma_t4", "EWMA, Student-t 4"),
    ("ewma_t3", "EWMA, Student-t 3"),
    ("fhs", "Filtered HS, all residuals"),
    ("fhs_2y", "Filtered HS, latest 500"),
]

# Kurtosis of the standardized returns r_t / sigma_t: the tail the volatility filter leaves.
ret = out["returns"]
f0 = out["ewma_normal"]
z = [ret[f0["start"] + j] / s for j, s in enumerate(f0["sigma"])]
m2 = sum(x * x for x in z) / len(z)
kurt = sum(x ** 4 for x in z) / len(z) / m2 ** 2 - 3
nu_mom = 4 + 6 / kurt  # Student-t with the same excess kurtosis

rows = []
for key, label in MODELS:
    v, e = out[f"var_{key}"], out[f"es_{key}"]
    zones = [w["zone"] for w in v["basel_windows"]]
    es_zones = [w["zone"] for w in e["windows"]]
    rows.append({
        "model": key, "label": label,
        "observations": v["observations"], "exceptions": v["exceptions"],
        "rate": v["exceptions"] / v["observations"],
        "kupiec_p": v["kupiec"]["p_value"], "independence_p": v["christoffersen"]["p_ind"],
        "basel": {z_: zones.count(z_) for z_ in ("red", "yellow", "green")},
        "var_status": v["status"],
        "es_exceptions": e["exceptions"], "es_expected": e["expected_exceptions"],
        "severity": e["severity"], "z2": e["z2"], "es_zone": e["zone"],
        "es_windows": {z_: es_zones.count(z_) for z_ in ("red", "yellow", "green")},
        "latest_var_zone": v["latest_basel_window"]["zone"], "latest_es_zone": e["latest_window"]["zone"],
        "mean_var": sum(out[key]["var"][280 - out[key]["start"]:]) / v["observations"],
        "mean_es": sum(out[key]["es"][280 - out[key]["start"]:]) / e["observations"],
    })

lines = [
    f"Run `{run['run_hash'][:16]}`, {rows[0]['observations']} days evaluated, every model on the same days.",
    f"Standardized returns of the EWMA filter: excess kurtosis {kurt:.2f}, i.e. a Student-t with {nu_mom:.1f} degrees of freedom.",
    "",
    "| Model | VaR 99% exceptions | Rate | Kupiec p | Christoffersen p (independence) | Basel windows red / yellow / green | ES 97.5% exceptions (expected) | Severity | Z2 | ES windows red / yellow / green | Mean VaR | Mean ES |",
    "|---|---:|---:|---:|---:|---|---:|---:|---:|---|---:|---:|",
]
for r in rows:
    b, w = r["basel"], r["es_windows"]
    lines.append(
        f"| {r['label']} | {r['exceptions']} | {r['rate']:.2%} | {r['kupiec_p']:.1e} | {r['independence_p']:.2f} | "
        f"{b['red']} / {b['yellow']} / {b['green']} | {r['es_exceptions']} ({r['es_expected']:.1f}) | {r['severity']:.2f} | "
        f"{r['z2']:+.2f} ({r['es_zone']}) | {w['red']} / {w['yellow']} / {w['green']} | {r['mean_var']:.2%} | {r['mean_es']:.2%} |"
    )
# Consecutive 250-day windows (the last one ends on the latest day): VaR zone and ES zone per model.
win = out["var_ewma_normal"]["basel_windows"]
lines += ["", "Basel zone of the 99% VaR / Z2 zone of the 97.5% ES, by 250-day window:", "",
          "| Window | " + " | ".join(label for _, label in MODELS) + " |", "|---|" + "---|" * len(MODELS)]
for i, w in enumerate(win):
    cells = []
    for key, _ in MODELS:
        vw, ew = out[f"var_{key}"]["basel_windows"][i], out[f"es_{key}"]["windows"][i]
        cells.append(f"{vw['zone']} ({vw['exceptions']}) / {ew['zone']} ({ew['z2']:+.2f})")
    lines.append(f"| {w['from']} → {w['to']} | " + " | ".join(cells) + " |")
(HERE / "results").mkdir(exist_ok=True)
(HERE / "results/summary.md").write_text("\n".join(lines) + "\n")
(HERE / "results/summary.json").write_text(json.dumps({"run_hash": run["run_hash"], "kurtosis": kurt, "nu_moments": nu_mom, "models": rows}, indent=2) + "\n")
print("\n".join(lines))
