"""Control set from the raw ECB curve (ModelFlow protocol: JSON on stdin -> JSON on stdout).
params.output = "trades": zero-coupon bonds (face 100) at all 358 ECB maturities on params.curve;
params.output = "prices": the ECB spot rates as zero-coupon prices, 100 * exp(-r t)."""
import json, math, sys

msg = json.load(sys.stdin)
ecb, p = msg["inputs"]["ecb"], msg["params"]
spots = sorted(ecb["spot_rates"].items(), key=lambda kv: kv[1]["t"])
if p["output"] == "trades":
    out = {"trades": [{"id": f"ZC-{k}", "product": "zero_coupon_bond", "face": 100, "maturity": v["t"], "curve": p["curve"]}
                      for k, v in spots]}
else:
    out = {f"ZC-{k}": 100 * math.exp(-v["rate"] * v["t"]) for k, v in spots}
json.dump(out, sys.stdout)
