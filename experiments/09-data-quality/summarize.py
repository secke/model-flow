"""Tables of the latest run of quality.toml.

    python3 summarize.py            # writes results/summary.md and results/summary.json
"""
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
MF = sys.argv[1] if len(sys.argv) > 1 else str(HERE / "../../target/release/mf")
run = json.loads(subprocess.run([MF, "show", str(HERE / "quality.toml")], capture_output=True, text=True, check=True).stdout)

lines = [f"Run `{run['run_hash'][:16]}`, {run['engine']}.", ""]
summary = {"run_hash": run["run_hash"], "engine": run["engine"], "steps": {}}
for s in run["steps"]:
    o = s["output"]
    src = o["source"]
    lines += [f"## {s['name']}: {src['file']}, {src['rows']:,} rows, status {o['status']}", "",
              f"SHA-256 `{src['sha256']}`", "",
              "| Rule | Rows | Check | Count | Rate | Limit | First rows | Status |", "|---|---:|---|---:|---:|---:|---|---|"]
    rules = []
    for r in o["rules"]:
        rows = f"{r['rows']:,}" if "rows" in r else ""
        for c in r.get("checks", []):
            first = ", ".join(map(str, c.get("rows", []))) + (", …" if c["count"] > len(c.get("rows", [])) else "")
            lines.append(f"| {r['rule']} | {rows} | {c['check']} | {c['count']:,} | {100 * c['rate']:.4f}% | "
                         f"{100 * c['limit']:.2f}% | {first} | {c['status']} |")
        rules.append({"rule": r["rule"], "status": r["status"],
                      "failed": {c["check"]: c["count"] for c in r.get("checks", []) if c["count"]}})
    lines.append("")
    summary["steps"][s["name"]] = {"rows": src["rows"], "sha256": src["sha256"], "status": o["status"], "rules": rules}

(HERE / "results").mkdir(exist_ok=True)
(HERE / "results/summary.md").write_text("\n".join(lines))
(HERE / "results/summary.json").write_text(json.dumps(summary, indent=1) + "\n")
print("\n".join(lines))
