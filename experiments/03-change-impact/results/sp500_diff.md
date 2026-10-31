### ModelFlow diff — FAIL: 2629 changes beyond tolerance in 2 steps; origin: ewma_model (version 0.1.0 → 1.0.0-t5)

Reference `champion.toml#1` (`dba385488f25df22`) · candidate `challenger.toml#2` (`16d75af46b7e3999`) · 3173 values compared, 2629 changed, 2629 beyond tolerance

| Step | State | Compared | Changed | Beyond tol. | Max rel. | Cause |
|---|---|---:|---:|---:|---:|---|
| `returns` | identical | 0 | 0 | 0 |  |  |
| `days` | identical | 0 | 0 | 0 |  |  |
| `var_hist_99` | identical | 3 | 0 | 0 |  |  |
| `ewma_model` | changed | 2486 | 2484 | 2484 | 10.75% | version 0.1.0 → 1.0.0-t5 **(origin)** |
| `backtest_hist` | identical | 305 | 0 | 0 |  |  |
| `backtest_ewma` | changed | 379 | 145 | 145 | 100.00% | upstream steps changed |

**Verdict changes**

| Path | Reference | Candidate |
|---|---|---|
| `backtest_ewma.basel_windows[4].status` | warn | pass |
| `backtest_ewma.basel_windows[4].zone` | yellow | green |

**Changes by field**

| Field | Changed | Beyond tol. | Max \|Δ\| | Max rel. | Mean Δ |
|---|---:|---:|---:|---:|---:|
| `ewma_model.var[*]` | 2481 | 2481 | 0.0149682 | 10.75% | +0.00272801 |
| `ewma_model.model` | 1 | 1 |  |  |  |
| `ewma_model.nu` | 1 | 1 |  |  |  |
| `ewma_model.quantile` | 1 | 1 |  |  |  |
| `backtest_ewma.exception_list[*].excess_loss` | 47 | 47 | 0.0123533 | 87.01% | -0.00266397 |
| `backtest_ewma.exception_list[*].var` | 47 | 47 | 0.0123533 | 10.75% | +0.00266397 |
| `backtest_ewma.exception_list[*]` | 10 | 10 |  |  |  |
| `backtest_ewma.by_year[*].exceptions` | 7 | 7 | 2 | 100.00% | -1.42857 |
| `backtest_ewma.by_year[*].rate` | 7 | 7 | 0.008 | 100.00% | -0.00568933 |
| `backtest_ewma.basel_windows[*].exceptions` | 6 | 6 | 3 | 100.00% | -1.66667 |
| `backtest_ewma.basel_windows[*].plus_factor` | 3 | 3 | 0.5 | 100.00% | -0.283333 |
| `backtest_ewma.basel_windows[*].status` | 1 | 1 |  |  |  |
| `backtest_ewma.basel_windows[*].zone` | 1 | 1 |  |  |  |
| `backtest_ewma.christoffersen.lr_cc` | 1 | 1 | 13.4225 | 41.22% | -13.4225 |

<details><summary>Largest changes</summary>

| | Path | Reference | Candidate | Δ (rel.) | Tolerance |
|---|---|---:|---:|---:|---|
| **!** | `ewma_model.model` | ewma | ewma-t | changed | exact |
| **!** | `ewma_model.nu` | — | 5 | added | exact |
| **!** | `ewma_model.quantile` | — | 2.60646 | added | exact |
| **!** | `ewma_model.var[1778]` | 0.0158407 | 0.0177481 | +0.00190738 (10.75%) | exact |
| **!** | `ewma_model.var[105]` | 0.00835674 | 0.00936297 | +0.00100624 (10.75%) | exact |
| **!** | `ewma_model.var[1202]` | 0.01691 | 0.0189461 | +0.00203613 (10.75%) | exact |
| **!** | `ewma_model.var[1630]` | 0.0187889 | 0.0210512 | +0.00226237 (10.75%) | exact |
| **!** | `ewma_model.var[1689]` | 0.0172879 | 0.0193695 | +0.00208164 (10.75%) | exact |
| **!** | `ewma_model.var[1848]` | 0.017323 | 0.0194089 | +0.00208586 (10.75%) | exact |
| **!** | `ewma_model.var[1974]` | 0.0167093 | 0.0187212 | +0.00201197 (10.75%) | exact |
| | `ewma_model`: 2474 more | | | | |
| **!** | `backtest_ewma.basel_windows[4].status` | warn | pass | changed | exact |
| **!** | `backtest_ewma.basel_windows[4].zone` | yellow | green | changed | exact |
| **!** | `backtest_ewma.exception_list[date=2018-03-22]` | {"date":"2018-03-22","excess_loss":0.0020251338… | — | removed | exact |
| **!** | `backtest_ewma.exception_list[date=2018-06-25]` | {"date":"2018-06-25","excess_loss":0.0002609820… | — | removed | exact |
| **!** | `backtest_ewma.exception_list[date=2020-09-08]` | {"date":"2020-09-08","excess_loss":0.0005341226… | — | removed | exact |
| **!** | `backtest_ewma.exception_list[date=2021-11-30]` | {"date":"2021-11-30","excess_loss":0.0006094750… | — | removed | exact |
| **!** | `backtest_ewma.exception_list[date=2022-04-22]` | {"date":"2022-04-22","excess_loss":0.0008131271… | — | removed | exact |
| **!** | `backtest_ewma.exception_list[date=2022-04-29]` | {"date":"2022-04-29","excess_loss":0.0025161289… | — | removed | exact |
| **!** | `backtest_ewma.exception_list[date=2023-08-02]` | {"date":"2023-08-02","excess_loss":0.0003359745… | — | removed | exact |
| **!** | `backtest_ewma.exception_list[date=2023-09-21]` | {"date":"2023-09-21","excess_loss":0.0006040419… | — | removed | exact |
| | `backtest_ewma`: 135 more | | | | |

</details>
