# Experiment 04 — Ongoing monitoring over time: dated pipelines and `mf series`

**Question.** Supervisors expect models to be monitored continuously, not only validated once (SS1/23, principle 4; SR 26-2). Can ModelFlow run the same validation at every date of a history, keep each date replayable, and show how the verdicts evolve, without a script that copies pipelines around?

**Date run:** 2026-10-31 (parts A, B) and 2026-11-01 (part C) · **ModelFlow:** v0.14.0, v0.15.0 for part C · **Engine:** quantparity `eccd513`

## What v0.14 adds

A pipeline is **dated** when `${as_of}` appears in its data paths:

```toml
[[step]]
name = "returns"
kind = "csv"
version = "1.0.0"
path = "data/${as_of}/returns.csv"
column = "ret"
```

| Command | Effect |
|---|---|
| `mf run pipeline.toml --as-of 2024-01-31` | one run for that date, recorded with it |
| `mf run pipeline.toml --as-of all` | one run per date whose data exists (read from the file system), in parallel |
| `mf run pipeline.toml --as-of new` | only the dates without a recorded run: the command of a daily or monthly job |
| `mf verify pipeline.toml --as-of 2024-01-31` | replays the latest run of that date, with that date |
| `mf diff pipeline.toml@2024-01-31 pipeline.toml@2024-02-29` | what changed from one date to the next, and where it started |
| `mf series pipeline.toml PATH...` | the value of any output, or the status of every check, date by date (text, CSV, Markdown, JSON), with the verdict changes in date order |

The as-of date is recorded in the manifest and the history, sent to external models (`"as_of"` next to `"inputs"` and `"params"`) and passed to Python steps that ask for it. The server runs a dated pipeline for the latest date with data (scheduler, or `POST …/runs?as_of=`), and serves the series (`GET /api/pipelines/{name}/series?path=…`). The Python SDK has `run(as_of=…)`, `run_dates("all" | "new" | [...])` and `series(paths)`, which returns rows ready for `pandas.DataFrame`.

## Reproduce

```bash
python3 ../01-sp500-var-backtest/fetch.py     # S&P 500 returns (FRED)
python3 ../02-ecb-aaa-curve/fetch.py          # ECB curves
cd ../04-monitoring-over-time
python3 snapshots.py                          # point-in-time snapshots, one per month end
MF=../../target/release/mf
$MF run var_monitoring.toml --as-of all       # 82 month ends
$MF run ecb.toml --as-of all                  # 58 month ends
$MF series var_monitoring.toml backtest.latest_basel_window.zone backtest.latest_basel_window.exceptions \
    backtest.latest_basel_window.plus_factor drift.psi drift.status var_250d.var
$MF series var_monitoring.toml                # the outputs that make the run status (v0.15)
$MF alerts var_monitoring.toml                # alert rules over the 82 month ends (v0.15)
$MF series ecb.toml spot_check.failed spot_check.max_abs_diff spot_check_naive.max_abs_diff \
    'curve.pillars[maturity=10.0].zero_rate'
```

## A. A VaR model frozen at the end of 2019, monitored monthly to October 2026

**Set-up.** The model is the RiskMetrics EWMA VaR of experiment 01 (λ = 0.94, normal quantile, 99%), frozen at its calibration date, 31 December 2019. At every month end from January 2020, [`var_monitoring.toml`](var_monitoring.toml) runs on a **point-in-time snapshot**: the returns a risk system would have held on that date, nothing after ([`snapshots.py`](snapshots.py)). Each run checks:

- the **regulatory backtest**: exceptions, Basel zone and capital add-on over the latest 250 days (`backtest.latest_basel_window`);
- the **drift of the data**: population stability index of the last 250 returns against the calibration sample, November 2016 to December 2019 (`drift`, PSI: pass below 0.10, warn up to 0.25, fail above);
- the current 99% historical VaR on 250 days, for scale.

82 runs in 2.0 s on 8 threads, each replayable on its own (`mf verify var_monitoring.toml --as-of 2020-03-31`: identical). Full series: [`results/var_monitoring.csv`](results/var_monitoring.csv), [`results/var_monitoring.txt`](results/var_monitoring.txt).

**The verdict changes, as `mf series` lists them:**

