"""Market builder for the ECB AAA curve, two methodology versions (ModelFlow protocol: JSON on stdin -> JSON on stdout).

One curve, EUR-AAA, bootstrapped from 30 annual par instruments (1Y..30Y). Parameter `convention`:
- "ecb_par_yields" (version 1): the ECB "par yields" used as annual-coupon par rates;
- "derived" (version 2): annual-coupon par rates recomputed from the ECB spot rates,
  c(n) = (1 - DF(n)) / sum DF(1..n), DF(t) = exp(-r(t) t). See experiment 02 for why."""
import json, math, sys

req = json.load(sys.stdin)
ecb = req["inputs"]["ecb"]
convention = (req.get("params") or {})["convention"]
annual = lambda kind: {int(v["t"]): v["rate"] for v in ecb[kind].values() if v["t"] >= 1 and v["t"] == int(v["t"])}
if convention == "ecb_par_yields":
    quotes = annual("par_yields")
elif convention == "derived":
    df = {n: math.exp(-r * n) for n, r in annual("spot_rates").items()}
    quotes = {n: (1 - df[n]) / sum(df[i] for i in range(1, n + 1)) for n in sorted(df)}
else:
    sys.exit(f"unknown convention {convention}")

json.dump({"as_of": ecb["date"], "curves": {"EUR-AAA": {"float_frequency": 1, "instruments": [
    {"type": "swap", "maturity": n, "rate": quotes[n], "fixed_frequency": 1} for n in sorted(quotes)]}}}, sys.stdout)
