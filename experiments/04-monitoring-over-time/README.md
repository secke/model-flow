# Experiment 04 — Ongoing monitoring over time: dated pipelines and `mf series`

**Question.** Supervisors expect models to be monitored continuously, not only validated once (SS1/23, principle 4; SR 26-2). Can ModelFlow run the same validation at every date of a history, keep each date replayable, and show how the verdicts evolve, without a script that copies pipelines around?

**Date run:** 2026-10-31 · **ModelFlow:** v0.14.0 · **Engine:** quantparity `eccd513`

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

## What this experiment changed in ModelFlow

| Finding | Status |
|---|---|
| A series of validations needed one pipeline per date (experiment 02) | **added** in v0.14.0: dated pipelines, `--as-of DATE / all / new`, parallel runs |
| No view of a result over time | **added**: `mf series`, API `series`, SDK `series()` |
| Diff and replay did not know which date a run was for | **added**: the as-of date is part of the manifest, the history, the references (`@date`) and the diff causes |
| The overall run status mixes the whole-history backtest with the regulatory window | open: a per-pipeline choice of the checks that make the status |
| Monitoring thresholds (PSI fail for 3 consecutive months, zone changes) do not raise alerts yet | open: alert rules on series, through the existing webhooks |
