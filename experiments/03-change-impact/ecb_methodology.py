"""Experiment 03, part B: impact of the market-builder methodology change on the 58 ECB curves.

For every date of experiment 02: run the pipeline with version 1 of the market builder, switch it
to version 2, run again, and compare the two runs with `mf diff pipeline.toml` (previous vs latest).
Writes results/ecb_by_date.csv and results/ecb_summary.json from the JSON output of mf diff.
Requires the data of experiment 02 (python3 ../02-ecb-aaa-curve/fetch.py)."""
import csv, json, pathlib, shutil, subprocess, sys

HERE = pathlib.Path(__file__).resolve().parent
MF = str(HERE.parents[1] / "target" / "release" / "mf")
DATA = HERE.parent / "02-ecb-aaa-curve" / "data"
TEMPLATE = (HERE / "ecb_pipeline.toml").read_text()
VERSIONS = [("1.0.0", "ecb_par_yields"), ("2.0.0", "derived")]


def mf(*args, cwd):
    p = subprocess.run([MF, *args], cwd=cwd, capture_output=True, text=True)
    if p.returncode >= 2:
        sys.exit(f"mf {' '.join(args)} failed in {cwd}:\n{p.stderr}")
    return p


rows = []
dates = sorted(d.name for d in DATA.iterdir() if (d / "ecb.json").exists())
for date in dates:
    work = HERE / "work" / date
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    for version, convention in VERSIONS:
        text = TEMPLATE.replace("@DATE@", date).replace("@VERSION@", version).replace("@CONVENTION@", convention)
        (work / "pipeline.toml").write_text(text)
        mf("run", "pipeline.toml", cwd=work)
    out = mf("diff", "pipeline.toml", "--format", "json", cwd=work)
    d = json.loads(out.stdout)
    rep = d["diff"]
    steps = {s["name"]: s for s in rep["steps"]}
    fields = {f["field"]: f for s in rep["steps"] for f in s["by_field"]}
    zr = fields.get("curve.pillars[*].zero_rate", {})
    ts = fields.get("curve.term_structure[*].forward_rate", {})
    sc = {c["path"]: c for c in steps["spot_check"]["changes"]}
    failed = f'{sc["spot_check.failed"]["left"]} -> {sc["spot_check.failed"]["right"]}' if "spot_check.failed" in sc else "unchanged"
    verdict = {c["path"]: f'{c["left"]} -> {c["right"]}' for c in rep["status_changes"]}
    rows.append({
        "date": date,
        "exit_code": out.returncode,
        "origins": "; ".join(rep["origins"]),
        "values_compared": rep["leaves_compared"],
        "changes": rep["changes"],
        "beyond_tolerance": rep["breaches"],
        "curve_beyond_tolerance": steps["curve"]["breaches"],
        "pillar_zero_rates_changed": zr.get("changes", 0),
        "pillar_zero_rates_beyond_tol": zr.get("breaches", 0),
        "max_pillar_zero_rate_change_bp": round((zr.get("max_abs_delta") or 0.0) * 1e4, 4),
        "mean_pillar_zero_rate_change_bp": round((zr.get("mean_delta") or 0.0) * 1e4, 4),
        "max_forward_change_bp": round((ts.get("max_abs_delta") or 0.0) * 1e4, 4),
        "spot_check_failed": failed,
        "spot_check_status": verdict.get("spot_check.status", "unchanged"),
    })
    print(date, rows[-1]["beyond_tolerance"], f'{rows[-1]["max_pillar_zero_rate_change_bp"]:.2f} bp', rows[-1]["spot_check_status"])

res = HERE / "results"
res.mkdir(exist_ok=True)
with open(res / "ecb_by_date.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0]))
    w.writeheader()
    w.writerows(rows)
mx = max(rows, key=lambda r: r["max_pillar_zero_rate_change_bp"])
summary = {
    "dates": len(rows),
    "dates_failing_the_gate": sum(r["exit_code"] == 1 for r in rows),
    "values_compared": sum(r["values_compared"] for r in rows),
    "changes": sum(r["changes"] for r in rows),
    "beyond_tolerance": sum(r["beyond_tolerance"] for r in rows),
    "pillar_zero_rates_beyond_tol": sum(r["pillar_zero_rates_beyond_tol"] for r in rows),
    "pillar_zero_rates_compared": 30 * len(rows),
    "max_pillar_zero_rate_change_bp": mx["max_pillar_zero_rate_change_bp"],
    "max_on": mx["date"],
    "spot_check_status_changes": sorted({r["spot_check_status"] for r in rows}),
    "spot_check_failed_left": sum(int(r["spot_check_failed"].split(" -> ")[0]) for r in rows if "->" in r["spot_check_failed"]),
    "spot_check_failed_right": sum(int(r["spot_check_failed"].split(" -> ")[1]) for r in rows if "->" in r["spot_check_failed"]),
    "origins": sorted({r["origins"] for r in rows}),
}
(res / "ecb_summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, indent=2))
