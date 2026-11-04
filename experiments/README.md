# Experiments on real data

ModelFlow's examples use synthetic data, which proves that the code works but not that it is useful. The experiments here run ModelFlow on real, public market data to find out what a validator can actually conclude with it, and what is missing. Each finding about the tool is either fixed or added to the backlog below.

## Conventions

- One directory per experiment, with a `README.md`: question, data source, method, how to reproduce, results, findings.
- Data is downloaded by a `fetch.py` script and is not committed (`data/` is ignored): some sources forbid redistribution, and the script documents exactly where every number comes from. Derived results are committed when the license allows it.
- Every run is replayed with `mf verify` before its results are written up, and the run fingerprints are quoted so that a result can be traced to its exact inputs and code.
- Data transformations run as pipeline steps (`process`), not as hidden pre-processing, so that their code is fingerprinted with the results.

## Index

| # | Experiment | Data | Headline result |
|---|---|---|---|
| 01 | [99% VaR backtest on the S&P 500](01-sp500-var-backtest/README.md) | FRED, 2016–2026, 2,511 days | Both textbook models fail over ten years (historical VaR: clustered exceptions, red over Nov 2021 – Nov 2022; normal EWMA: 2.3% exception rate, red during Covid), yet both are green on the current regulatory window |
| 02 | [Reconstruction of the ECB AAA yield curve](02-ecb-aaa-curve/README.md) | ECB, 58 month ends 2022–2026, 358 maturities | The ECB "par yields" are continuous-coupon par yields: reading them as annual par rates is wrong by up to 7.5 bp. Rebuilt consistently, the bootstrap matches the ECB curve to 2e-12 bp at every pillar; interpolation error is below 0.12 bp beyond 10 years but up to 87 bp when extrapolating below 1 year |
| 03 | [Change impact analysis with `mf diff`](03-change-impact/README.md) | S&P 500 and ECB data of 01 and 02 | A Student-t challenger removes 10 of 57 exceptions but still fails; a curve methodology fix moves pillar zero rates by up to 7.5 bp (beyond tolerance on 55 of 58 dates) while the check's verdict does not change; the v0.13 engine reproduces 706,000 recorded values exactly |
| 04 | [Ongoing monitoring over time](04-monitoring-over-time/README.md) | S&P 500 point-in-time snapshots (82 month ends), ECB curves (58) | A VaR model frozen in 2019 is red for 11 months after Covid and needs a capital add-on in 53 of 82 months; for a whole year (Nov 2022 – Oct 2023) its backtest is green while its calibration data fails the drift test every month. The ECB check of 02 runs from one pipeline instead of 58 copies, in 15 s. With alert rules (v0.15), the 32 months of failing drift become 2 alerts, raised on the third month of each regime change |
| 05 | [Fat-tailed challengers and ES backtesting](05-fat-tails-es-backtest/README.md) | S&P 500, 2016–2026 (data of 01) | Student-t challengers (5, 4, 3 degrees of freedom) all fail Kupiec: the champion misses the asymmetry and the scale of the standardized returns, not only their kurtosis. Filtered historical simulation passes the VaR and the ES backtests (18 exceptions for 22.3 expected, Z2 +0.11, every window green), for a VaR 46% higher. The ES backtest shows the champion's tail losses are 23% larger than its ES |
| 06 | [GARCH-type volatility filters](06-garch-filters/README.md) | S&P 500, 2019–2026 (data of 01) | GARCH and GJR filters remove the second-day exceptions of a shock (4 April 2025: EWMA volatility ×1.47 overnight, GJR ×2.50), but with a normal quantile they fail like the champion (34–40 exceptions for 17.6). With filtered historical simulation they pass every test for a 99% VaR 8–9% lower than EWMA FHS. After Covid the GJR fit is at the stationarity bound, in `arch` as in quantparity |

## What the experiments changed in ModelFlow

| Experiment | Finding | Status |
|---|---|---|
| 01 | `mf run pipeline.toml` from the pipeline's own directory broke every external step | fixed in v0.12.0 |
| 01 | VaR backtests gave no exception dates | added: `exception_list` |
| 01 | Basel traffic light applied to the whole sample instead of 250-day windows; no capital add-on | added: `basel_windows`, `latest_basel_window` |
| 01 | No view per period | added: `by_year` |
| 02 | Replay, fingerprints and negative rates hold on 58 real curves | confirmed |
| 02 | Comparing two curve methodologies needed a custom script | added in v0.13.0: `mf diff`, with tolerances in bp by maturity band |
| 03 | Step parameters were not fingerprinted: a parameter change looked like a reproducibility failure | fixed in v0.13.0 |
| 03 | An engine upgrade could not be told apart from a model change | fixed: the engine is recorded in every manifest and named by the diff |
| 03 | Arrays were compared by position | fixed: arrays of objects are matched by identifier |
| 03, 02 | A series of validations needed one pipeline per date and a loop | added in v0.14.0: dated pipelines (`${as_of}`), `mf run --as-of all / new` in parallel, `mf series` |
| 04 | The run status mixes the whole-history backtest with the regulatory window | fixed in v0.15: `status_from` |
| 04 | Monitoring thresholds over time do not raise alerts | fixed in v0.15: `[[alert]]` rules, `mf alerts`, webhooks |
| 05, 03 | Fat-tailed challengers were external scripts with tabulated quantiles; no ES backtest | added in v0.16 / quantparity 0.2.0: `var_model` (normal, Student-t, filtered HS), `es_backtest` (Acerbi-Székely), each reconciled with QuantLib or an independent implementation |
| 05 | Models with different warm-ups were backtested on different periods | added: `from` in the backtests |
| 05, 06 | Exceptions cluster on the day after a shock with an EWMA filter | added in v0.17 / quantparity 0.3.0: GARCH and GJR filters in `var_model`, reconciled with `arch`; they remove the second-day exceptions |

## Backlog

Ordered by the value they would have had in the experiments above.

1. **Rate-based tolerances for curve checks** (02): `mf diff` now compares two curves in basis points per maturity band (03); a check *against a reference* (`pricing_benchmark`) still has price tolerances only. Next: a `curve_check` step with the same tolerance rules.
2. ~~**Fat-tailed challenger VaR models and ES backtesting**~~ (01): done in v0.16 with quantparity 0.2.0 (experiment 05); GARCH and GJR filters in v0.17 with quantparity 0.3.0 (experiment 06). Next: Z2 p-values by simulation from the forecast distribution (05), and a check on the fitted GARCH persistence, which reaches the stationarity bound after Covid (06).
3. ~~**Validation over time**~~ (02): done in v0.14 (experiment 04). Alert rules on series and the per-pipeline choice of the checks that make the run status: done in v0.15 (experiment 04, part C).
4. **Zero-rate pillars and quote conventions in the curve builder** (02): accept zero rates and continuous-coupon par yields directly, in quantparity, reconciled with QuantLib.
5. **Flag pricing outside a curve's pillar range** (02): warn instead of silently extrapolating.

## Next experiments

- **Multi-asset portfolio risk on real history**: equity, EUR/USD, US rates and VIX from FRED; full-revaluation VaR/ES, the 2020 and 2022 stress periods, the FRTB P&L attribution test.
- **PD model validation on a public credit dataset**: discrimination, calibration and population stability between two periods.
