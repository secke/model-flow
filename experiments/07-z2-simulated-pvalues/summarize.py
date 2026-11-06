"""Fixed-threshold and simulated Z2 zones of the latest runs of ewma_models.toml and garch_models.toml.

    python3 summarize.py            # writes results/summary.md and results/summary.json
"""
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
MF = sys.argv[1] if len(sys.argv) > 1 else str(HERE / "../../target/release/mf")
PIPELINES = {
    "ewma_models.toml": ["ewma_normal", "ewma_t5", "ewma_t4", "ewma_t3", "fhs", "fhs_2y"],
    "garch_models.toml": ["ewma_normal", "ewma_fhs", "garch_normal", "gjr_normal", "gjr_t5", "garch_fhs", "gjr_fhs", "gjr_fhs_4y"],
}
result, lines = {}, []
def pv(p, n=100_000):
    """p-value; the smallest one, 1 / (n + 1), means that no simulated Z2 was as low."""
    if p <= 1 / (n + 1) + 1e-15:
        return "< 1/100,000"
    return f"{p:.1e}" if p < 1e-3 else f"{p:.4f}"


def fixed_zone(z2):
    """Zone of the fixed 250-day thresholds of Acerbi and Szekely, applied to the whole sample as before v0.20."""
    return "green" if z2 >= -0.70 else "yellow" if z2 >= -1.8 else "red"


for pipeline, models in PIPELINES.items():
    run = json.loads(subprocess.run([MF, "show", str(HERE / pipeline)], capture_output=True, text=True, check=True).stdout)
    out = {s["name"]: s["output"] for s in run["steps"]}
    rows = []
    for key in models:
        e = out[f"es_{key}"]
        s = e["simulated"]
        changed = [{"from": w["from"], "to": w["to"], "z2": w["z2"], "fixed": w["zone"], "simulated": w["simulated_zone"],
                    "p_value": w["p_value"]} for w in e["windows"] if w["zone"] != w["simulated_zone"]]
        rows.append({"model": key, "days": e["observations"], "z2": e["z2"], "fixed_zone": fixed_zone(e["z2"]),
                     "p_value": s["p_value"], "simulated_zone": s["zone"], "critical_5": s["critical_5"],
                     "critical_0_01": s["critical_0_01"],
                     "windows_fixed": [w["zone"] for w in e["windows"]],
                     "windows_simulated": [w["simulated_zone"] for w in e["windows"]],
                     "window_p_values": [w["p_value"] for w in e["windows"]], "changed_windows": changed})
    result[pipeline] = {"run_hash": run["run_hash"], "models": rows}
    lines += [f"## `{pipeline}` (run `{run['run_hash'][:16]}`, {rows[0]['days']} days)", "",
              "| Model | Z2 | Fixed zone | Simulated p | Simulated zone | Simulated 5% quantile (fixed: −0.70) | 0.01% quantile (fixed: −1.8) | Windows red / yellow / green, fixed | same, simulated |",
              "|---|---:|---|---:|---|---:|---:|---|---|"]
    for r in rows:
        c = lambda z: f"{z.count('red')} / {z.count('yellow')} / {z.count('green')}"
        lines.append(f"| {r['model']} | {r['z2']:+.2f} | {r['fixed_zone']} | {pv(r['p_value'])} | {r['simulated_zone']} | "
                     f"{r['critical_5']:+.2f} | {r['critical_0_01']:+.2f} | {c(r['windows_fixed'])} | {c(r['windows_simulated'])} |")
    lines += ["", "250-day windows whose zone changes:", "", "| Model | Window | Z2 | Fixed | Simulated (p) |", "|---|---|---:|---|---|"]
    for r in rows:
        for w in r["changed_windows"]:
            lines.append(f"| {r['model']} | {w['from']} → {w['to']} | {w['z2']:+.2f} | {w['fixed']} | {w['simulated']} ({pv(w['p_value'])}) |")
    lines.append("")
(HERE / "results").mkdir(exist_ok=True)
(HERE / "results/summary.md").write_text("\n".join(lines))
(HERE / "results/summary.json").write_text(json.dumps(result, indent=2) + "\n")
print("\n".join(lines))
