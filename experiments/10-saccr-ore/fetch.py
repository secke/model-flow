"""Downloads the market data of ORE's examples (as of 2016-02-05), at the 1.8.17.0 tag, into data/market.

    python3 fetch.py
"""
import hashlib
import pathlib
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
BASE = "https://raw.githubusercontent.com/OpenSourceRisk/Engine/1.8.17.0/Examples/Input/"
FILES = {
    "conventions.xml": "443a70711a5835d8fe2e41f135aab0822cf624221d589894ce167c08fa849d74",
    "curveconfig.xml": "a8b63df2e2b615536e523e9652ef0b69ca900ef2819f60365c3ef558ca5a0c3d",
    "fixings_20160205.txt": "3278d52f4fc1d8befe98273c20bc5b89f2f5684b593cf96b95ea47f6cd11b9f7",
    "market_20160205_flat.txt": "dfe8f0e483f3b0c3e3b245e1ce71b82e151d416808f8de1ee5477a5caca51348",
    "pricingengine.xml": "7bbd88f3d29c860c03a153d582f8d9b06636e3af73dcea5f2056f5c3feb2f0eb",
    "todaysmarket.xml": "42915582e0156d79ab5755bd32d96c6ed6a1e93b08181528e638c9c97401e2e0",
}

out = HERE / "data/market"
out.mkdir(parents=True, exist_ok=True)
for name, sha in FILES.items():
    path = out / name
    if not path.exists() or hashlib.sha256(path.read_bytes()).hexdigest() != sha:
        with urllib.request.urlopen(BASE + name) as r:
            path.write_bytes(r.read())
    got = hashlib.sha256(path.read_bytes()).hexdigest()
    if got != sha:
        raise SystemExit(f"{name}: SHA-256 {got}, expected {sha}")
    print(f"{name}: ok")
