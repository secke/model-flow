# Experiment 03 — Change impact analysis with `mf diff`

**Question.** When a model, a methodology or the engine itself changes, can a validator see in one command *what* moved, *where the change started* and *whether it matters*, on real data? Experiments 01 and 02 needed custom analysis scripts to answer this kind of question; this one uses `mf diff` (v0.13) only.

**Date run:** 2026-10-31 · **ModelFlow:** v0.13.0 · **Engine:** quantparity `eccd513`

Three changes, three typical validation questions:

| Part | Change | Data | Question |
|---|---|---|---|
| A | Champion → challenger model | S&P 500, 2016–2026 (experiment 01) | What does a fat-tailed challenger change in the backtest and the regulatory verdicts? |
| B | Methodology fix in the market builder | ECB AAA curve, 58 month ends (experiment 02) | How far does the fix move the curve, by maturity band, beyond which tolerance? |
| C | Engine upgrade v0.12.0 → v0.13.0 | the 59 recorded runs of experiments 01 and 02 | Does the upgrade change any result? |

## What `mf diff` does

`mf diff <reference> <candidate>` compares two runs step by step and value by value. A run is a recorded run (`pipeline.toml`, `pipeline.toml#ID`, `#previous`), a run executed on the spot (`#fresh`) or a manifest file (`mf run -o run.json`). For each step it reports:

- **the cause**: version bump, code or parameter change, upstream change, new source data;
- **the origin**: the steps whose output changed although their inputs did not, i.e. where the change starts;
- **the changes by field** (`curve.pillars[*].zero_rate`: count, largest absolute and relative change, mean change), with arrays of objects matched by identifier (`trades[id=T1]`, `exception_list[date=2020-03-16]`, `pillars[maturity=10.0]`) rather than by position;
- **the verdict changes** (`status`, `zone`, `decision` fields);
- **the tolerances**: `[[tolerance]]` rules in the pipeline, `abs`, `rel` or `bp` (basis points) per path pattern, first match wins.

