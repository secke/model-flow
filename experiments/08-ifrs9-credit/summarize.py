"""Tables of the latest run of ifrs9.toml.

    python3 summarize.py            # writes results/summary.md and results/summary.json
"""
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
MF = sys.argv[1] if len(sys.argv) > 1 else str(HERE / "../../target/release/mf")
run = json.loads(subprocess.run([MF, "show", str(HERE / "ifrs9.toml")], capture_output=True, text=True, check=True).stdout)
out = {s["name"]: s["output"] for s in run["steps"]}
status = {s["name"]: s["output"].get("status") for s in run["steps"] if isinstance(s["output"], dict)}
G = "ABCDEFG"
curves, lgd = out["pd_curves"]["curves"], out["pd_curves"]["lgd"]
ecl, conv, nodisc = out["ecl"], out["ecl_model_conventions"], out["ecl_no_discount"]


def pct(x, d=2):
    return f"{100 * x:.{d}f}%"


def m(x):
    return f"{x / 1e6:,.1f}"


lines = [f"# Lending Club IFRS 9 validation: run `{run['run_hash'][:16]}`", "",
         f"Test book: {ecl['facilities']:,} loans of the 2013-2015 vintages, {m(ecl['total']['exposure'])} million USD lent.", "",
         "## The model: PD curves and LGD by grade (2010-2012 vintages)", "",
         "| Grade | PD year 1 | Cumulative PD year 2 | Cumulative PD year 3 | LGD |", "|---|---:|---:|---:|---:|"]
for g in range(1, 8):
    c = curves[str(g)]
    lines.append(f"| {G[g - 1]} | {pct(c[0])} | {pct(c[1])} | {pct(c[2])} | {pct(lgd[str(g)], 1)} |")

rec = ecl["reconciliation"]
lines += ["", "## ECL recomputation (stage 1 at origination: 12-month ECL), million USD", "",
          "| Conventions | ECL | Against the model's figure |", "|---|---:|---:|",
          f"| The model's: amount lent, annual period, no discounting | {m(conv['total']['ecl'])} | {conv['reconciliation']['relative_difference']:+.1e} |",
          f"| Monthly instalments, no discounting | {m(nodisc['total']['ecl'])} | {nodisc['reconciliation']['relative_difference']:+.1%} |",
          f"| Monthly instalments, discounting at the loan rate (IFRS 9 5.5.17) | {m(ecl['total']['ecl'])} | {rec['relative_difference']:+.1%} |",
          "", f"Model's figure: {m(rec['reported'])}. Step status: `{rec['status']}`, {rec['facilities_outside_tolerance']:,} loans outside tolerance.",
          "", "## ECL against realized losses, million USD (losses discounted to the issue date at the loan rate)", "",
          "| Segment | Loans | 12-month ECL | Realized, first 12 months | Ratio | Lifetime ECL | Realized, lifetime | Ratio |",
          "|---|---:|---:|---:|---:|---:|---:|---:|"]
summary = {"run_hash": run["run_hash"], "status": status, "curves": curves, "lgd": lgd,
           "ecl": {"reported": rec["reported"], "recomputed": rec["recomputed"], "no_discount": nodisc["total"]["ecl"],
                   "model_conventions": conv["total"]["ecl"], "total": ecl["total"], "by": ecl["by"]}}


def ecl_row(label, r):
    return (f"| {label} | {r['facilities']:,} | {m(r['ecl_12m'])} | {m(r['realized_12m'])} | {r['realized_to_ecl_12m']:.2f} | "
            f"{m(r['ecl_lifetime'])} | {m(r['realized_lifetime'])} | {r['realized_to_ecl_lifetime']:.2f} |")


lines += [ecl_row(f"grade {G[r['segment'] - 1]}", r) for r in ecl["by"]["grade"]]
lines += [ecl_row(f"vintage {r['segment']}", r) for r in ecl["by"]["vintage"]]
lines.append(ecl_row("**all**", ecl["total"]))

lines += ["", "## PD calibration by grade (binomial test, one-sided)", "",
          "| Grade | Loans | 12-month PD | Observed | p-value | Lifetime PD | Observed | p-value |", "|---|---:|---:|---:|---:|---:|---:|---:|"]
for a, b in zip(out["pd_12m"]["grades"], out["pd_lifetime"]["grades"]):
    lines.append(f"| {G[a['grade'] - 1]} | {a['obligors']:,} | {pct(a['mean_pd'])} | {pct(a['observed_dr'])} | {a['p_value']:.1e} | "
                 f"{pct(b['mean_pd'])} | {pct(b['observed_dr'])} | {b['p_value']:.1e} |")
r = out["pd_ranking"]
lines += ["", f"Hosmer-Lemeshow p-value: {out['pd_12m']['hl_p_value']:.1e} (12 months), {out['pd_lifetime']['hl_p_value']:.1e} (lifetime). "
          f"Ranking of the lifetime PD: AUC {r['auc']:.3f}, Gini {r['gini']:.3f}, KS {r['ks']:.3f}."]
summary["pd"] = {"12m": out["pd_12m"], "lifetime": out["pd_lifetime"], "ranking": {k: r[k] for k in ("auc", "gini", "ks")}}

for key, title, label in [("lgd_by_grade", "grade", lambda s: G[s - 1]), ("lgd_by_default_year", "year of default", str),
                          ("lgd_completed_workouts", "year of default, workouts of 12 months or more", str)]:
    b = out[key]
    lines += ["", f"## LGD backtest by {title} (t-test of realized - estimated, H0: the LGD is not too low)", "",
              "| Segment | Defaults | Estimated | Realized | t | p-value | Status |", "|---|---:|---:|---:|---:|---:|---|"]
    for s in b["segments"] + [b["overall"]]:
        name = "**all**" if s["segment"] is None else label(s["segment"])
        lines.append(f"| {name} | {s['observations']:,} | {pct(s['mean_estimated'], 1)} | {pct(s['mean_realized'], 1)} | "
                     f"{s['t_statistic']:+.1f} | {s['p_value']:.1e} | {s['status']} |")
    lines += ["", f"Concordance of the estimated and realized LGDs: {b['concordance']:.3f}."
              + (f" Defaults left out, workout under 12 months at the data date: {b['excluded_incomplete']:,}." if b.get("excluded_incomplete") else "")]
    summary[key] = b

(HERE / "results").mkdir(exist_ok=True)
(HERE / "results/summary.md").write_text("\n".join(lines) + "\n")
(HERE / "results/summary.json").write_text(json.dumps(summary, indent=1) + "\n")
print("\n".join(lines))
