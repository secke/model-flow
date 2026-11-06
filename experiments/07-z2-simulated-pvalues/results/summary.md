## `ewma_models.toml` (run `619f77c5d301dbf2`, 2231 days)

| Model | Z2 | Fixed zone | Simulated p | Simulated zone | Simulated 5% quantile (fixed: −0.70) | 0.01% quantile (fixed: −1.8) | Windows red / yellow / green, fixed | same, simulated |
|---|---:|---|---:|---|---:|---:|---|---|
| ewma_normal | -0.98 | yellow | < 1/100,000 | red | -0.23 | -0.53 | 1 / 4 / 3 | 1 / 4 / 3 |
| ewma_t5 | -0.66 | green | < 1/100,000 | red | -0.24 | -0.54 | 0 / 3 / 5 | 0 / 3 / 5 |
| ewma_t4 | -0.64 | green | 3.0e-05 | red | -0.24 | -0.58 | 0 / 3 / 5 | 0 / 3 / 5 |
| ewma_t3 | -0.75 | yellow | 3.5e-04 | yellow | -0.27 | -0.98 | 0 / 3 / 5 | 0 / 3 / 5 |
| fhs | +0.11 | green | 0.7611 | green | -0.23 | -0.54 | 0 / 0 / 8 | 0 / 0 / 8 |
| fhs_2y | -0.07 | green | 0.2680 | green | -0.22 | -0.53 | 0 / 1 / 7 | 0 / 1 / 7 |

250-day windows whose zone changes:

| Model | Window | Z2 | Fixed | Simulated (p) |
|---|---|---:|---|---|

## `garch_models.toml` (run `404386fc1504f239`, 1761 days)

| Model | Z2 | Fixed zone | Simulated p | Simulated zone | Simulated 5% quantile (fixed: −0.70) | 0.01% quantile (fixed: −1.8) | Windows red / yellow / green, fixed | same, simulated |
|---|---:|---|---:|---|---:|---:|---|---|
| ewma_normal | -0.91 | yellow | < 1/100,000 | red | -0.26 | -0.59 | 1 / 3 / 3 | 1 / 3 / 3 |
| ewma_fhs | +0.23 | green | 0.9314 | green | -0.26 | -0.61 | 0 / 0 / 7 | 0 / 0 / 7 |
| garch_normal | -0.64 | green | 3.0e-05 | red | -0.26 | -0.59 | 0 / 3 / 4 | 0 / 3 / 4 |
| gjr_normal | -0.75 | yellow | < 1/100,000 | red | -0.26 | -0.59 | 1 / 3 / 3 | 1 / 2 / 4 |
| gjr_t5 | -0.43 | green | 0.0048 | yellow | -0.27 | -0.63 | 0 / 1 / 6 | 0 / 1 / 6 |
| garch_fhs | +0.28 | green | 0.9617 | green | -0.25 | -0.60 | 0 / 0 / 7 | 0 / 0 / 7 |
| gjr_fhs | +0.22 | green | 0.9084 | green | -0.27 | -0.65 | 0 / 0 / 7 | 0 / 0 / 7 |
| gjr_fhs_4y | +0.28 | green | 0.9554 | green | -0.27 | -0.65 | 0 / 0 / 7 | 0 / 0 / 7 |

250-day windows whose zone changes:

| Model | Window | Z2 | Fixed | Simulated (p) |
|---|---|---:|---|---|
| gjr_normal | 2020-11-06 → 2021-11-03 | -0.70 | yellow | green (0.0510) |
