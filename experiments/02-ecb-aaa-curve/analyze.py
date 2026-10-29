"""Runs and replays the pipeline for every date, then measures the gap between the ECB spot rates
and each reconstruction, in basis points of continuously compounded zero rate, per maturity segment.
Writes results/by_date.csv and results/summary.json."""
import csv, json, math, os, statistics, subprocess, sys

MF = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../target/release/mf"))
SEGMENTS = ("below_1y", "pillars", "between_pillars")


def mf(*args):
    return subprocess.run([MF, *args], capture_output=True, text=True)


def segment(t):
    return "below_1y" if t < 1 else "pillars" if abs(t - round(t)) < 1e-9 else "between_pillars"


rows, replays = [], 0
for d in sorted(os.listdir("data")):
    p = f"data/{d}/pipeline.toml"
    if not os.path.exists(f"data/{d}/.modelflow/runs.db") and mf("run", p).returncode not in (0, 1):
        sys.exit(f"{d}: run failed")
    v = mf("verify", p)
    replays += v.returncode == 0
    m = json.loads(mf("show", p).stdout)
    o = {s["name"]: s["output"] for s in m["steps"]}
    mat = {t["id"]: t["maturity"] for t in o["zero_coupons"]["trades"]}
    row = {"date": d, "replay": "identical" if v.returncode == 0 else "MISMATCH", "run_hash": m["run_hash"][:16],
           "rate_10y": o["ecb"]["spot_rates"]["10Y"]["rate"], "rate_3m": o["ecb"]["spot_rates"]["3M"]["rate"],
           "repricing_error": max(o["curve"]["max_repricing_error"], o["curve_naive"]["max_repricing_error"])}
    for check, tag in (("spot_check", "derived"), ("spot_check_naive", "naive")):
        gaps = {s: [] for s in SEGMENTS}
        for t in o[check]["trades"]:
            T = mat[t["id"]]
            gaps[segment(T)].append(abs(math.log(t["candidate"] / t["reference"]) / -T) * 1e4)
        for s in SEGMENTS:
            row[f"{tag}_{s}_max_bp"] = max(gaps[s])
            row[f"{tag}_{s}_median_bp"] = statistics.median(gaps[s])
        row[f"{tag}_status"] = o[check]["status"]
    rows.append(row)

os.makedirs("results", exist_ok=True)
with open("results/by_date.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0]))
    w.writeheader()
    w.writerows(rows)
summary = {"dates": len(rows), "identical_replays": replays,
           "max_repricing_error": max(r["repricing_error"] for r in rows),
           "rate_10y_range": [min(r["rate_10y"] for r in rows), max(r["rate_10y"] for r in rows)],
           "rate_3m_range": [min(r["rate_3m"] for r in rows), max(r["rate_3m"] for r in rows)]}
for tag in ("derived", "naive"):
    for s in SEGMENTS:
        summary[f"{tag}_{s}_max_bp"] = max(r[f"{tag}_{s}_max_bp"] for r in rows)
        summary[f"{tag}_{s}_median_bp"] = statistics.median(r[f"{tag}_{s}_median_bp"] for r in rows)
with open("results/summary.json", "w") as f:
    json.dump(summary, f, indent=1)
print(json.dumps(summary, indent=1))
