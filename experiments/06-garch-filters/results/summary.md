Run `8e8c4d06c484f795`, 1761 days evaluated (2019-11-11 onwards), every model on the same days. Expected exceptions: 17.6 at 99%, 44.0 at 97.5%.

| Model | VaR 99% exceptions | Kupiec p | Christoffersen p (independence) | Consecutive exception days | Basel windows red / yellow / green | ES 97.5% exceptions | Severity | Z2 | ES windows red / yellow / green | Mean VaR | Mean ES |
|---|---:|---:|---:|---|---|---:|---:|---:|---|---:|---:|
| EWMA, normal (champion) | 40 (2.27%) | 4.3e-06 | 0.31 | 2020-02-24 / 02-25, 2025-04-03 / 04-04 | 1 / 3 / 3 | 71 | 1.18 | -0.91 (yellow) | 1 / 3 / 3 | 2.49% | 2.50% |
| EWMA, filtered HS (experiment 05) | 13 (0.74%) | 2.5e-01 | 0.08 | 2025-04-03 / 04-04 | 0 / 0 / 7 | 35 | 0.97 | +0.23 (green) | 0 / 0 / 7 | 3.59% | 3.81% |
| GARCH, normal | 40 (2.27%) | 4.3e-06 | 0.92 | 2025-04-03 / 04-04 | 1 / 3 / 3 | 59 | 1.22 | -0.64 (green) | 0 / 3 / 4 | 2.43% | 2.44% |
| GJR, normal | 34 (1.93%) | 5.0e-04 | 0.25 | none | 0 / 4 / 3 | 62 | 1.24 | -0.75 (yellow) | 1 / 3 / 3 | 2.44% | 2.45% |
| GJR, Student-t 5 | 26 (1.48%) | 6.1e-02 | 0.38 | none | 0 / 2 / 5 | 58 | 1.09 | -0.43 (green) | 0 / 1 / 6 | 2.73% | 2.86% |
| GARCH, filtered HS | 14 (0.80%) | 3.7e-01 | 0.64 | none | 0 / 1 / 6 | 29 | 1.10 | +0.28 (green) | 0 / 0 / 7 | 3.26% | 3.38% |
| GJR, filtered HS | 15 (0.85%) | 5.2e-01 | 0.61 | none | 0 / 1 / 6 | 33 | 1.04 | +0.22 (green) | 0 / 0 / 7 | 3.30% | 3.68% |
| GJR, filtered HS, fit on latest 1,000 | 12 (0.68%) | 1.5e-01 | 0.68 | none | 0 / 0 / 7 | 31 | 1.03 | +0.28 (green) | 0 / 0 / 7 | 3.31% | 3.70% |

Persistence check of the GARCH model steps (warn at α + γ/2 + β ≥ 0.999):

| Model | Fits | Highest persistence | Fits at the bound | Status |
|---|---:|---:|---|---|
| GARCH, normal | 9 | 0.985974 | none | pass |
| GJR, normal | 9 | 1.000000 | 2020-10-22 (1.000000), 2021-10-20 (1.000000) | warn |
| GJR, Student-t 5 | 9 | 1.000000 | 2020-10-22 (1.000000), 2021-10-20 (1.000000) | warn |
| GARCH, filtered HS | 9 | 0.985974 | none | pass |
| GJR, filtered HS | 9 | 1.000000 | 2020-10-22 (1.000000), 2021-10-20 (1.000000) | warn |
| GJR, filtered HS, fit on latest 1,000 | 9 | 1.000000 | 2020-10-22 (1.000000), 2021-10-20 (1.000000) | warn |

Standardized returns r_t / σ_t on the evaluated days (what the volatility filter leaves to the distribution):

| Filter | Std | Excess kurtosis | 1% quantile | 99% quantile |
|---|---:|---:|---:|---:|
| EWMA | 1.049 | 2.12 | -3.12 | +2.13 |
| GARCH | 1.049 | 2.07 | -2.86 | +2.28 |
| GJR | 1.081 | 5.41 | -2.93 | +2.23 |
| GJR, fit on latest 1,000 | 1.049 | 5.76 | -2.79 | +2.17 |

Basel zone of the 99% VaR (exceptions) / Z2 zone of the 97.5% ES, by 250-day window:

| Window | EWMA, normal (champion) | EWMA, filtered HS (experiment 05) | GARCH, normal | GJR, normal | GJR, Student-t 5 | GARCH, filtered HS | GJR, filtered HS | GJR, filtered HS, fit on latest 1,000 |
|---|---|---|---|---|---|---|---|---|
| 2019-11-11 → 2020-11-05 | red (12) / red (-1.88) | green (3) / green (-0.62) | red (10) / yellow (-1.72) | yellow (7) / red (-2.16) | yellow (5) / yellow (-1.48) | green (4) / green (-0.30) | green (3) / green (-0.37) | green (3) / green (-0.37) |
| 2020-11-06 → 2021-11-03 | yellow (6) / green (-0.69) | green (1) / green (+0.33) | yellow (8) / green (-0.53) | yellow (5) / yellow (-0.70) | green (4) / green (-0.46) | green (1) / green (+0.41) | green (1) / green (+0.33) | green (1) / green (+0.33) |
| 2021-11-04 → 2022-11-01 | yellow (6) / yellow (-1.42) | green (2) / green (+0.52) | yellow (7) / yellow (-1.04) | yellow (7) / yellow (-0.83) | green (4) / green (-0.57) | green (2) / green (+0.30) | green (3) / green (+0.11) | green (3) / green (+0.24) |
| 2022-11-02 → 2023-10-31 | green (2) / green (+0.12) | green (0) / green (+0.77) | green (2) / green (+0.19) | green (3) / green (+0.04) | green (1) / green (+0.17) | green (0) / green (+0.86) | green (0) / green (+0.64) | green (0) / green (+0.77) |
| 2023-11-01 → 2024-10-29 | green (4) / yellow (-0.82) | green (1) / green (+0.42) | green (4) / green (-0.27) | green (4) / green (-0.50) | green (4) / green (-0.28) | green (0) / green (+0.58) | green (1) / green (+0.34) | green (0) / green (+0.50) |
| 2024-10-30 → 2025-10-29 | yellow (8) / yellow (-1.36) | green (4) / green (-0.46) | yellow (7) / yellow (-1.17) | yellow (6) / yellow (-1.07) | yellow (6) / green (-0.66) | yellow (5) / green (-0.42) | yellow (5) / green (-0.17) | green (3) / green (-0.21) |
| 2025-10-30 → 2026-10-28 | green (2) / green (-0.39) | green (2) / green (+0.63) | green (2) / green (+0.02) | green (2) / green (-0.10) | green (2) / green (+0.17) | green (2) / green (+0.48) | green (2) / green (+0.62) | green (2) / green (+0.64) |

