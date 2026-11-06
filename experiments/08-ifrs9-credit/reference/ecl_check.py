"""Recomputes the ECL of the run with pandas and numpy from data/loans.csv, without ModelFlow,
quantparity or the pipeline's scripts: default dates and LGDs, PD curves and LGDs by grade on the
2010-2012 vintages, monthly hazard constant within each year, instalments and discounting at the
loan rate. Writes results/reference_ecl.md.

    pip install numpy pandas && python3 reference/ecl_check.py
"""
import json
import pathlib
import subprocess
import sys

import numpy as np
import pandas as pd

HERE = pathlib.Path(__file__).resolve().parent.parent
MF = sys.argv[1] if len(sys.argv) > 1 else str(HERE / "../../target/release/mf")
loans = pd.read_csv(HERE / "data/loans.csv")
last = loans.last_payment_month.where(loans.last_payment_month >= 0, loans.issue_month)
loans["default"] = loans.defaulted
loans["default_age"] = np.where(loans.defaulted == 1, last - loans.issue_month + 4, 0)
ead = loans.funded - loans.total_rec_prncp
loans["lgd_valid"] = ((loans.defaulted == 1) & (ead >= 1)).astype(int)
loans["lgd_realized"] = (1 - (loans.recoveries - loans.collection_recovery_fee) / ead).clip(0, 1).where(loans.lgd_valid == 1, 0.0)
dev = loans[loans.vintage <= 2012].reset_index(drop=True)
test = loans[loans.vintage > 2012].reset_index(drop=True)

year = np.minimum((dev.default_age - 1) // 12, 2)
cum = {}
for g, d in dev.groupby("grade"):
    hits = [(d.default.astype(bool) & (year[d.index] <= y)).sum() for y in range(3)]
    cum[g] = np.array(hits) / len(d)
lgd = dev[dev.lgd_valid == 1].groupby("grade").lgd_realized.mean()

k = np.arange(36)
t = (k + 1) / 12
ecl_12m, ecl_life = np.zeros(len(test)), np.zeros(len(test))
for g, d in test.groupby("grade"):
    s = np.concatenate([[1.0], 1 - cum[g]])
    y = np.floor(t).astype(int)
    surv = np.where(y < 3, s[np.minimum(y, 3)] * (s[np.minimum(y + 1, 3)] / s[np.minimum(y, 3)]) ** (t - y), s[3])
    marginal = np.diff(np.concatenate([[0.0], 1 - surv]))
    r = (1 + d.int_rate.to_numpy()[:, None]) ** (1 / 12) - 1
    ead = d.funded.to_numpy()[:, None] * (1 - (1 + r) ** -(36 - k)) / (1 - (1 + r) ** -36)
    df = (1 + d.int_rate.to_numpy()[:, None]) ** -t
    loss = marginal * lgd[g] * ead * df
    ecl_12m[d.index], ecl_life[d.index] = loss[:, :12].sum(axis=1), loss.sum(axis=1)

run = json.loads(subprocess.run([MF, "show", str(HERE / "ifrs9.toml")], capture_output=True, text=True, check=True).stdout)
e = {s["name"]: s["output"] for s in run["steps"]}["ecl"]
rows = [("all", e["total"]["ecl_12m"], ecl_12m.sum(), e["total"]["ecl_lifetime"], ecl_life.sum())]
for r in e["by"]["grade"]:
    m = (test.grade == r["segment"]).to_numpy()
    rows.append(("ABCDEFG"[r["segment"] - 1], r["ecl_12m"], ecl_12m[m].sum(), r["ecl_lifetime"], ecl_life[m].sum()))
lines = ["# ECL of the run against an independent numpy computation", "",
         f"Run `{run['run_hash'][:16]}`, {len(test):,} loans.", "",
         "| Grade | 12-month ECL, ModelFlow | numpy | Relative gap | Lifetime ECL, ModelFlow | numpy | Relative gap |",
         "|---|---:|---:|---:|---:|---:|---:|"]
worst = 0.0
for g, a, b, c, d in rows:
    worst = max(worst, abs(a / b - 1), abs(c / d - 1))
    lines.append(f"| {g} | {a:,.2f} | {b:,.2f} | {a / b - 1:.1e} | {c:,.2f} | {d:,.2f} | {c / d - 1:.1e} |")
lines += ["", f"Largest relative gap: {worst:.1e}."]
(HERE / "results/reference_ecl.md").write_text("\n".join(lines) + "\n")
print("\n".join(lines))
