# Experiment 02 — Independent reconstruction of the ECB AAA yield curve (2022–2026)

**Question.** Can ModelFlow rebuild an official yield curve from its published quotes and tell a validator where the reconstruction can be trusted? The test uses the ECB's euro area AAA government curve at every month end from January 2022 to October 2026, a period that runs from negative rates through the fastest hiking cycle in the euro's history to the easing that followed.

**Date run:** 2026-10-29 · **ModelFlow:** v0.12.0 · **Engine:** quantparity `eccd513`

## Data

| | |
|---|---|
| Source | [ECB Data Portal](https://data.ecb.europa.eu/), dataset `YC`, key `B.U2.EUR.4F.G_N_A.SV_C_YM.*` (AAA-rated euro area central government bonds, Svensson model) |
| Series | par yields `PY_*` and spot rates `SR_*` for 358 maturities from 3 months to 30 years |
| Dates | the last business day of each month, 2022-01-31 to 2026-10-28: 58 curves |
| Range covered | 3-month spot from −0.73% to +3.82%; 10-year spot from +0.03% to +3.58% |
| License | ECB statistics, free reuse with the source cited. Raw data is downloaded by `fetch.py`; the derived results in `results/` are committed |

## Method

One pipeline per date ([`pipeline.toml`](pipeline.toml), copied into `data/<date>/` by `prepare.py`), so that each date is a separate run that can be replayed on its own:

| Step | Kind | Content |
|---|---|---|
| `ecb` | `json` | raw ECB curve for the date |
| `market` | `process` ([`build_market.py`](models/build_market.py)) | two curves bootstrapped from 30 annual par instruments, 1Y to 30Y (see below) |
| `curve`, `curve_naive` | `curve_bootstrap` | construction and re-pricing of the 30 instruments |
| `zero_coupons`, `zero_coupons_naive`, `ecb_prices` | `process` ([`checks.py`](models/checks.py)) | 358 zero-coupon bonds and the ECB spot rates as prices, 100·e^(−r·t) |
| `spot_check`, `spot_check_naive` | `pricing_benchmark` | ECB spot curve vs. each reconstruction, at the 358 maturities |

The two reconstructions:

- **Naive** (`EUR-AAA-NAIVE`): the ECB "par yields" are read as annual-coupon par rates, as a desk would read quotes labelled "par yield".
- **Derived** (`EUR-AAA`): annual-coupon par rates are recomputed from the ECB spot rates at the annual pillars, c(n) = (1 − DF(n)) / Σ DF(1..n), with DF(t) = e^(−r(t)·t).

Every data transformation runs as a pipeline step, so its code is fingerprinted with the results. Gaps are measured in basis points of continuously compounded zero rate by [`analyze.py`](analyze.py).

## Reproduce

```bash
cd experiments/02-ecb-aaa-curve
python3 fetch.py        # 58 requests to the ECB API, about 1 minute
python3 prepare.py      # one pipeline per date
python3 analyze.py      # runs, replays and measures every date (about 90 s); writes results/
```

## Results

All 58 runs replay identically. Full per-date figures: [`results/by_date.csv`](results/by_date.csv); aggregate: [`results/summary.json`](results/summary.json).

### Whole period (58 dates × 358 maturities)

| Reconstruction | Segment | Median gap | Max gap |
|---|---|---:|---:|
| Both | re-pricing of the 30 input instruments | — | 2.2e-16 (in rate) |
| **Derived** | annual pillars (1Y…30Y) | 2e-13 bp | **2e-12 bp** |
| Derived | between pillars | 0.026 bp | 6.76 bp |
| Derived | below 1 year (extrapolated) | 13.5 bp | 87.0 bp |
| **Naive** | annual pillars | 3.40 bp | **7.52 bp** |
| Naive | between pillars | 3.41 bp | 11.90 bp |

### Interpolation error of the derived curve, by maturity band

| Band | Points | Median | 95th percentile | Max |
|---|---:|---:|---:|---:|
| below 1Y (extrapolation) | 522 | 9.53 bp | 49.08 bp | 86.95 bp |
| 1–2Y | 638 | 1.24 bp | 5.23 bp | 6.76 bp |
| 2–5Y | 1,914 | 0.35 bp | 1.02 bp | 1.52 bp |
| 5–10Y | 3,190 | 0.11 bp | 0.36 bp | 0.57 bp |
| 10–30Y | 12,760 | 0.016 bp | 0.041 bp | 0.117 bp |

### Representative dates

| Date | 3M spot | 10Y spot | Naive, pillars max | Derived, between pillars max | Derived, below 1Y max | Replay |
|---|---:|---:|---:|---:|---:|---|
| 2022-01-31 | −0.70% | 0.03% | 0.20 bp | 1.96 bp | 3.4 bp | identical |
| 2022-09-30 | 0.67% | 2.13% | 2.34 bp | 1.35 bp | **87.0 bp** | identical |
| 2023-10-31 | 3.82% | 2.82% | 5.30 bp | 4.01 bp | 43.3 bp | identical |
| 2024-01-31 | 3.81% | 2.27% | 4.10 bp | 5.20 bp | 75.8 bp | identical |
| 2025-06-30 | 1.86% | 2.68% | 4.72 bp | 1.36 bp | 4.4 bp | identical |
| 2026-10-28 | 2.55% | 3.39% | 6.68 bp | 0.31 bp | 35.7 bp | identical |

## Findings

1. **The ECB "par yields" are not annual-coupon par yields.** Read as annual par rates, they produce a curve that is wrong by 3.4 bp on median and up to 7.5 bp, and the bias grows with the rate level, from 0.2 bp at zero rates (January 2022) to 6.7 bp at 3.4% (October 2026): the signature of a compounding mismatch. A direct check on the ECB data settles it. Annual-coupon par yields computed from the ECB spot rates miss the published ones by up to 6.6 bp. Par yields with a continuously paid coupon, c(n) = (1 − DF(n)) / ∫₀ⁿ DF(t) dt, match them to 0.007 bp (residual of a monthly integration). The published par yields are the continuous-coupon par yields of the Svensson curve. A desk that bootstraps them as swap quotes misprices a 10-year zero-coupon by 0.31% of its price on median, and by up to 0.64%.
2. **The bootstrap is exact on real curves, including negative rates.** Rebuilt from consistent annual par rates, the curve matches the ECB spot rates at all 30 pillars to 2e-12 bp on all 58 dates, from −0.7% short rates to inverted curves. This is real-data evidence for the quantparity bootstrap, on top of its synthetic QuantLib parity tests.
3. **Log-linear interpolation is reliable beyond 2 years, not below.** Between annual pillars the gap to the smooth Svensson curve stays within about 1.5 bp from 2 years on and under 0.12 bp beyond 10 years. Between 1 and 2 years it reaches 6.8 bp (1Y5M, June 2023), at the peak of the curve inversion, where piecewise-flat forwards cannot follow the curvature.
4. **A curve built from annual pillars must not be used below 1 year.** Flat extrapolation of the first forward misses the money-market end by up to 87 bp: −87 bp in September 2022 during the hikes, +76 bp in January 2024 when cuts were priced in. Short maturities need money-market instruments (deposits, €STR OIS), which are not available as free public data: this is a data constraint that a real deployment has to solve.

## What this experiment changed or showed in ModelFlow

| Finding | Status |
|---|---|
| Replay, fingerprints and negative rates work on 58 real curves without change | confirmed |
| `pricing_benchmark` only has price tolerances; a curve check needs tolerances in basis points of rate, per maturity band, so the analysis had to be scripted outside ModelFlow (`analyze.py`) | open: rate-based tolerances, or a dedicated `curve_check` step |
| One market snapshot per run: a time series of validations needs one pipeline per date | open: a date parameter for `mf run`, or partitioned runs |
| The single-curve builder accepts deposits and par swaps only; zero-rate or continuous-coupon par quotes have to be converted first | open: zero-rate pillars in quantparity, reconciled with QuantLib |
| A curve's valid maturity range is not flagged; quotes below the first pillar are silently extrapolated | open: warn when pricing outside the pillar range |
