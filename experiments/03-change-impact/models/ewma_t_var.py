"""Challenger model: EWMA VaR with Student-t innovations (ModelFlow protocol: JSON on stdin -> JSON on stdout).

Same volatility filter as the champion (RiskMetrics EWMA), but the quantile is the one of a Student-t
distribution with `nu` degrees of freedom, scaled to unit variance: q = t_alpha(nu) * sqrt((nu - 2) / nu).
Student-t quantiles are tabulated (standard tables, 6 decimals) to keep the model free of dependencies."""
import json, math, sys

T = {
    0.99: {3: 4.540703, 4: 3.746947, 5: 3.364930, 6: 3.142668, 8: 2.896459, 10: 2.763769},
    0.95: {3: 2.353363, 4: 2.131847, 5: 2.015048, 6: 1.943180, 8: 1.859548, 10: 1.812461},
}

req = json.load(sys.stdin)
p = req.get("params") or {}
lam, alpha, warmup, nu = p.get("lambda", 0.94), p.get("alpha", 0.99), int(p.get("warmup", 30)), int(p.get("nu", 5))
q = T[alpha][nu] * math.sqrt((nu - 2) / nu)
r = req["inputs"]["returns"]

s2 = sum(x * x for x in r[:warmup]) / warmup
var = []
for t in range(warmup, len(r)):
    var.append(q * math.sqrt(s2))      # forecast for r[t] using information up to t-1
    s2 = lam * s2 + (1 - lam) * r[t] ** 2

json.dump({"start": warmup, "var": var, "model": "ewma-t", "lambda": lam, "nu": nu, "quantile": q}, sys.stdout)
