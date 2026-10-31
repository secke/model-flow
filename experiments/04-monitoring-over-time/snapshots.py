"""Point-in-time data for the VaR monitoring: one snapshot per month end, as a risk system would
have stored it at that date (every return known on that day, nothing after).

Reads the S&P 500 returns of experiment 01 (python3 ../01-sp500-var-backtest/fetch.py) and writes
data/<month end>/returns.csv for every month end from January 2020, plus data/reference.csv, the
calibration sample of the frozen model (returns up to 2019-12-31)."""
import csv, pathlib

HERE = pathlib.Path(__file__).resolve().parent
SRC = HERE.parent / "01-sp500-var-backtest" / "data" / "returns.csv"
rows = list(csv.DictReader(open(SRC)))
day = lambda r: f'{r["day"][:4]}-{r["day"][4:6]}-{r["day"][6:]}'

# Month end = last trading day of each month in the data.
ends = {}
for r in rows:
    ends[day(r)[:7]] = day(r)
ends = [d for m, d in sorted(ends.items()) if d >= "2020-01-01"]
if ends and ends[-1] == day(rows[-1]) and day(rows[-1])[8:] < "28":
    ends.pop()  # the current month is not over

data = HERE / "data"
data.mkdir(exist_ok=True)
with open(data / "reference.csv", "w") as f:
    f.write("day,ret\n" + "".join(f'{r["day"]},{r["ret"]}\n' for r in rows if day(r) <= "2019-12-31"))
for d in ends:
    (data / d).mkdir(exist_ok=True)
    with open(data / d / "returns.csv", "w") as f:
        f.write("day,ret\n" + "".join(f'{r["day"]},{r["ret"]}\n' for r in rows if day(r) <= d))
print(f"{len(ends)} snapshots, {ends[0]} to {ends[-1]}")
