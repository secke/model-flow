"""Copies pipeline.toml into every data/<date>/ directory fetched by fetch.py."""
import glob, os, shutil

for d in sorted(glob.glob("data/*/")):
    shutil.copy("pipeline.toml", os.path.join(d, "pipeline.toml"))
    for f in ("market.json", "trades.json", "ecb_prices.json"):
        if os.path.exists(os.path.join(d, f)):
            os.remove(os.path.join(d, f))
print(len(glob.glob("data/*/pipeline.toml")), "pipelines")
