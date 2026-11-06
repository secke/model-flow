"""The IFRS 9 model under validation: a lender's simple ECL model for 36-month consumer loans.

PD: cumulative default rate by grade and loan age (years 1 to 3), on the development vintages.
LGD: mean realized LGD by grade on the development defaults. ECL reported at origination
(stage 1): 12-month PD x LGD x amount lent, with no amortization and no discounting.

Development vintages: `params.dev` (first and last year), the later ones are the test book.
Reads {"inputs": {"outcomes": table of outcomes.py}, "params": {"output": ..., "dev": [y0, y1]}}
on stdin and writes, for `output` = "curves", the PD curves by grade; "book", the test loans at origination with the
model's parameters and the realized outcomes; "defaults", the test defaults with their LGDs.
The realized losses of the book, LGD x EAD at default, are discounted to the issue date at the
loan rate, as the ECL is.
"""
import json
import sys

doc = json.load(sys.stdin)
loans, output = doc["inputs"]["outcomes"], doc["params"]["output"]
y0, y1 = doc["params"]["dev"]


def subset(keep):
    rows = [i for i, v in enumerate(loans["vintage"]) if keep(v)]
    return {k: [col[i] for i in rows] for k, col in loans.items()}


dev, test = subset(lambda v: y0 <= v <= y1), subset(lambda v: v > y1)
grades = range(1, 8)

loans = {g: 0 for g in grades}
by_year = {g: [0, 0, 0] for g in grades}
lgd_sum, lgd_n = {g: 0.0 for g in grades}, {g: 0 for g in grades}
for i, g in enumerate(dev["grade"]):
    g = int(g)
    loans[g] += 1
    if dev["default"][i]:
        by_year[g][min(int((dev["default_age"][i] - 1) // 12), 2)] += 1
        if dev["lgd_valid"][i]:
            lgd_sum[g] += dev["lgd_realized"][i]
            lgd_n[g] += 1
curves = {g: [sum(by_year[g][: y + 1]) / loans[g] for y in range(3)] for g in grades}
lgd = {g: lgd_sum[g] / lgd_n[g] for g in grades}

if output == "curves":
    json.dump({"curves": {str(g): c for g, c in curves.items()}, "lgd": {str(g): x for g, x in lgd.items()}}, sys.stdout)
elif output == "book":
    n = len(test["grade"])
    g = [int(x) for x in test["grade"]]
    d12 = [int(test["default"][i] and test["default_age"][i] <= 12) for i in range(n)]
    loss = [test["lgd_realized"][i] * test["ead_default"][i] * (1 + test["int_rate"][i]) ** (-test["default_age"][i] / 12)
            for i in range(n)]
    json.dump({
        "stage": [1] * n,
        "balance": test["funded"],
        "eir": test["int_rate"],
        "periods": [36] * n,
        "grade": g,
        "vintage": test["vintage"],
        "lgd": [lgd[x] for x in g],
        "pd": [curves[x][0] for x in g],
        "pd_lifetime": [curves[x][2] for x in g],
        "default_12m": d12,
        "default": test["default"],
        "reported": [curves[g[i]][0] * lgd[g[i]] * test["funded"][i] for i in range(n)],
        "realized_12m": [loss[i] * d12[i] for i in range(n)],
        "realized_lifetime": [loss[i] * test["default"][i] for i in range(n)],
    }, sys.stdout)
elif output == "defaults":
    keep = [i for i in range(len(test["grade"])) if test["lgd_valid"][i]]
    json.dump({
        "grade": [test["grade"][i] for i in keep],
        "vintage": [test["vintage"][i] for i in keep],
        "default_year": [test["default_year"][i] for i in keep],
        "lgd_estimate": [lgd[int(test["grade"][i])] for i in keep],
        "lgd_realized": [test["lgd_realized"][i] for i in keep],
    }, sys.stdout)
else:
    sys.exit(f"unknown output {output!r}")
