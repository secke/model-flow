"""External model: parametric EWMA VaR (RiskMetrics).
ModelFlow protocol: JSON on stdin -> JSON on stdout."""
import json, math, sys

Z = {0.95: 1.6448536269514722, 0.99: 2.3263478740408408}

req = json.load(sys.stdin)
p = req.get("params") or {}
lam, alpha, warmup = p.get("lambda", 0.94), p.get("alpha", 0.99), int(p.get("warmup", 30))
r = req["inputs"]["returns"]

s2 = sum(x * x for x in r[:warmup]) / warmup
var = []
for t in range(warmup, len(r)):
    var.append(Z[alpha] * math.sqrt(s2))      # forecast for r[t] using information up to t-1
    s2 = lam * s2 + (1 - lam) * r[t] ** 2

json.dump({"start": warmup, "var": var, "model": "ewma", "lambda": lam}, sys.stdout)