```
2020-02-28  backtest zone: yellow → red          Covid crash
2020-06-30  drift: warn → fail
2020-08-31  backtest zone: red → yellow
2020-09-30  backtest zone: yellow → red
2021-02-26  backtest zone: red → yellow
2021-06-30  drift: fail → warn
2022-04-29  drift: warn → fail                   rate-hike cycle
2022-11-30  backtest zone: yellow → green
2023-11-30  drift: fail → warn
2024-08-30  backtest zone: green → yellow
2024-09-30  backtest zone: yellow → green
2024-10-31  backtest zone: green → yellow
2025-06-30  drift: warn → fail                   after the April 2025 tariff shock
2026-04-30  backtest zone: yellow → green
```

| Over 82 month ends | |
|---|---|
| Basel zone of the latest 250 days | green 29, yellow 42, **red 11** (February to July 2020, September 2020 to January 2021) |
| Months with a capital add-on | 53 of 82; add-on 1.00 (multiplier 4) during the red months, 0.85 after the tariff shock (9 exceptions, April–June 2025) |
| Data drift (PSI) | pass 15, warn 35, **fail 32**; peak 0.996 on 2022-11-30 |
| 99% historical VaR, 250 days | from 1.59% (April 2024) to 7.90% (March 2020 to February 2021) |

**Findings.**

1. **A model can pass its backtest while its calibration data no longer describes the market.** From November 2022 to October 2023, the backtest is green every month while the PSI fails every month, with its peak of the whole period (0.996) in the very first green month. The EWMA volatility adapts, so the exceptions stay rare. But the distribution of returns is nothing like the 2016–2019 sample the model was calibrated and approved on. A monitoring framework that only looks at the traffic light misses that the approval evidence is out of date. That is the signal that should trigger a review.
2. **The two indicators react on different clocks.** The backtest turns red at the first month end of the Covid crash (February 2020), because it counts exceptions as they happen. The PSI only fails in June 2020, because it needs enough of the 250-day window to be in the new regime. The PSI then stays in fail until May 2021, long after the backtest has calmed down. Monitoring needs both indicators, and their history, to tell a one-off shock from a lasting change.
3. **The run status alone is not a monitoring indicator.** The overall status of every run is `fail`, because the backtest over the whole history (Kupiec, from 2016) fails, as experiment 01 found. `mf series` is what makes the regulatory window and the drift visible month by month.
4. **The production cycle works on its own.** With one snapshot missing, `mf run --as-of new` ran the 81 existing dates. Once the October 2026 snapshot was added, the same command ran only that date (0.1 s). `mf diff var_monitoring.toml@2026-09-30 var_monitoring.toml@2026-10-28` then traces the month's changes to the new data (`returns`, `days`: "source data changed, as of 2026-09-30 → 2026-10-28").

## B. The ECB curve check of experiment 02, without its preparation script

Experiment 02 copied its pipeline into 58 directories (`prepare.py`) to get one run per date. Here [`ecb.toml`](ecb.toml) is the same pipeline with one change, `path = "../02-ecb-aaa-curve/data/${as_of}/ecb.json"`:

| | Experiment 02 | Experiment 04 |
|---|---|---|
| Pipelines | 58 copies | 1 |
| Runs of the 58 dates | sequential, in an analysis script | `mf run ecb.toml --as-of all`: 34.9 s on 1 thread, **15.5 s** on 8 |
| Outputs | — | identical to experiment 02 on 58 of 58 dates (`mf diff`, 0 changes) |
| Time series | custom script | `mf series` ([`results/ecb_monitoring.csv`](results/ecb_monitoring.csv)) |

The external models are Python processes, so parallelism is limited by process start-up: 2.3 times faster on 8 cores. Built-in steps scale better (part A: 82 dates in 2.0 s).

The series makes the trend of experiment 02's first finding visible directly: the largest price error of the naive reading of the ECB par yields grows from 0.027 (January 2022, 10-year rate 0.03%) to 0.718 per 100 of face value (September 2026, 3.58%). Its correlation with the 10-year rate over the 58 dates is 0.94. Comparing two consecutive dates (`mf diff ecb.toml@2023-05-31 ecb.toml@2023-06-30`, [output](results/ecb_2023-05-31_vs_2023-06-30.txt)) traces the monthly changes to the new ECB data. Every other change is classified as downstream.