99% VaR exception days (x = exception):

| Day | Return | ewma_normal | ewma_fhs | garch_normal | gjr_normal | gjr_t5 | garch_fhs | gjr_fhs | gjr_fhs_4y |
|---|---:|---|---|---|---|---|---|---|---|
| 2020-01-27 | -1.59% | x |  | x | x |  |  |  |  |
| 2020-01-31 | -1.79% | x |  | x |  |  |  |  |  |
| 2020-02-24 | -3.41% | x | x | x | x | x | x | x | x |
| 2020-02-25 | -3.07% | x |  |  |  |  |  |  |  |
| 2020-02-27 | -4.52% | x | x | x |  |  |  |  |  |
| 2020-03-09 | -7.90% | x |  | x | x | x |  |  |  |
| 2020-03-12 | -9.99% | x |  | x |  |  |  |  |  |
| 2020-03-16 | -12.77% | x |  |  |  |  |  |  |  |
| 2020-06-11 | -6.08% | x |  | x | x | x | x | x | x |
| 2020-09-03 | -3.58% | x | x | x | x | x | x | x | x |
| 2020-09-08 | -2.81% | x |  |  |  |  |  |  |  |
| 2020-10-26 | -1.88% |  |  | x | x |  |  |  |  |
| 2020-10-28 | -3.59% | x |  | x | x | x | x |  |  |
| 2021-01-04 | -1.49% |  |  | x | x | x |  |  |  |
| 2021-01-27 | -2.60% | x | x | x | x | x | x | x | x |
| 2021-02-25 | -2.48% | x |  | x | x | x |  |  |  |
| 2021-05-12 | -2.17% | x |  | x |  |  |  |  |  |
| 2021-06-18 | -1.32% |  |  | x | x | x |  |  |  |
| 2021-07-19 | -1.60% | x |  | x |  |  |  |  |  |
| 2021-09-20 | -1.71% | x |  | x |  |  |  |  |  |
| 2021-09-28 | -2.06% | x |  | x | x |  |  |  |  |
| 2021-11-26 | -2.30% | x | x | x | x | x | x | x | x |
| 2021-11-30 | -1.91% | x |  |  |  |  |  |  |  |
| 2022-01-05 | -1.96% |  |  | x | x | x |  | x | x |
| 2022-02-03 | -2.47% |  |  |  | x |  |  |  |  |
| 2022-03-07 | -3.00% |  |  | x | x |  |  |  |  |
| 2022-04-22 | -2.81% | x |  | x |  |  |  |  |  |
| 2022-04-29 | -3.70% | x |  |  |  |  |  |  |  |
| 2022-05-18 | -4.12% |  |  | x | x |  |  |  |  |
| 2022-08-26 | -3.43% | x |  | x | x | x |  |  |  |
| 2022-09-13 | -4.42% | x | x | x | x | x | x | x | x |
| 2022-12-15 | -2.52% |  |  | x | x |  |  |  |  |
| 2023-04-25 | -1.59% |  |  | x | x | x |  |  |  |
| 2023-08-02 | -1.39% | x |  |  | x |  |  |  |  |
| 2023-09-21 | -1.65% | x |  |  |  |  |  |  |  |
| 2023-12-20 | -1.48% |  |  |  | x | x |  |  |  |
| 2024-01-31 | -1.62% | x |  | x | x | x |  | x |  |
| 2024-07-17 | -1.40% | x |  |  |  |  |  |  |  |
| 2024-07-24 | -2.34% | x | x | x | x | x |  |  |  |
| 2024-08-05 | -3.04% | x |  | x |  |  |  |  |  |
| 2024-09-03 | -2.14% |  |  | x | x | x |  |  |  |
| 2024-10-31 | -1.88% | x |  | x | x | x | x | x |  |
| 2024-12-18 | -2.99% | x | x | x | x | x | x | x | x |
| 2025-02-21 | -1.72% | x |  | x | x | x |  |  |  |
| 2025-03-10 | -2.73% | x |  |  |  |  |  |  |  |
| 2025-04-03 | -4.96% | x | x | x | x | x | x | x | x |
| 2025-04-04 | -6.16% | x | x | x |  |  |  |  |  |
| 2025-08-01 | -1.61% | x |  | x | x | x | x | x |  |
| 2025-10-10 | -2.75% | x | x | x | x | x | x | x | x |
| 2026-01-20 | -2.08% | x | x | x | x | x | x | x | x |
| 2026-06-05 | -2.68% | x | x | x | x | x | x | x | x |
