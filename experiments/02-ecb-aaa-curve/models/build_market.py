"""Builds the market from the raw ECB curve (ModelFlow protocol: JSON on stdin -> JSON on stdout).

Two curves, both bootstrapped from 30 annual par instruments (1Y..30Y):
- EUR-AAA-NAIVE: the ECB "par yields" used as annual-coupon par rates (a plausible but wrong reading);
- EUR-AAA: annual-coupon par rates derived from the ECB spot rates at the annual pillars,
  c(n) = (1 - DF(n)) / sum DF(1..n) with DF(t) = exp(-r(t) t), the ECB spot rates being
  continuously compounded."""
import json, math, sys

ecb = json.load(sys.stdin)["inputs"]["ecb"]
annual = lambda kind: {int(v["t"]): v["rate"] for v in ecb[kind].values() if v["t"] >= 1 and v["t"] == int(v["t"])}
spot, par = annual("spot_rates"), annual("par_yields")
df = {n: math.exp(-spot[n] * n) for n in spot}
derived = {n: (1 - df[n]) / sum(df[i] for i in range(1, n + 1)) for n in sorted(df)}


def curve(quotes):
    return {"float_frequency": 1, "instruments": [
        {"type": "swap", "maturity": n, "rate": quotes[n], "fixed_frequency": 1} for n in sorted(quotes)]}


json.dump({"as_of": ecb["date"], "curves": {"EUR-AAA": curve(derived), "EUR-AAA-NAIVE": curve(par)}}, sys.stdout)
