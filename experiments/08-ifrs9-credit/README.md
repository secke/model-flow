# Experiment 08 — IFRS 9 validation of a consumer loan ECL model

**Question.** Market risk was the subject of experiments 01 to 07. Credit risk under IFRS 9 is a large part of a validation team's work: the expected credit loss (ECL) of a loan book, the PD, LGD and EAD models behind it, and the staging between 12-month and lifetime ECL. What can a validator check with ModelFlow on a real loan book, and what does it find on a simple model?

**Date run:** 2026-11-06 · **ModelFlow:** v0.22.0 · **Engine:** quantparity 0.5.0

## What this experiment added

| quantparity 0.5.0 | Reference check ([`validation/RESULTS.md`](https://github.com/secke/quantparity/blob/main/validation/RESULTS.md)) |
|---|---|
| `ifrs9`: ECL of a facility for stages 1, 2 and 3, from a PD curve (flat hazard, empirical cumulative PDs by year, or the powers of a rating migration matrix), an LGD, an exposure profile (bullet, linear, annuity) and discounting at the effective interest rate | independent numpy implementation on 600 random facilities: 9.8e-15 in relative terms; migration matrices against `numpy.linalg.matrix_power`: 4.4e-16 |
| LGD and CCF backtests: one-sided t-test of realized − estimated by segment and overall (the test of the ECB's instructions for reporting validation results, 2019), the concordance of estimated and realized values, (1 + Somers' D) / 2, and realized CCFs from limits and drawn amounts | `scipy.stats.ttest_1samp` and `scipy.stats.somersd`: 8.8e-14 and 0. On realized LGDs with the U shape of observed recoveries, the test rejects 4.2% to 4.7% of correct estimates at 5% |
| Staging: the stage of each facility recomputed from the default definition, the days-past-due backstops, a watch list or forbearance flag, and the PD increase since origination (relative and absolute thresholds, low credit risk exemption); comparison with the reported stage, and how many of the next year's defaults were in stage 2 beforehand | the same rules in numpy on 20,000 random facilities: identical table |

ModelFlow exposes them as four built-in steps: `ecl` (recomputation, reconciliation with the reported ECL, comparison with realized losses, by stage and by segment), `lgd_backtest`, `ccf_backtest` and `staging_check`.

## Reproduce

```bash
python3 fetch.py                                # Lending Club loans: 1.7 GB download, then about a minute
MF=../../target/release/mf
$MF run ifrs9.toml                              # 14 steps, 90 s
$MF verify ifrs9.toml
python3 summarize.py                            # results/summary.md, results/summary.json
pip install numpy pandas && python3 reference/ecl_check.py   # results/reference_ecl.md
```

## Data

The Lending Club loan file: every loan issued from 2007 to 2018Q4, with its status as of March 2019. It is published under CC0 on Hugging Face ([`codesignal/lending-club-loan-accepted`](https://huggingface.co/datasets/codesignal/lending-club-loan-accepted)); `fetch.py` checks its SHA-256. The experiment keeps the 612,745 36-month loans issued from 2010 to 2015 that have reached a final status: fully paid, charged off or in default. Only 147 such loans were still running in March 2019, and they are left out.

The file has no payment history, only the date of the last payment. [`model/outcomes.py`](model/outcomes.py), the first step of the pipeline, dates each default at 90 days past due: four months after the last payment. It takes the principal not repaid as the exposure at default, and computes the realized LGD from the recoveries net of collection fees. Recovery dates are not in the file, so the realized LGD is undiscounted, and it understates the economic LGD.

## Set-up

The model under validation, [`model/ifrs9_model.py`](model/ifrs9_model.py), is the kind a small lender might build. It is estimated on the 2010–2012 vintages (66,727 loans):

- **PD:** the cumulative default rate by grade (A to G) after one, two and three years.
- **LGD:** the mean realized LGD of the defaults, by grade.
- **ECL:** stage 1 at origination, so 12-month PD × LGD × amount lent. There is no amortization and no discounting.

The pipeline [`ifrs9.toml`](ifrs9.toml) validates it out of time, on the 546,018 loans of the 2013–2015 vintages. Each loan is taken at its issue date, when it is in stage 1:

- **`ecl`** recomputes the ECL with the model's PD curves and LGDs. It uses monthly periods with a constant hazard rate within each year, the instalments of the loan, and discounting at its rate. It then reconciles the result with the model's figure. Two more `ecl` steps repeat the computation, one with the model's own conventions and one without discounting.
- **ECL against losses:** the realized losses, discounted to the issue date at the loan rate, are compared with the 12-month and lifetime ECL, by grade and by vintage.
- **`pd_calibration`**, run twice, compares the PDs with the observed default rates by grade, over 12 months and over the life of the loan. `pd_discrimination` measures how well the lifetime PD ranks the loans.
- **`lgd_backtest`**, run three times, compares the LGDs with the realized LGDs of the 76,818 test defaults, by grade and by year of default, then by year of default on the 71,398 defaults whose workout had lasted at least 12 months at the data date (`min_workout_months`).

## Results

[`results/summary.md`](results/summary.md), [`results/summary.json`](results/summary.json), [`results/reference_ecl.md`](results/reference_ecl.md). Run fingerprint `acbc90b37a0a6a6b`, replayed identically by `mf verify`.

The ECL recomputed by ModelFlow agrees with an independent pandas / numpy computation from the raw file (default dates, LGDs, PD curves, ECL) to 4e-13 in relative terms, in total and by grade.

| ECL of the test book at origination | Million USD | Against the model |
|---|---:|---:|
| The model's figure: 12-month PD × LGD × amount lent | 184.9 | |
| Recomputed with the model's conventions | 184.9 | 0 on every loan |
| Monthly instalments, no discounting | 160.8 | −13.0% |
| **Monthly instalments, discounting at the loan rate** | **151.0** | **−18.3%** |

| Segment | 12-month ECL | Realized, first 12 months | Ratio | Lifetime ECL | Realized, lifetime | Ratio |
|---|---:|---:|---:|---:|---:|---:|
| grade A | 18.9 | 8.4 | 0.44 | 47.4 | 36.3 | 0.76 |
| grade B | 42.3 | 27.6 | 0.65 | 107.9 | 104.8 | 0.97 |
| grade C | 49.6 | 41.6 | 0.84 | 115.8 | 135.6 | 1.17 |
| grade D | 28.2 | 32.7 | 1.16 | 63.3 | 88.1 | 1.39 |
| grade E | 9.0 | 13.3 | 1.48 | 19.3 | 33.0 | 1.72 |
| grade F | 2.4 | 3.2 | 1.31 | 3.7 | 6.6 | 1.77 |
| grade G | 0.5 | 0.7 | 1.30 | 0.8 | 1.1 | 1.46 |
| vintage 2013 | 27.7 | 19.9 | 0.72 | 65.9 | 62.6 | 0.95 |
| vintage 2014 | 45.3 | 34.7 | 0.77 | 106.8 | 113.1 | 1.06 |
| vintage 2015 | 78.0 | 72.9 | 0.94 | 185.5 | 229.7 | 1.24 |
| **all** | **151.0** | **127.5** | **0.84** | **358.2** | **405.4** | **1.13** |

(Million USD. Losses discounted to the issue date at the loan rate.)

**Findings.**

1. **The model's ECL is 22% too high, and the recomputation explains all of it.** The model's simplification overstates the ECL against the IFRS 9 measurement. The amount lent is not the exposure at default: a 36-month loan at 12% has repaid 30% of its principal by month 12, and its balance over the first year averages 87% of the amount lent. Amortization accounts for 13 points. The ECL must also be discounted at the effective interest rate (IFRS 9 5.5.17(b), B5.5.44), which accounts for the other 5. Recomputed with the model's own conventions, ModelFlow gives the model's figure back exactly, on every one of the 546,018 loans. So the gap comes from the method, not from the data or the code. A reconciliation that only compares totals could not have said so: the step reports the gap with each set of conventions, and the largest gaps loan by loan.
2. **The 12-month ECL covers the losses on aggregate but not by grade.** The losses of the first year are 84% of the recomputed 12-month ECL. By grade the ratio runs from 0.44 for grade A to 1.48 for grade E, because the model's PDs are too flat. The calibration test rejects the 12-month PD of grades D to G, and the lifetime PD of grades C to G (p ≤ 0.013; Hosmer-Lemeshow p < 1e-200). Grades A and B are overestimated. The grades of 2013–2015 separate the risks more than those of 2010–2012 did: grade A's first-year default rate halved, from 1.40% to 0.68%, while grade E's rose from 5.4% to 7.9%. A PD by grade, estimated on earlier vintages, carries the old grading scale into the new book.
3. **The lifetime ECL is 13% short, and 24% short for the latest vintage.** This is the figure a loan moves to when it goes to stage 2. Lifetime losses exceed it by 39% to 77% for grades D to F. Lifetime default rates rose from vintage to vintage, from 12.3% in 2013 to 14.9% in 2015, and a through-the-cycle PD curve from 2010–2012 does not follow them. IFRS 9 asks for forward-looking information (5.5.17(c)); the model has none. As a credit risk test, this is the most serious finding. The 12-month figure hides it, because it looks conservative on aggregate.
4. **The LGD is conservative but does not rank.** The model's LGD by grade, 87.6% to 91.6%, is higher than the realized one overall: 89.0% against 90.5%, t = −32. Only grade F is underestimated, by one point (p = 0.002). But the concordance of estimated and realized LGDs is 0.518, close to 0.5, which means no ranking power. Realized LGDs of these unsecured loans pile up at the top: 25% recover nothing, 90% lose at least 80%, and the median is 91%. An LGD by grade is then little more than a constant. To rank defaults, an LGD model would need drivers the grade does not carry, such as the amount repaid before default or the time to default.
5. **Workouts near the data cut-off bias realized LGDs upward.** The 585 defaults dated 2019, in the last months before the March 2019 snapshot, have a realized LGD of 93.1%, against 87.4% to 89.8% for 2014–2018. Recoveries come after the charge-off, and those had not come in yet. The LGD backtest by year of default fails on 2019 for that reason, not because the model is wrong. A validator must leave out incomplete workouts, or the backtest will find a bias that is not there. With `min_workout_months = 12`, the step leaves out 5,420 defaults (all 585 of 2019, 4,835 of 2018) and 2019 no longer appears; the overall result does not change (89.1% realized, t = −30.5). The defaults of 2013 are also high (91.9%, p = 0.0008), but they are of another kind: all of them occur within twelve months of issue.
6. **Ranking power is moderate.** The lifetime PD by grade has an AUC of 0.657 and a Gini of 0.31, against a pass threshold of 0.40 (`warn`). That is low for a scorecard, but it is what seven grades can carry.

**What the data cannot test.** Staging needs each facility's days past due and current PD at the reporting date, and the file only has the last payment date and the March 2019 status. The `staging_check` and `ccf_backtest` steps are therefore validated in quantparity, on synthetic data, and not here. Lending Club loans are fully drawn at issue, so they have no CCF. Freddie Mac's single-family loan-level dataset has monthly delinquency and balances, which is enough for staging and SICR. It needs a (free) registration.

## What this experiment changed in ModelFlow

| Finding | Status |
|---|---|
| No credit risk step beyond PD discrimination and calibration | **added** in quantparity 0.5.0 / ModelFlow v0.22.0: `ecl`, `lgd_backtest`, `ccf_backtest`, `staging_check`, each reconciled with an independent reference |
| A reconciliation that only compares totals cannot say why the ECLs differ | **added**: `ecl` takes the conventions as parameters (`repayment`, `periods_per_year`, `discount`); rerun with the model's conventions, it isolates the methodological gap (finding 1) |
| An ECL checked against realized losses in total hides a miscalibration by grade | **added**: `ecl` with `segments` and `realized_12m` / `realized_lifetime` columns (findings 2, 3) |
| Recent defaults bias the LGD backtest | **added**: `lgd_backtest` takes a `workout_months` column and leaves out, and counts, the defaults with a shorter workout than `min_workout_months` (finding 5) |
