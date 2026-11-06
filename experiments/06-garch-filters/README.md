# Experiment 06 — GARCH-type volatility filters on the S&P 500

**Question.** In experiment 05, filtered historical simulation (FHS) on the RiskMetrics EWMA volatility was the only challenger to pass the 99% VaR and 97.5% ES backtests, but its exceptions still clustered: on the day after a shock, the EWMA volatility had not caught up. Does a volatility filter that reacts faster, GARCH(1,1), or more to losses, GJR-GARCH(1,1), fix the clustering? Does it change the capital?

**Date run:** 2026-11-04, persistence check added 2026-11-06 · **ModelFlow:** v0.17.0, v0.18.0 · **Engine:** quantparity `de7636c` (0.3.0)

## What this experiment added

| quantparity 0.3.0 | Reference check |
|---|---|
| GARCH(1,1) and GJR-GARCH(1,1) by Gaussian quasi-maximum likelihood, zero mean, with the conventions of the Python `arch` package (backcast, constraints) | `arch` 8.0.0 on 36 simulated samples ([`validation/RESULTS.md`](https://github.com/secke/quantparity/blob/main/validation/RESULTS.md)): conditional volatility 3.5e-18, log-likelihood at the optimum 1.8e-10, parameters 7.2e-7; and on the 27 fits of this experiment (below) |
| VaR / ES forecasts with a GARCH or GJR filter, fitted on a warm-up sample and refitted every `refit` days on an expanding or rolling window, with normal, Student-t or FHS innovations | the forecast of day t is recomputed from the fit in force and the returns up to t-1 (unit tests) |

ModelFlow exposes it through `var_model`: `volatility = "garch"` or `"gjr"`, `refit`, `fit_window`. The fits used are part of the step output, so they are fingerprinted, diffed and shown in the report with the forecasts. Since v0.18, a GARCH model step also has a status: `warn` when a fit reaches the stationarity bound (finding 5).

## Reproduce

```bash
python3 ../01-sp500-var-backtest/fetch.py      # S&P 500 returns (FRED)
cd ../06-garch-filters
MF=../../target/release/mf
$MF run challengers.toml                       # 8 models, 26 steps, 0.7 s
$MF verify challengers.toml
python3 summarize.py                           # results/summary.md, results/summary.json
pip install arch==8.0.0 && python3 reference/arch_check.py   # results/reference_arch.md
```

## Set-up

Data of experiment 01: 2,511 daily S&P 500 log returns, 2016-10-31 to 2026-10-28. [`challengers.toml`](challengers.toml) forecasts the one-day 99% VaR and 97.5% ES with eight models:

| Model | Volatility σ_t | Distribution of r_t / σ_t |
|---|---|---|
| EWMA, normal | RiskMetrics, λ = 0.94: the champion of experiment 01 | normal |
| EWMA, filtered HS | same | every past standardized return: the best challenger of experiment 05 |
| GARCH, normal | GARCH(1,1) | normal |
| GJR, normal | GJR-GARCH(1,1): losses raise the variance by α + γ, gains by α | normal |
| GJR, Student-t 5 | same | Student-t, 5 degrees of freedom, unit variance |
| GARCH, filtered HS | GARCH(1,1) | every past standardized return |
| GJR, filtered HS | GJR-GARCH(1,1) | every past standardized return |
| GJR, filtered HS, fit on latest 1,000 | GJR-GARCH(1,1) fitted on the latest 1,000 returns only (4 years) | every past standardized return |

The GARCH filters are fitted on the first 500 returns (two years) and refitted every 250 days, on every return up to the refit day unless stated otherwise; between refits the variance follows the fitted recursion. The filtered simulations start once 250 standardized returns are available. All models are backtested on the same 1,761 days, 2019-11-11 to 2026-10-28 (`from = 750`). The period has the Covid crash, the 2022 rate shock and the April 2025 tariff announcement, but not February 2018, one of the two clusters of experiment 05.

## Reference check on the real fits

[`reference/arch_check.py`](reference/arch_check.py) refits each of the 27 GARCH and GJR fits of the run with `arch`, on the same returns ([results/reference_arch.md](results/reference_arch.md)). The largest gap is 2.8e-9 in log-likelihood and 3.5e-6 on a parameter, including the fits that end at the stationarity bound (finding 5).

## Results

[`results/summary.md`](results/summary.md), [`results/summary.json`](results/summary.json). Run fingerprint `8e8c4d06c484f795` (ModelFlow v0.18.0), replayed identically by `mf verify`. The forecasts and backtests are identical to those of the first run under v0.17.0 (`262bf6fcdf34d94f`): `mf diff` finds only the added persistence checks. Expected exceptions: 17.6 at 99%, 44.0 at 97.5%.

| Model | VaR 99% exceptions | Kupiec p | Independence p | Consecutive exception days | Basel windows red / yellow / green | ES 97.5% exceptions | Severity | Z2 | Mean VaR 99% |
|---|---:|---:|---:|---|---|---:|---:|---:|---:|
| EWMA, normal (champion) | 40 (2.27%) | 4.3e-6 | 0.31 | 24–25 Feb 2020, 3–4 Apr 2025 | 1 / 3 / 3 | 71 | 1.18 | −0.91 yellow | 2.49% |
| EWMA, filtered HS (exp. 05) | 13 (0.74%) | 0.25 | 0.08 | 3–4 Apr 2025 | 0 / 0 / 7 | 35 | 0.97 | +0.23 green | 3.59% |
| GARCH, normal | 40 (2.27%) | 4.3e-6 | 0.92 | 3–4 Apr 2025 | 1 / 3 / 3 | 59 | 1.22 | −0.64 green | 2.43% |
| GJR, normal | 34 (1.93%) | 5.0e-4 | 0.25 | none | 0 / 4 / 3 | 62 | 1.24 | −0.75 yellow | 2.44% |
| GJR, Student-t 5 | 26 (1.48%) | 0.06 | 0.38 | none | 0 / 2 / 5 | 58 | 1.09 | −0.43 green | 2.73% |
| **GARCH, filtered HS** | **14 (0.80%)** | **0.37** | **0.64** | **none** | 0 / 1 / 6 | 29 | 1.10 | +0.28 green | **3.26%** |
| **GJR, filtered HS** | **15 (0.85%)** | **0.52** | **0.61** | **none** | 0 / 1 / 6 | 33 | 1.04 | +0.22 green | **3.30%** |
| GJR, filtered HS, fit on latest 1,000 | 12 (0.68%) | 0.15 | 0.68 | none | 0 / 0 / 7 | 31 | 1.03 | +0.28 green | 3.31% |

The Basel and Z2 zones of each model in each of the seven 250-day windows, and every exception day of every model, are in the summary. The whole-sample Z2 zones of this table use the 250-day thresholds; with p-values simulated over the 1,761 days ([experiment 07](../07-z2-simulated-pvalues/README.md)), the champion, GARCH normal and GJR normal are red and GJR t5 yellow; the filtered simulations stay green.

> **Since ModelFlow v0.20** the whole-sample ES zone of `es_backtest` is the zone of a p-value simulated from each model's forecast distribution ([experiment 07](../07-z2-simulated-pvalues/README.md)), not the 250-day thresholds used in this table. Rerunning this pipeline with v0.20 changes the whole-sample ES zones (and the recorded runs no longer replay identically: `mf diff` names the engine as the cause); the forecasts and the VaR backtests do not change, the windows keep their fixed-threshold zone, and their simulated zone agrees with it in 103 of the 104 windows of experiments 05 and 06.

**Findings.**

1. **A faster filter removes the second day of a shock, not the first.** On 3 April 2025 the S&P 500 fell 4.96%: every model has an exception, because no filter fed with returns up to the day before can see it coming. On 4 April it fell another 6.16%. Overnight, the EWMA volatility rose by a factor 1.47 and its FHS VaR to 5.13%: a second exception. GARCH raised the volatility by 2.13 (VaR 6.35%), GJR by 2.50 (VaR 8.17%): no exception. On 24–25 February 2020 the champion has two exceptions in a row; GARCH and GJR with a normal quantile only the first. No GARCH or GJR model with FHS has two exceptions in a row, and the independence p-value goes from 0.08 to 0.61–0.68. With 12 to 15 exceptions the test has little power, and this improvement rests on one pair of days. It removes a known weakness, not a rejection.
2. **The filter does not fix the tail.** With a normal quantile, GARCH has as many exceptions as the EWMA champion (40), and GJR has 34. Both fail Kupiec. The standardized returns still have an excess kurtosis of 2 to 6 and a 1% quantile of −2.8 to −3.1 against −2.33 for a normal. GJR with a Student-t 5 is borderline (26 exceptions, p = 0.06). What passes is the filtered simulation, with any of the three filters. Experiment 05 found this with EWMA; it holds for GARCH and GJR.
3. **A better filter makes the passing model cheaper.** GARCH and GJR with FHS pass every test EWMA FHS passes, with a mean 99% VaR of 3.26% and 3.30% against 3.59%, 8–9% less capital for the same test results. The volatility is about the same on average (1.04% and 1.05% against 1.07%). The saving is in the FHS multiplier of σ, 3.10 and 3.12 against 3.31: a filter that reacts to the first loss of a shock leaves fewer extreme standardized losses for the tail. The ES is right in size as well (severity 1.04 for GJR FHS, 0.97 for EWMA FHS; Z2 +0.22 and +0.23).
4. **Asymmetry matters in the volatility, not only in the distribution.** The GJR fits put almost all the reaction on losses: γ is 0.14 to 0.39 against α ≤ 0.12 in every fit, and α = 0 in the first two fits ([results/reference_arch.md](results/reference_arch.md)). The negative skew that experiment 05 found in the EWMA residuals (1% quantile −3.18, 99% quantile +2.21) is partly a volatility effect. With GJR, the 1% quantile of the residuals on the evaluated days is −2.93 against −3.12 for EWMA.
5. **After Covid, the expanding-window GJR fit is at the stationarity bound.** The fits of October 2020 and October 2021 give α + γ/2 + β = 1.000: an integrated variance with no long-run level. `arch` finds the same optimum, so this is not an optimizer artefact. The forecasts stay usable for one day ahead, but the model's long-run volatility and multi-day forecasts are undefined. Fitting on the latest 1,000 days does not avoid it in 2020–2021, and gives a persistence of 0.96 to 0.99 afterwards. Since v0.18 the step flags it: the four GJR models are `warn`, with the two fit dates, and the two GARCH models (highest persistence 0.986) `pass` ([summary](results/summary.md)).
6. **The window choice for the fit is second order.** GJR FHS on the latest 1,000 returns has 12 exceptions against 15, all windows green, and the same mean VaR. The difference comes from three days in 2024–2025 and is well inside sampling noise.

## What this experiment changed in ModelFlow

| Finding | Status |
|---|---|
| Exceptions cluster on the day after a shock with an EWMA filter (experiment 05) | **added** in quantparity 0.3.0 / ModelFlow v0.17.0: GARCH and GJR-GARCH filters in `var_model`, reconciled with `arch`. They remove the clustering (finding 1) |
| `arch` fitted on unscaled daily returns stops short of the optimum and reports convergence (quantparity validation: 33 of 36 samples) | documented; quantparity rescales internally |
| A fit at the stationarity bound is only visible in the step output | **added** in v0.18.0: `var_model` warns when a fit has α + γ/2 + β ≥ `max_persistence` (0.999), with the fit dates (`persistence_check`) |
| Z2 thresholds assume moderate tails (experiment 05) | **added** in v0.19: p-values of Z2 by simulation ([experiment 07](../07-z2-simulated-pvalues/README.md)) |
