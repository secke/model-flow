# Lending Club IFRS 9 validation: run `acbc90b37a0a6a6b`

Test book: 546,018 loans of the 2013-2015 vintages, 6,942.4 million USD lent.

## The model: PD curves and LGD by grade (2010-2012 vintages)

| Grade | PD year 1 | Cumulative PD year 2 | Cumulative PD year 3 | LGD |
|---|---:|---:|---:|---:|
| A | 1.40% | 4.15% | 6.60% | 91.6% |
| B | 2.42% | 7.44% | 11.84% | 90.7% |
| C | 3.85% | 10.98% | 16.66% | 90.2% |
| D | 4.87% | 13.44% | 20.22% | 90.5% |
| E | 5.42% | 14.47% | 21.10% | 89.8% |
| F | 9.27% | 15.73% | 23.79% | 87.6% |
| G | 14.71% | 23.53% | 29.41% | 88.1% |

## ECL recomputation (stage 1 at origination: 12-month ECL), million USD

| Conventions | ECL | Against the model's figure |
|---|---:|---:|
| The model's: amount lent, annual period, no discounting | 184.9 | +0.0e+00 |
| Monthly instalments, no discounting | 160.8 | -13.0% |
| Monthly instalments, discounting at the loan rate (IFRS 9 5.5.17) | 151.0 | -18.3% |

Model's figure: 184.9. Step status: `fail`, 546,018 loans outside tolerance.

## ECL against realized losses, million USD (losses discounted to the issue date at the loan rate)

| Segment | Loans | 12-month ECL | Realized, first 12 months | Ratio | Lifetime ECL | Realized, lifetime | Ratio |
|---|---:|---:|---:|---:|---:|---:|---:|
| grade A | 122,508 | 18.9 | 8.4 | 0.44 | 47.4 | 36.3 | 0.76 |
| grade B | 185,498 | 42.3 | 27.6 | 0.65 | 107.9 | 104.8 | 0.97 |
| grade C | 146,158 | 49.6 | 41.6 | 0.84 | 115.8 | 135.6 | 1.17 |
| grade D | 67,726 | 28.2 | 32.7 | 1.16 | 63.3 | 88.1 | 1.39 |
| grade E | 19,740 | 9.0 | 13.3 | 1.48 | 19.3 | 33.0 | 1.72 |
| grade F | 3,949 | 2.4 | 3.2 | 1.31 | 3.7 | 6.6 | 1.77 |
| grade G | 439 | 0.5 | 0.7 | 1.30 | 0.8 | 1.1 | 1.46 |
| vintage 2013 | 100,422 | 27.7 | 19.9 | 0.72 | 65.9 | 62.6 | 0.95 |
| vintage 2014 | 162,570 | 45.3 | 34.7 | 0.77 | 106.8 | 113.1 | 1.06 |
| vintage 2015 | 283,026 | 78.0 | 72.9 | 0.94 | 185.5 | 229.7 | 1.24 |
| **all** | 546,018 | 151.0 | 127.5 | 0.84 | 358.2 | 405.4 | 1.13 |

## PD calibration by grade (binomial test, one-sided)

| Grade | Loans | 12-month PD | Observed | p-value | Lifetime PD | Observed | p-value |
|---|---:|---:|---:|---:|---:|---:|---:|
| A | 122,508 | 1.40% | 0.68% | 1.0e+00 | 6.60% | 5.29% | 1.0e+00 |
| B | 185,498 | 2.42% | 1.69% | 1.0e+00 | 11.84% | 11.10% | 1.0e+00 |
| C | 146,158 | 3.85% | 3.43% | 1.0e+00 | 16.66% | 18.06% | 4.1e-46 |
| D | 67,726 | 4.87% | 5.64% | 4.9e-20 | 20.22% | 23.75% | 1.8e-111 |
| E | 19,740 | 5.42% | 7.89% | 1.8e-47 | 21.10% | 29.20% | 2.1e-158 |
| F | 3,949 | 9.27% | 10.64% | 2.1e-03 | 23.79% | 33.53% | 1.3e-43 |
| G | 439 | 14.71% | 18.68% | 1.3e-02 | 29.41% | 41.46% | 5.2e-08 |

Hosmer-Lemeshow p-value: 3.8e-275 (12 months), 0.0e+00 (lifetime). Ranking of the lifetime PD: AUC 0.657, Gini 0.313, KS 0.246.

## LGD backtest by grade (t-test of realized - estimated, H0: the LGD is not too low)

| Segment | Defaults | Estimated | Realized | t | p-value | Status |
|---|---:|---:|---:|---:|---:|---|
| A | 6,481 | 91.6% | 89.0% | -14.1 | 1.0e+00 | pass |
| B | 20,586 | 90.7% | 89.2% | -17.4 | 1.0e+00 | pass |
| C | 26,399 | 90.2% | 89.0% | -16.3 | 1.0e+00 | pass |
| D | 16,082 | 90.5% | 88.9% | -16.8 | 1.0e+00 | pass |
| E | 5,764 | 89.8% | 88.7% | -6.8 | 1.0e+00 | pass |
| F | 1,324 | 87.6% | 88.6% | +2.9 | 2.0e-03 | fail |
| G | 182 | 88.1% | 88.7% | +0.7 | 2.5e-01 | pass |
| **all** | 76,818 | 90.5% | 89.0% | -32.0 | 1.0e+00 | pass |

Concordance of the estimated and realized LGDs: 0.518.

## LGD backtest by year of default (t-test of realized - estimated, H0: the LGD is not too low)

| Segment | Defaults | Estimated | Realized | t | p-value | Status |
|---|---:|---:|---:|---:|---:|---|
| 2013 | 506 | 90.4% | 91.9% | +3.2 | 8.5e-04 | fail |
| 2014 | 4,993 | 90.4% | 89.3% | -7.2 | 1.0e+00 | pass |
| 2015 | 14,378 | 90.4% | 88.5% | -22.0 | 1.0e+00 | pass |
| 2016 | 26,622 | 90.4% | 89.8% | -8.3 | 1.0e+00 | pass |
| 2017 | 21,690 | 90.5% | 88.7% | -19.6 | 1.0e+00 | pass |
| 2018 | 8,044 | 90.5% | 87.4% | -18.2 | 1.0e+00 | pass |
| 2019 | 585 | 90.6% | 93.1% | +4.5 | 3.7e-06 | fail |
| **all** | 76,818 | 90.5% | 89.0% | -32.0 | 1.0e+00 | pass |

Concordance of the estimated and realized LGDs: 0.518.

## LGD backtest by year of default, workouts of 12 months or more (t-test of realized - estimated, H0: the LGD is not too low)

| Segment | Defaults | Estimated | Realized | t | p-value | Status |
|---|---:|---:|---:|---:|---:|---|
| 2013 | 506 | 90.4% | 91.9% | +3.2 | 8.5e-04 | fail |
| 2014 | 4,993 | 90.4% | 89.3% | -7.2 | 1.0e+00 | pass |
| 2015 | 14,378 | 90.4% | 88.5% | -22.0 | 1.0e+00 | pass |
| 2016 | 26,622 | 90.4% | 89.8% | -8.3 | 1.0e+00 | pass |
| 2017 | 21,690 | 90.5% | 88.7% | -19.6 | 1.0e+00 | pass |
| 2018 | 3,209 | 90.5% | 86.6% | -14.5 | 1.0e+00 | pass |
| **all** | 71,398 | 90.4% | 89.1% | -30.5 | 1.0e+00 | pass |

Concordance of the estimated and realized LGDs: 0.519. Defaults left out, workout under 12 months at the data date: 5,420.
