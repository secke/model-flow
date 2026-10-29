"""Downloads the ECB euro area AAA government yield curve (Svensson model) at every month end
from January 2022: par yields and spot rates for all 358 maturities from 3 months to 30 years.
Writes data/<date>/ecb.json. Source: ECB Data Portal, dataset YC (free reuse, source: ECB)."""
import csv, io, json, os, urllib.request

API = "https://data-api.ecb.europa.eu/service/data/YC/B.U2.EUR.4F.G_N_A.SV_C_YM."
START = "2022-01-01"


def get(key, params):
    url = f"{API}{key}?format=csvdata&detail=dataonly&{params}"
    with urllib.request.urlopen(url, timeout=60) as r:
        return list(csv.DictReader(io.StringIO(r.read().decode())))


def years(label):
    """`SR_10Y6M` -> 10.5, `PY_3M` -> 0.25."""
    t = label.split("_", 1)[1]
    y, m = (t.split("Y") + [""])[:2] if "Y" in t else ("0", t)
    return int(y) + (int(m.rstrip("M")) if m.rstrip("M") else 0) / 12


days = sorted(r["TIME_PERIOD"] for r in get("SR_10Y", f"startPeriod={START}"))
month_ends = sorted({d[:7]: d for d in days}.values())
for d in month_ends:
    rows = get("", f"startPeriod={d}&endPeriod={d}")
    out = {"date": d, "par_yields": {}, "spot_rates": {}}
    for r in rows:
        k = r["KEY"].rsplit(".", 1)[1]
        if k.startswith(("PY_", "SR_")):
            kind = "par_yields" if k.startswith("PY_") else "spot_rates"
            out[kind][k[3:]] = {"t": years(k), "rate": float(r["OBS_VALUE"]) / 100}
    os.makedirs(f"data/{d}", exist_ok=True)
    with open(f"data/{d}/ecb.json", "w") as f:
        json.dump(out, f, indent=1, sort_keys=True)
    print(d, len(out["par_yields"]), "par yields,", len(out["spot_rates"]), "spot rates")
