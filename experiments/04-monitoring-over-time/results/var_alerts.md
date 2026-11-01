**sp500-var-monitoring** — 0 active alert(s), 82 date(s), latest 2026-10-28

| Alert | Condition | State | Episodes | Periods |
|---|---|---|---:|---|
| backtest-red | `backtest.latest_basel_window.zone is red` | ok | 2 | 2020-02-28 → 2020-07-31 (6 dates)  red<br>2020-09-30 → 2021-01-29 (5 dates)  red |
| zone-change | `backtest.latest_basel_window.zone changes` | ok | 9 | 2020-02-28  yellow → red<br>2020-08-31  red → yellow<br>2020-09-30  yellow → red<br>2021-02-26  red → yellow<br>2022-11-30  yellow → green<br>2024-08-30  green → yellow<br>2024-09-30  yellow → green<br>2024-10-31  green → yellow<br>2026-04-30  yellow → green |
| drift-persistent | `drift.psi > 0.25, 3 dates in a row` | ok | 2 | 2020-06-30 → 2021-05-28 (12 dates, raised 2020-08-31)  0.285554<br>2022-04-29 → 2023-10-31 (19 dates, raised 2022-06-30)  0.423868 |
