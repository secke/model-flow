"""Downloads the S&P 500 daily close from FRED and writes daily log returns.
The data is not committed (S&P Dow Jones Indices license): run this script first."""
import csv, io, math, os, sys, urllib.request

URL = "https://fred.stlouisfed.org/graph/fredgraph.csv?id=SP500"
rows = list(csv.reader(io.StringIO(urllib.request.urlopen(URL, timeout=30).read().decode())))[1:]
px = [(d, float(v)) for d, v in rows if v not in (".", "")]
out = "data/returns.csv" if len(sys.argv) < 2 else sys.argv[1]
os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
with open(out, "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["day", "ret"])
    for i in range(1, len(px)):
        w.writerow([px[i][0].replace("-", ""), f"{math.log(px[i][1] / px[i - 1][1]):.10f}"])
print(f"{len(px) - 1} returns, {px[0][0]} to {px[-1][0]}")
