# Experiment 05 — Fat-tailed challengers and expected shortfall backtesting on the S&P 500

**Question.** Experiment 01 found that the RiskMetrics champion (EWMA volatility, normal quantile) has twice the expected number of 99% VaR exceptions on the S&P 500, and experiment 03 that a Student-t quantile with 5 degrees of freedom does not fix it. Which challenger does? And what does an expected shortfall backtest add, now that FRTB puts the ES, not the VaR, at the centre of market risk capital?

**Date run:** 2026-11-01 · **ModelFlow:** v0.16.0 · **Engine:** quantparity `bde35dd` (0.2.0)

## What this experiment added

The models and the tests are new in quantparity 0.2.0. Every function is reconciled with an independent reference in its CI ([`validation/RESULTS.md`](https://github.com/secke/quantparity/blob/main/validation/RESULTS.md)):

| quantparity 0.2.0 | Reference check |
|---|---|
| Student-t density, cdf and quantile, non-integer degrees of freedom | QuantLib `CumulativeStudentDistribution`, `InverseCumulativeStudent`: 1.4e-13 on 77 cases |
| Normal quantile refined to double precision | scipy: 4.2e-15. QuantLib's `InverseCumulativeNormal` is off by up to 3.4e-9: Acklam's approximation, not refined |
| ES of the normal and of the unit-variance Student-t, closed forms | QuantLib densities integrated numerically: 1.4e-12 |
| VaR / ES forecasts: EWMA volatility with normal, Student-t or filtered historical simulation (FHS) innovations | independent Python recursions: 7e-18 (EWMA), 1.4e-17 (FHS) |
| Acerbi-Székely Z1 and Z2 ES backtests, zones at -0.70 and -1.8 | formulas of the paper: 1.8e-15; size of the test checked by simulation |

ModelFlow exposes them as two built-in steps, `var_model` and `es_backtest`. Both backtests gained `from`, the first day evaluated, so that models with different warm-ups are compared on the same days.

## Reproduce

```bash
python3 ../01-sp500-var-backtest/fetch.py      # S&P 500 returns (FRED)
cd ../05-fat-tails-es-backtest
MF=../../target/release/mf
$MF run challengers.toml                       # 6 models, 18 steps, 0.1 s
python3 summarize.py                           # results/summary.md
# Reference checks: the built-in models against the external Python models of experiments 01 and 03
cd reference
for p in champion_external champion_builtin t5_external t5_builtin; do $MF run $p.toml; done
$MF diff champion_external.toml champion_builtin.toml
$MF diff t5_external.toml t5_builtin.toml
```

## Set-up

Data of experiment 01: 2,511 daily S&P 500 log returns, 2016-10-31 to 2026-10-28. [`challengers.toml`](challengers.toml) forecasts the one-day 99% VaR and 97.5% ES with six models that share the RiskMetrics volatility (λ = 0.94, started on 30 days) and differ by the distribution of the standardized return r_t / σ_t:

| Model | Distribution of r_t / σ_t |
|---|---|
| EWMA, normal | normal: the champion of experiment 01 |
| EWMA, Student-t 5, 4, 3 | Student-t with 5, 4 or 3 degrees of freedom, scaled to unit variance |
| Filtered HS, all residuals | the empirical distribution of every past standardized return (Barone-Adesi, Giannopoulos and Vosper 1999; Hull and White 1998) |
| Filtered HS, latest 500 | the same, on the latest 500 standardized returns (two years) |

Each model is backtested on the 99% VaR (`backtest_forecasts`: Kupiec, Christoffersen, Basel windows) and on the 97.5% ES (`es_backtest`), on the same 2,231 days, 2017-12-12 to 2026-10-28: the first forecast of the filtered simulation needs 250 residuals.

## Reference checks

Before comparing models, the built-in steps are checked against the external models they replace (`mf diff`, [results/reference_champion.txt](results/reference_champion.txt), [results/reference_t5.txt](results/reference_t5.txt)):

| Built-in `var_model` vs | Forecasts | Largest relative gap | Why |
|---|---:|---:|---|
| the champion of experiment 01 (`ewma_var.py`) | 2,482 | 6.5e-16 | floating-point order |
| the t5 challenger of experiment 03 (`ewma_t_var.py`) | 2,482 | 3.2e-10 | the external model reads its quantile from a table with 6 decimals (3.364930 instead of 3.3649299989) |

## Results

[`results/summary.md`](results/summary.md), [`results/summary.json`](results/summary.json). Run fingerprint `ba61765b896ba1a5`.

| Model | VaR 99% exceptions (expected 22.3) | Kupiec p | Christoffersen independence p | Basel windows red / yellow / green | ES 97.5% exceptions (expected 55.8) | Severity | Z2 | Z2 windows red / yellow / green | Mean VaR 99% |
|---|---:|---:|---:|---|---:|---:|---:|---|---:|
| EWMA, normal (champion) | 53 (2.38%) | 2.9e-8 | 0.17 | 1 / 4 / 3 | 90 | 1.23 | −0.98 yellow | 1 / 4 / 3 | 2.40% |
| EWMA, Student-t 5 | 43 (1.93%) | 9.4e-5 | 0.06 | 1 / 3 / 4 | 87 | 1.06 | −0.66 green | 0 / 3 / 5 | 2.69% |
| EWMA, Student-t 4 | 42 (1.88%) | 1.9e-4 | 0.05 | 1 / 3 / 4 | 90 | 1.02 | −0.64 green | 0 / 3 / 5 | 2.74% |
| EWMA, Student-t 3 | 43 (1.93%) | 9.4e-5 | 0.06 | 1 / 3 / 4 | 104 | 0.94 | −0.75 yellow | 0 / 3 / 5 | 2.71% |
| **Filtered HS, all residuals** | **18 (0.81%)** | **0.34** | 0.007 | **0 / 0 / 8** | **51** | **0.97** | **+0.11 green** | **0 / 0 / 8** | 3.51% |
| Filtered HS, latest 500 | 22 (0.99%) | 0.95 | 0.02 | 0 / 1 / 7 | 58 | 1.02 | −0.07 green | 0 / 1 / 7 | 3.48% |

*Severity*: mean ratio of the loss to the forecast ES on the days beyond the 97.5% VaR (1 when the ES is right). The windows are the eight consecutive 250-day windows ending on 2026-10-28; their zones, model by model, are in the summary.

**Findings.**

1. **Fatter symmetric tails do not fix the champion.** Going from 5 to 3 degrees of freedom does not reduce the 99% exceptions (43, 42, 43): once scaled to unit variance, the Student-t 99% quantile is almost flat in ν (2.61, 2.65, 2.62 standard deviations), and its 97.5% quantile falls (1.99, 1.96, 1.84). With 3 degrees of freedom the ES exceptions rise to 104 against 55.8 expected and Z2 gets worse than with 4. Every Student-t challenger fails Kupiec.
2. **What the champion misses is asymmetry and scale, not only kurtosis.** The standardized returns of the EWMA filter have an excess kurtosis of 3.8, that of a Student-t with 5.6 degrees of freedom: experiment 03's choice of 5 was about right. But their standard deviation is 1.06, not 1 (the filter underestimates the volatility by 6%), and their left tail is much longer than their right one: 1% quantile −3.18, 99% quantile +2.21. No unit-variance Student-t reaches −3.18: its 1% quantile is at most −2.66, at 3.55 degrees of freedom.
3. **Filtered historical simulation is the only challenger that passes.** It keeps the EWMA volatility and takes the quantile from the past standardized returns, so it captures both. The exception rate is right at 99% (18 and 22 exceptions for 22.3 expected, Kupiec p = 0.34 and 0.95) and at 97.5% (51 and 58 for 55.8). The Z2 is green over the whole period and in every 250-day window (one yellow window for the 500-day version, in 2024–2025), including the Covid window where the champion is red at 99% (12 exceptions) and at 97.5% (Z2 = −1.88).
4. **The ES backtest says something the VaR backtest does not.** The champion's losses beyond its 97.5% VaR are 23% larger than its ES on average (severity 1.23): the ES is too low in size, not only in frequency. The Student-t challengers have the opposite profile: their ES is about right in size (severity 0.94 to 1.06) but exceeded too often (87 to 104 times). With 5 and 4 degrees of freedom the two effects partly offset each other and the Z2 is green over the whole period, while Kupiec rejects both. A validator needs the Z2, its frequency and its severity together.
   **Correction ([experiment 07](../07-z2-simulated-pvalues/README.md)):** the "green over the whole period" comes from applying the 250-day thresholds to 2,231 days. Over that length, the simulated 5% quantile of Z2 is −0.24, and the Z2 of the t5 and t4 challengers has a p-value below 1e-4: the ES backtest rejects them too. The offset of frequency and severity is real but small. The window zones of this table are unaffected.
5. **The price of a correct tail is a 46% higher VaR.** The filtered simulation's mean 99% VaR is 3.51% against 2.40% for the champion, with the same volatility. The multiplier on σ ranges from 3.06 to 4.59, against 2.33 for the champion, and is 3.18 at the end of the period. That is the gap a capital comparison of champion and challenger starts from.
6. **The exceptions still cluster.** With far fewer exceptions, two pairs of consecutive days (2 and 5 February 2018, the "Volmageddon"; 3 and 4 April 2025, the tariff announcement) are enough to reject independence for the filtered simulation (p = 0.007 and 0.02). On the first day of a shock, the EWMA volatility has not reacted yet; a faster or asymmetric volatility filter (GARCH, GJR) is the next challenger to try.
7. **The Z2 thresholds are a caveat for very fat tails.** quantparity's validation simulates the Z2 test on 250 days when the forecasts are right. At −0.70 it rejects 4.3% to 5.0% of correct normal, t10 and t5 models, as Acerbi and Székely state, but 6.7% of correct t3 models, and 0.43% instead of 0.01% at −1.8. A red Z2 zone is strong evidence against a model with moderate tails and weaker evidence against one with very fat tails. ([Experiment 07](../07-z2-simulated-pvalues/README.md) replaces the thresholds with p-values simulated from each model's forecast distribution.)

## What this experiment changed in ModelFlow

| Finding | Status |
|---|---|
| Fat-tailed challengers had to be written as external scripts, with tabulated quantiles (experiment 03) | **added** in quantparity 0.2.0 / ModelFlow v0.16.0: `var_model` (EWMA normal, Student-t, filtered historical simulation), exact Student-t quantiles |
| No expected shortfall backtest | **added**: `es_backtest` (Acerbi-Székely Z1, Z2, zones, 250-day windows) |
| Models with different warm-ups were backtested on different periods | **added**: `from` in `backtest_forecasts` and `es_backtest` |
| QuantLib's normal quantile is accurate to about 1e-9 only | documented in quantparity's validation; quantparity refines its own to 4e-15 |
| Exceptions cluster on the first day of a shock, whatever the distribution | **added** in v0.17: GARCH and GJR filters ([experiment 06](../06-garch-filters/README.md)); they remove the second-day exceptions |
| Z2 thresholds assume moderate tails | **added** in v0.19: p-values of Z2 by simulation from the forecast distribution ([experiment 07](../07-z2-simulated-pvalues/README.md)) |
