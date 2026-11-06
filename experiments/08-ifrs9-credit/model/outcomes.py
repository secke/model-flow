"""Observed outcome of each loan: default date, exposure and LGD at default.

The file has no payment history, only the last payment date. A charged-off loan is taken to
default at 90 days past due: three months after its first missed instalment, which falls one
month after the last payment (or after the issue date). Exposure at default: principal not
repaid. Realized LGD: 1 - (recoveries - collection fees) / EAD, floored at 0 and capped at 1;
recovery dates are not in the data, so it is undiscounted. Loans with an EAD below 1 USD have
no LGD (`lgd_valid` = 0).
"""
import json
import sys

t = json.load(sys.stdin)["inputs"]["loans"]
n = len(t["grade"])
age, year, ead, lgd, valid = [0] * n, [0] * n, [0.0] * n, [0.0] * n, [0] * n
for i in range(n):
    if not t["defaulted"][i]:
        continue
    issue = int(t["issue_month"][i])
    last = int(t["last_payment_month"][i])
    age[i] = (last if last >= 0 else issue) - issue + 4
    year[i] = (issue + age[i]) // 12
    ead[i] = t["funded"][i] - t["total_rec_prncp"][i]
    if ead[i] >= 1:
        lgd[i] = min(max(1 - (t["recoveries"][i] - t["collection_recovery_fee"][i]) / ead[i], 0.0), 1.0)
        valid[i] = 1
json.dump({"vintage": t["vintage"], "grade": t["grade"], "funded": t["funded"], "int_rate": t["int_rate"],
           "default": t["defaulted"], "default_age": age, "default_year": year, "ead_default": ead,
           "lgd_realized": lgd, "lgd_valid": valid}, sys.stdout)
