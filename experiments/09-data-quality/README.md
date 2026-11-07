# Experiment 09 — Data quality of a public loan file

**Question.** Experiment 08 validated IFRS 9 models on the Lending Club loan file. Before any model, a validator checks the data: BCBS 239 (principles 3 and 4) asks for complete, accurate risk data whose quality is measured, and SR 26-2 asks that data used in development be assessed for suitability. What does a declared set of data quality rules find on the 2.26 million loans of the raw file, and on the extract the models of experiment 08 read?

**Date run:** 2026-11-07 · **ModelFlow:** v0.25.0 · **Engine:** quantparity 0.6.0

## What this experiment added

A built-in step, `data_quality`. It reads the raw cells of a CSV or Parquet file, or a table produced by another step (a pandas frame from the Python SDK, for instance). Each `[[step.rule]]` checks one or more columns:

- **completeness:** `required`, `max_missing` (a share), and `missing_values` for sentinels such as −1 or 999;
- **type and range:** `numeric`, `min`, `max`;
- **allowed values:** `one_of`;
- **uniqueness:** `unique`, on the combination of the columns;
- **consistency between columns:** `not_above`, `not_below`, with an absolute `tolerance` for rounding;
- **conditional rules:** `when = { column, one_of }` restricts a rule to some rows;
- **tolerated share:** `max_rate`;
- **severity:** `fail` or `warn`.

The loading steps stop at the first bad cell. This step counts every violation, gives the line of the first five in the file, and profiles each column (missing share, minimum, maximum, mean). It also records the SHA-256 of the file it checked.

A rule change is a change of the check: rules and parameters are part of the step's code fingerprint.

## Reproduce

```bash
python3 ../08-ifrs9-credit/fetch.py        # if experiment 08 was not run: Lending Club file, 1.7 GB
MF=../../target/release/mf
$MF run quality.toml                       # 2 steps, 12 s
$MF verify quality.toml
$MF report quality.toml -o report.html     # one row per check, with the first lines that break it
python3 summarize.py                       # results/summary.md, results/summary.json
```

## Data

- **`raw`:** `accepted_2007_to_2018Q4.csv`, the file experiment 08 downloads: 2,260,701 records and 151 columns, status as of March 2019. The step records SHA-256 `3eae03c2…d1f36a`, the hash `fetch.py` checks after the download, so the run states which file it checked.
- **`extract`:** `loans.csv`, the 612,745 36-month loans issued in 2010–2015 that `fetch.py` writes for the models of experiment 08.

## Set-up

[`quality.toml`](quality.toml) declares 25 rules on 23 columns of the raw file and 11 rules on 9 columns of the extract. The thresholds come from Lending Club's published terms: loan amounts of 500 to 40,000 USD, rates of 5% to 31%, grades A–G and sub-grades A1–G5, the nine known loan statuses, FICO scores of 300 to 850, and ratios of 0% to 100%.

Where a first, naive rule failed for a reason the data explains, the refined rule sits next to it. The naive rule stays as a warning, so both results are in the same run.

## Results

Full tables: [`results/summary.md`](results/summary.md).

| Finding | Rows | What the rule says |
|---|---:|---|
| Lines that are not loans | 33 | `id` must be numeric: 32 lines "Total amount funded in policy code 1 (or 2): …" and 1 heading "Loans that do not meet the credit policy". They are left from joining 16 quarterly files, at lines 421,097, 421,098, 528,963 and on. Every other field is empty, so 10 required-field rules fail on the same 33 lines. |
| `member_id` empty | 2,260,701 (100%) | The borrower identifier was removed from the public file. It is not possible to check that a borrower appears once, or to link loans of the same borrower. |
| Joint applications: debt-to-income ratio above 100% | 2,561 | All of them are joint applications; 135 are 999, a sentinel. For a joint application, `dti` is the first borrower's ratio; the household's is `dti_joint`. Restricted to individual applications (2,139,958), `dti` passes, with one −1 as missing. On joint applications (120,710), `dti_joint` lies between 0% and 69.5%, but 4 are missing (lines 23,852, 62,798, 1,039,795, 1,055,330). |
| Joint applications: annual income of zero | 1,668 | 1,667 are joint applications, whose income is in `annual_inc_joint`. Individual applications: no zero income. |
| Principal repaid above the funded amount | 136 | By 0.01 to 0.25 USD, on fully paid loans of 2009–2014: rounding. None beyond a 0.50 USD tolerance. |
| Revolving utilization above 100% | 7,343 (0.32%) | Possible (balances above limits); kept as a warning. |
| Employment length missing | 146,940 (6.5%) | "n/a" in the file, above the 5% the rule allows; a warning. |
| Extract: last payment before issue | 470 (0.08%) | All are −1, the extract's code for "no payment". All 470 loans were charged off, and every repaid loan has a last payment. `outcomes.py` in experiment 08 handles −1 as the issue date. With −1 declared as missing, the rule passes. |
| Extract: principal repaid above funded | 72 | Rounding, within 0.50 USD. |

1. **The models of experiment 08 read clean data, by luck of a filter.** The 33 non-loan lines have no `term`, and `fetch.py` keeps 36-month loans only, so they never reached the extract. A loader keeping every line, for a PD model on all terms, would have read 32 totals, of up to 6.4 billion USD, as loans, or failed on them.
2. **Joint applications change the meaning of borrower fields.** They are 5.3% of the loans, 92% of them from 2017–2018. A scorecard using `dti` or `annual_inc` as such would see an income of zero and a ratio above 100% or of 999, where the joint columns are within normal ranges. The rule is only right when restricted with `when`. This is the kind of finding a validation of model inputs is meant to produce.
3. **Every failure points to lines that can be checked.** The step gives line numbers in the file: `sed -n 421097p accepted.csv` shows the first total. The rows stay in the output and the report, with the share and the limit of each check.
4. **12 s for both files** (1.6 GB and 34 MB, 8 cores), with a peak of 1.96 GB of memory: the checked columns are kept in memory, about 30 bytes a cell.

## What this experiment changed in ModelFlow

| Change | Why |
|---|---|
| `data_quality` step | No step checked the data a pipeline reads. |
| `missing_values` | Credit data codes missing values as −1 or 999; without it, the extract's −1 broke a date rule. |
| `when` | The debt-to-income ratio and income are only comparable within the same type of application. |
| `tolerance` | 136 loans repaid a few cents more than the funded amount. |
| Line numbers from the CSV reader, corrected for CRLF line endings | The extract has CRLF line endings, and the reader gave the line before. A quoted cell spanning lines is counted too. |
| The HTML report shows quality rules one check per row, with the first lines | The generic table showed nested checks as empty cells. |

## Limits

- Value rules only: no date formats or patterns, no checks across files (keys of one file present in another).
- A `when` condition is a list of values of one column, not a range or a combination of conditions.
- The checked columns are read into memory: a 10-million-line file with 30 checked columns needs about 9 GB.
- In the Python SDK, a pandas `NaN` cannot be recorded. A missing value must reach the step as `None`, for instance in an object column.
