# Experiment 01 — 99% VaR backtest on the S&P 500 (2016–2026)

**Question.** How do two textbook VaR models behave on ten years of real equity returns, including the Covid crash (2020), the 2022 rate-hike cycle and the April 2025 tariff shock? And can a validator reach a conclusion with ModelFlow alone?

**Date run:** 2026-10-29 · **ModelFlow:** v0.12.0 · **Engine:** quantparity `eccd513`

## Data

| | |
|---|---|
| Source | FRED series [`SP500`](https://fred.stlouisfed.org/series/SP500) (S&P 500 daily close) |
| Period | 2016-10-31 to 2026-10-28, 2,511 daily log returns |
| Statistics | annualized volatility 18.1%, excess kurtosis 16.7, worst day −12.8% (2020-03-16) |
| File fingerprint | `returns.csv` SHA-256 `2a6e5754…cdf8` |
| License | © S&P Dow Jones Indices: the data is **not committed**; `fetch.py` downloads it |

FRED only distributes the last ten years of this series, so rerunning later shifts the window and changes the numbers. The run fingerprints below identify the exact snapshot used.

## Pipeline

[`pipeline.toml`](pipeline.toml):

| Step | Kind | Content |
|---|---|---|
| `returns`, `days` | `csv` | returns and their `YYYYMMDD` dates |
| `var_hist_99` | `var_historical` | full-sample 99% historical VaR (3.34%) |
| `backtest_hist` | `kupiec_backtest` | rolling 250-day historical VaR, backtested over 2,261 days |
| `ewma_model` | `process` (Python, the model of [`examples/var-equity`](../../examples/var-equity/models/ewma_var.py)) | RiskMetrics EWMA, λ = 0.94, normal quantile |
| `backtest_ewma` | `backtest_forecasts` | backtest of the EWMA forecasts over 2,481 days |

## Reproduce

```bash
cd experiments/01-sp500-var-backtest
python3 fetch.py                       # downloads data/returns.csv from FRED
../../target/release/mf run pipeline.toml
../../target/release/mf verify pipeline.toml
../../target/release/mf report pipeline.toml -o report.html
```

Run fingerprint of the snapshot above: `run_hash b999d23818154536c3d4e6bd6fc7876f4af00b59cf9cc1291869352d33553d00`. Replay: identical (6 steps).

## Results

### Whole sample

| Model | Exceptions | Rate (expected 1%) | Kupiec POF | Christoffersen | Traffic light (whole sample) | Verdict |
|---|---:|---:|---|---|---|---|
| Historical VaR, 250-day window | 35 / 2,261 | 1.55% | p = 0.015 (warn) | p_ind = 0.0001 (**fail**) | yellow | **fail** |
| EWMA, normal | 57 / 2,481 | 2.30% | p < 0.001 (**fail**) | p_ind = 0.19 (pass) | red | **fail** |

### Regulatory 250-day windows (Basel traffic light)

| Window | Historical: exceptions | Zone | Add-on | EWMA: exceptions | Zone | Add-on |
|---|---:|---|---:|---:|---|---:|
| 2017-11-14 → 2018-11-09 | 5 | yellow | 0.40 | 7 | yellow | 0.65 |
| 2018-11-12 → 2019-11-08 | 0 | green | 0 | 6 | yellow | 0.50 |
| 2019-11-11 → 2020-11-05 | 8 | yellow | 0.75 | 12 | **red** | 1.00 |
| 2020-11-06 → 2021-11-03 | 0 | green | 0 | 6 | yellow | 0.50 |
| 2021-11-04 → 2022-11-01 | 11 | **red** | 1.00 | 6 | yellow | 0.50 |
| 2022-11-02 → 2023-10-31 | 0 | green | 0 | 2 | green | 0 |
| 2023-11-01 → 2024-10-29 | 5 | yellow | 0.40 | 4 | green | 0 |
| 2024-10-30 → 2025-10-29 | 5 | yellow | 0.40 | 8 | yellow | 0.75 |
| **2025-10-30 → 2026-10-28 (current)** | **1** | **green** | **0** | **2** | **green** | **0** |

### Largest exceptions (historical VaR)

| Date | Return | VaR | Excess loss |
|---|---:|---:|---:|
| 2020-03-16 | −12.77% | 5.02% | 7.75% |
| 2020-03-12 | −9.99% | 4.51% | 5.48% |
| 2020-03-09 | −7.90% | 3.41% | 4.49% |
| 2025-04-04 | −6.16% | 2.99% | 3.17% |
| 2018-02-05 | −4.18% | 1.55% | 2.63% |

## Interpretation

1. **The historical VaR reacts too slowly.** Its exceptions cluster: after an exception, the probability of another one the next day is 14% instead of 1%, so Christoffersen rejects independence. It went red over November 2021 – November 2022, when rising rates pushed volatility up for months, while its 250-day window still held the calm of 2021.
2. **The normal EWMA reacts fast, but its tails are too thin.** Independence holds, but the exception rate is 2.3% with an excess kurtosis of 16.7, so a normal quantile is not enough at 99%. It was red during Covid (12 exceptions) and yellow in 5 of the other 8 windows.
3. **The whole-sample verdict and the regulatory verdict tell different stories.** Both models fail over ten years, yet both are green on the current window, which is the one a supervisor would use for the capital multiplier today. A validation report needs both views, and this one is what the new `basel_windows` output gives.

**Validation conclusion (illustrative).** Neither model is fit for a 99% regulatory VaR without changes. Natural challengers: EWMA with Student-t innovations, filtered historical simulation, or a stressed-period floor for the historical VaR.

## What this experiment changed in ModelFlow

| Finding | Action |
|---|---|
| Running a pipeline by its bare file name (`mf run pipeline.toml`) broke every external step, with a misleading "failed to start python3" | **Fixed** in v0.12.0, with a test |
| The backtest gave exception counts but no dates: a validator could not investigate them | **Added** `exception_list` (date, return, VaR, excess loss) |
| The traffic light was applied to the whole sample, while the Basel rule is defined on 250 days, so the add-on was never computed | **Added** `basel_windows` and `latest_basel_window` with the add-on |
| No view per period | **Added** `by_year` |
| No ES backtest, no fat-tailed challenger model | Open (see [experiments index](../README.md#backlog)) |