The exit code is 1 when a change exceeds its tolerance, which makes it a gate in CI (see the GitHub Action in the [main README](../../README.md#compare-runs-mf-diff)).

## Reproduce

```bash
python3 ../01-sp500-var-backtest/fetch.py      # S&P 500 returns (FRED)
python3 ../02-ecb-aaa-curve/fetch.py           # ECB curves
cd ../03-change-impact
# A. champion vs challenger
../../target/release/mf run champion.toml && ../../target/release/mf run challenger.toml
../../target/release/mf diff champion.toml challenger.toml
# B. methodology change, 58 dates (about 1 minute)
python3 ecb_methodology.py
# C. engine upgrade: replay the recorded runs of experiments 01 and 02 with the new engine
(cd ../01-sp500-var-backtest && ../../target/release/mf diff pipeline.toml#1 pipeline.toml#fresh)
```

Part C needs the runs recorded by experiments 01 and 02 with v0.12.0.

## A. Champion vs challenger on the S&P 500

The champion is the pipeline of experiment 01 (RiskMetrics EWMA, normal quantile). The challenger ([`challenger.toml`](challenger.toml)) differs by one step: the same EWMA volatility with a Student-t quantile, 5 degrees of freedom, scaled to unit variance ([`models/ewma_t_var.py`](models/ewma_t_var.py)). It is the challenger that experiment 01 recommended.

Run fingerprints: champion `dba38548…`, challenger `16d75af4…`. The diff ([full output](results/sp500_diff.txt), [Markdown](results/sp500_diff.md)):

```
step             state       compared  changed  breaches  max rel  cause
var_hist_99      identical          3        0         0
ewma_model       changed         2486     2484      2484   10.75%  version 0.1.0 → 1.0.0-t5  ← origin
backtest_hist    identical        305        0         0
backtest_ewma    changed          379      145       145  100.00%  upstream steps changed
FAIL: 2629 changes beyond tolerance in 2 steps; origin: ewma_model (version 0.1.0 → 1.0.0-t5)
```

What the diff shows, and what a validator concludes:

| | Champion (normal) | Challenger (Student-t 5) |
|---|---:|---:|
| VaR multiplier on the volatility | 2.326 | 2.606 (every forecast +12.0%) |
| Exceptions, 2,481 days | 57 (2.30%) | 47 (1.89%) |
| Kupiec POF p-value | 2.8e-8, fail | 6.8e-5, fail |
| Christoffersen independence p-value | 0.19 | 0.07 |
| Basel windows red / yellow / green | 1 / 5 / 3 | 1 / 4 / 4 |
| Window 2021-11-04 → 2022-11-01 | yellow (6 exceptions, add-on 0.50) | **green** (3, add-on 0) |
| Current window | green (2) | green (2) |

1. **The change is isolated.** The diff traces every difference to one step, `ewma_model`, and checks that the historical VaR and its backtest are untouched: the comparison is a controlled experiment, not two pipelines that happen to differ.
2. **The challenger removes 10 of the 57 exceptions but does not fix the model.** The fat tail moves every forecast up by the same 12.0% (the diff's "max rel" of 10.75% is relative to the larger value, 0.120 / 1.120); Kupiec still rejects the exception rate (1.89% against 1%) and the Covid window stays red (11 exceptions). One regulatory window turns green. With an excess kurtosis of 16.7 (experiment 01), five degrees of freedom are still too thin; the next challenger should change the volatility filter, not only the quantile.
3. **Exceptions are compared by date.** Because `exception_list` is matched on its `date` field, the diff lists the 10 exception days that disappear (e.g. 2018-03-22, 2020-09-08, 2021-11-30) instead of 47 shifted positions.

## B. Methodology fix on the ECB curve, 58 dates

Version 1 of the market builder ([`models/market.py`](models/market.py)) reads the ECB "par yields" as annual-coupon par rates; version 2 derives annual-coupon par rates from the ECB spot rates (experiment 02 showed why). For each of the 58 month ends, [`ecb_methodology.py`](ecb_methodology.py) runs the pipeline with version 1, switches it to version 2, runs again, and calls `mf diff pipeline.toml` (previous vs latest run). The tolerances are declared in the pipeline ([`ecb_pipeline.toml`](ecb_pipeline.toml)), by maturity band:

```toml
[[tolerance]]            # below 10 years: 1 bp of rate
path = "*=?.0]*rate"
bp = 1.0

[[tolerance]]            # 10 years and beyond: 0.5 bp
path = "*rate"
bp = 0.5
```

Results ([per date](results/ecb_by_date.csv), [summary](results/ecb_summary.json), [diff of the largest date](results/ecb_2026-09-30_diff.txt)):

| | |
|---|---|
| Values compared | 231,130 over 58 pairs of runs |
| Origin found | `market (version 1.0.0 → 2.0.0)` on all 58 dates, every other change classified as downstream |
| Pillar zero rates beyond tolerance | 1,595 of 1,740 |
| Largest zero-rate change | 7.52 bp (2026-09-30), identical to the naive-reading error measured in experiment 02 |
| Direction | upward on every date (mean change between +0.03 bp and +6.7 bp) |
| Dates where the curve stays within tolerance | 3, January to March 2022 (rates near zero: largest change 0.20 bp); the gate still fails there, on the spot-check counts |
| ECB spot check, trades failing | 20,020 → 2,929 (−85%), but the step's status stays `fail` on every date |

| Date | 10Y spot | Pillar zero rates beyond tol. | Largest change | Mean change | Largest forward change | Failing spot-check trades |
|---|---:|---:|---:|---:|---:|---|
| 2022-01-31 | 0.03% | 0 / 30 | 0.20 bp | +0.03 bp | 0.20 bp | 54 → 44 |
| 2022-06-30 | 1.50% | 21 / 30 | 1.52 bp | +1.12 bp | 1.77 bp | 337 → 51 |
| 2023-10-31 | 2.82% | 30 / 30 | 5.30 bp | +4.33 bp | 5.57 bp | 358 → 49 |
| 2024-01-31 | 2.27% | 30 / 30 | 4.10 bp | +2.81 bp | 4.10 bp | 358 → 65 |
| 2026-09-30 | 3.58% | 30 / 30 | 7.52 bp | +6.68 bp | 8.87 bp | 358 → 35 |
| 2026-10-28 | 3.39% | 30 / 30 | 6.68 bp | +6.00 bp | 7.80 bp | 357 → 29 |

Tolerances have to be consistent with each other. A first version also had a relative tolerance of 1e-4 on discount factors: at 30 years that is tighter than 0.5 bp of rate (dDF/DF = −t·dr), so on the zero-rate dates the gate failed on discount factors while every rate was within tolerance. Discount factors are now excluded, since they are redundant with zero rates.

1. **One command replaces the analysis script of experiment 02.** The pillar error that experiment 02 measured with a custom script (7.52 bp at most) comes out of the diff, by field and in basis points, with its origin.
2. **Tolerances by maturity band turn a methodology change into a decision.** The fix is immaterial at zero rates (3 dates within tolerance) and material everywhere else; the size of the effect follows the level of rates, as expected for a compounding error.
3. **A verdict hides an improvement that the values show.** The spot check fails before and after the fix, because maturities below 1 year are extrapolated (experiment 02, finding 4). A gate on statuses alone would see no change; the diff shows 85% fewer failing trades.

## C. Engine upgrade v0.12.0 → v0.13.0

The runs recorded with v0.12.0 in experiments 01 and 02 were executed again with v0.13.0 (`#fresh`) and compared:

| Runs | Values compared | Result |
|---|---:|---|
| S&P 500 backtest (experiment 01) | 8,233 | identical; [output](results/engine_upgrade_sp500.txt) |
| ECB curves (experiment 02), 58 dates | 697,740 | identical on 58 of 58 |

Every built-in step is reported as `same output` with the cause `code or parameters changed without a version bump (engine before 0.13 → modelflow 0.13.0)`: the engine is part of each built-in step's fingerprint, and v0.13 also fingerprints step parameters, so the code fingerprints changed while no result did. This is the regression test a bank needs before upgrading a validation engine, on its own recorded runs.

## What this experiment changed in ModelFlow

| Finding | Status |
|---|---|
| Changes of step parameters (λ, α, window…) were not fingerprinted: a parameter change was reported as "non-reproducible result" instead of a model change | **fixed** in v0.13.0: parameters are part of the code fingerprint |
| An engine upgrade was reported as "code changed without a version bump", with no way to tell it from a model change | **fixed**: manifests record the engine (`modelflow 0.13.0`) and the diff names the upgrade |
| Rate changes were printed as decimals (7.379e-4) | **fixed**: values with a tolerance in bp are shown in bp |
| Arrays were compared by position, so one added trade or exception shifted every following element | **fixed**: arrays of objects are matched by identifier |
| Comparing many dates still needs a loop around `mf diff` | open: validation over time (backlog item 3) |
