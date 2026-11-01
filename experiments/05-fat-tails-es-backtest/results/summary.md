Run `ba61765b896ba1a5`, 2231 days evaluated, every model on the same days.
Standardized returns of the EWMA filter: excess kurtosis 3.79, i.e. a Student-t with 5.6 degrees of freedom.

| Model | VaR 99% exceptions | Rate | Kupiec p | Christoffersen p (independence) | Basel windows red / yellow / green | ES 97.5% exceptions (expected) | Severity | Z2 | ES windows red / yellow / green | Mean VaR | Mean ES |
|---|---:|---:|---:|---:|---|---:|---:|---:|---|---:|---:|
| EWMA, normal (champion) | 53 | 2.38% | 2.9e-08 | 0.17 | 1 / 4 / 3 | 90 (55.8) | 1.23 | -0.98 (yellow) | 1 / 4 / 3 | 2.40% | 2.42% |
| EWMA, Student-t 5 | 43 | 1.93% | 9.4e-05 | 0.06 | 1 / 3 / 4 | 87 (55.8) | 1.06 | -0.66 (green) | 0 / 3 / 5 | 2.69% | 2.82% |
| EWMA, Student-t 4 | 42 | 1.88% | 1.9e-04 | 0.05 | 1 / 3 / 4 | 90 (55.8) | 1.02 | -0.64 (green) | 0 / 3 / 5 | 2.74% | 2.92% |
| EWMA, Student-t 3 | 43 | 1.93% | 9.4e-05 | 0.06 | 1 / 3 / 4 | 104 (55.8) | 0.94 | -0.75 (yellow) | 0 / 3 / 5 | 2.71% | 3.01% |
| Filtered HS, all residuals | 18 | 0.81% | 3.4e-01 | 0.01 | 0 / 0 / 8 | 51 (55.8) | 0.97 | +0.11 (green) | 0 / 0 / 8 | 3.51% | 3.70% |
| Filtered HS, latest 500 | 22 | 0.99% | 9.5e-01 | 0.02 | 0 / 1 / 7 | 58 (55.8) | 1.02 | -0.07 (green) | 0 / 1 / 7 | 3.48% | 3.45% |

Basel zone of the 99% VaR / Z2 zone of the 97.5% ES, by 250-day window:

| Window | EWMA, normal (champion) | EWMA, Student-t 5 | EWMA, Student-t 4 | EWMA, Student-t 3 | Filtered HS, all residuals | Filtered HS, latest 500 |
|---|---|---|---|---|---|---|
| 2018-11-12 → 2019-11-08 | yellow (6) / yellow (-0.81) | yellow (6) / green (-0.55) | yellow (6) / green (-0.50) | yellow (6) / green (-0.56) | green (2) / green (+0.13) | green (2) / green (+0.16) |
| 2019-11-11 → 2020-11-05 | red (12) / red (-1.88) | red (11) / yellow (-1.35) | red (11) / yellow (-1.39) | red (11) / yellow (-1.42) | green (3) / green (-0.62) | green (3) / green (-0.63) |
| 2020-11-06 → 2021-11-03 | yellow (6) / green (-0.69) | yellow (6) / green (-0.45) | yellow (6) / green (-0.40) | yellow (6) / green (-0.46) | green (1) / green (+0.33) | green (1) / green (+0.42) |
| 2021-11-04 → 2022-11-01 | yellow (6) / yellow (-1.42) | green (3) / yellow (-1.07) | green (3) / yellow (-1.00) | green (3) / yellow (-1.05) | green (2) / green (+0.52) | green (2) / green (+0.21) |
| 2022-11-02 → 2023-10-31 | green (2) / green (+0.12) | green (0) / green (+0.36) | green (0) / green (+0.27) | green (0) / green (-0.23) | green (0) / green (+0.77) | green (0) / green (+0.71) |
| 2023-11-01 → 2024-10-29 | green (4) / yellow (-0.82) | green (3) / green (-0.56) | green (3) / green (-0.51) | green (3) / green (-0.47) | green (1) / green (+0.42) | green (4) / green (-0.49) |
| 2024-10-30 → 2025-10-29 | yellow (8) / yellow (-1.36) | yellow (7) / yellow (-1.02) | yellow (6) / yellow (-0.95) | yellow (7) / yellow (-0.89) | green (4) / green (-0.46) | yellow (5) / yellow (-0.85) |
| 2025-10-30 → 2026-10-28 | green (2) / green (-0.39) | green (2) / green (-0.07) | green (2) / green (-0.15) | green (2) / green (-0.64) | green (2) / green (+0.63) | green (2) / green (+0.64) |
