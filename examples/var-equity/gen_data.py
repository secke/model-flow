"""Generates deterministic synthetic GARCH(1,1) returns."""
import math, random
random.seed(7)
w, a, b = 2e-6, 0.08, 0.90
s2, rows = w / (1 - a - b), []
for i in range(1500):
    r = math.sqrt(s2) * random.gauss(0, 1)
    rows.append(f"{i},{r:.8f}")
    s2 = w + a * r * r + b * s2
open("data/returns.csv", "w").write("day,ret\n" + "\n".join(rows) + "\n")