## C. Run status and alerts that a monitoring team can act on (v0.15)

Parts A and B left two gaps, both found on these data: every run had the status `fail` (the whole-history backtest), and the monitoring signals had to be read off the series by eye. v0.15 adds two declarations to the pipeline ([`var_monitoring.toml`](var_monitoring.toml)):

```toml
# The run status follows the regulatory window and the drift, not the whole-history backtest
status_from = ["backtest.latest_basel_window.zone", "drift.status"]

[[alert]]
name = "backtest-red"
path = "backtest.latest_basel_window.zone"
one_of = ["red"]
message = "Basel red zone: capital multiplier 4, model review required"

[[alert]]
name = "zone-change"
path = "backtest.latest_basel_window.zone"
changes = true

[[alert]]
name = "drift-persistent"
path = "drift.psi"
above = 0.25
consecutive = 3
message = "Returns no longer match the calibration sample: review the approval evidence"
```

Traffic lights count as verdicts (green = pass, yellow = warn, red = fail); a chosen output that is missing counts as `fail`. The rules are evaluated on the series of results. `mf run` prints the alerts raised on the dates it runs, `mf alerts` lists the episodes and exits with 1 while an alert is active, and the server sends each raised alert to its webhook. The model and its outputs are unchanged: `mf diff` finds identical results on 82 of 82 dates between the v0.14 and v0.15 runs (only the engine version in the built-in steps' fingerprints differs).

**Run status** ([`results/var_status.csv`](results/var_status.csv)):

| | v0.14 (every check) | v0.15 (`status_from`) |
|---|---:|---:|
| fail | 82 | 36: 4 red zone only, 25 drift only, 7 both |
| warn | 0 | 38 |
| pass | 0 | 8 (March–September 2024 except August, April–May 2026) |

**Alerts** ([raised by `mf run`](results/var_alerts_raised.txt), [episodes](results/var_alerts.txt), [Markdown](results/var_alerts.md)): 13 alerts over 82 month ends.

| Alert | Raised | Episodes |
|---|---|---|
| backtest-red | 2020-02-28, 2020-09-30 | 6 and 5 months in the red zone |
| zone-change | 9 times | the 9 traffic-light changes of part A |
| drift-persistent | 2020-08-31, 2022-06-30 | PSI above 0.25 from June 2020 to May 2021 (12 months), from April 2022 to October 2023 (19 months) |

**Findings.**

5. **The status now separates the months that need action.** 21 of the 29 green months are not `pass`, because the data drift. That is finding 1 of part A, which a monitoring dashboard can now show directly instead of a uniform `fail`.
6. **Persistence turns 32 failing months into 2 alerts.** The PSI fails in 32 months; with `consecutive = 3`, the alert is raised twice, on the third month of each lasting regime change. The one-month spike after the April 2025 tariff shock (PSI fail in June 2025 only) raises no drift alert, and the zone stays yellow, with 9 exceptions and an add-on of 0.85 from April to June 2025, so no zone alert either. A rule on the exception count (`backtest.latest_basel_window.exceptions`, `above = 8`) would be the one to add for that case. The second drift alert, on 2022-06-30, comes five months before the backtest turns green and stays active through the whole green year that follows (until October 2023): the warning that the approval evidence is out of date reaches the monitoring team while the traffic light says nothing.

## What this experiment changed in ModelFlow

| Finding | Status |
|---|---|
| A series of validations needed one pipeline per date (experiment 02) | **added** in v0.14.0: dated pipelines, `--as-of DATE / all / new`, parallel runs |
| No view of a result over time | **added**: `mf series`, API `series`, SDK `series()` |
| Diff and replay did not know which date a run was for | **added**: the as-of date is part of the manifest, the history, the references (`@date`) and the diff causes |
| The overall run status mixes the whole-history backtest with the regulatory window | **added** in v0.15.0: `status_from`, traffic lights as verdicts |
| Monitoring thresholds (PSI fail for 3 consecutive months, zone changes) do not raise alerts | **added** in v0.15.0: `[[alert]]` rules (`one_of`, `above`, `below`, `changes`, `consecutive`), `mf alerts`, API, webhooks, SDK |
