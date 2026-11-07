# Experiment 10 — SA-CCR against the Open Source Risk Engine

**Question.** quantparity 0.7.0 computes the SA-CCR exposure at default (Basel CRE52) and matches the five worked examples of BCBS 279. Those examples have nine trades and no option on equity, FX or commodities. On a larger portfolio, does a second implementation give the same exposures? And when it does not, can each gap be explained?

**Date run:** 2026-11-07 · **ModelFlow:** v0.26.0 · **Engine:** quantparity 0.7.0 · **Reference:** ORE (Open Source Risk Engine) 1.8.17.0, Python package `open-source-risk-engine`

## Reproduce

```bash
python3 fetch.py                                  # ORE's example market data (2016-02-05), tag 1.8.17.0, SHA-256 checked
python3 portfolio.py                              # ORE inputs: ore/Input/*.xml, ore/ore.xml
pip install open-source-risk-engine==1.8.17.0
python3 run_ore.py                                # ORE: prices and SA-CCR, 1 s; ore/Output
python3 compare.py                                # quantparity under CRE52 and under ORE's conventions; results/summary.md
MF=../../target/release/mf
$MF run saccr.toml && $MF verify saccr.toml       # the saccr step, reconciled with ORE's exposures
$MF report saccr.toml -o report.html
```

The five ORE reports `compare.py` reads are committed (`ore/Output`), so the comparison and the pipeline run without ORE. Two ORE runs give byte-identical reports.

## Set-up

[`portfolio.py`](portfolio.py) writes the trades in ORE's format, as of 5 February 2016. ORE prices them with the market data of its own examples.

| Netting set | Margin | Trades |
|---|---|---|
| CPTY_A | none | 18: six interest rate swaps in EUR, USD and GBP (9 months to 12 years, one forward-starting), two European swaptions, three FX forwards and an FX option, two S&P 500 options and a Lufthansa forward, two commodity forwards (WTI, gold) and a WTI option |
| CPTY_B | threshold 1M, MTA 250k, MPOR 2 weeks, IM 500k, VM 300k | the same 18 trades |
| CPTY_C | MPOR 2 weeks, IM 2M | two EUR swaps, out of the money: excess collateral |
| CPTY_D | MTA 3M, MPOR 2 weeks | one FX forward: the minimum transfer amount dominates the replacement cost |

Credit is left out: ORE 1.8.17 has no SA-CCR for credit default swaps ("not yet implemented for trade type CreditDefaultSwap"), and drops such trades from its netting sets, with their market values. The credit rules of quantparity are checked by the Basel examples 2 and 4.

[`compare.py`](compare.py) reads the trades from ORE's own input files, and takes from ORE's output only what a pricer gives: the market values, forward prices and payment dates. It builds the quantparity inputs twice:

- **Basel CRE52.** S and E are the start and end of the swap. Adjusted notionals use the current price (equity spot, the nearest quoted WTI and gold prices, as ORE's own commodity option engine does). The S&P 500 is an index. MPOR is 10 business days for 2 weeks.
- **ORE's conventions**, applied one at a time. This is the bridge below.

Times are Actual/Actual (ISDA) from the valuation date, ORE's day count for SA-CCR; CRE52 does not fix one.

## Results

Full tables: [`results/summary.md`](results/summary.md).

### The exposures differ, and not by rounding

| Netting set | quantparity (CRE52) | ORE 1.8.17 | ORE / CRE52 |
|---|---:|---:|---:|
| CPTY_A | 9,967,637 | 9,883,317 | 0.992 |
| CPTY_B | 3,952,441 | 4,148,492 | 1.050 |
| CPTY_C | 8,861 | −129,696 | negative |
| CPTY_D | 205,722 | 4,282,926 | 20.8 |

The `saccr` step of [`saccr.toml`](saccr.toml) recomputes the four netting sets and reconciles them with ORE's EAD, replacement cost and PFE: the four EADs fail (run `c38675be`, replayed identically).

The near-match on CPTY_A is a coincidence. Steps from −668,000 to +552,000 offset each other (see the bridge below).

### Each gap is one convention, and together they give ORE's figures back

quantparity's inputs are changed one convention at a time, until the exposures are ORE's:

| Step | CPTY_A | CPTY_B | CPTY_C | CPTY_D |
|---|---:|---:|---:|---:|
| Basel CRE52 (quantparity) | 9,967,637 | 3,952,441 | 8,861 | 205,722 |
| 1. S and E: first and last payment dates, not the start and end of the swap | 9,808,615 | 3,902,701 | 8,166 | 205,722 |
| 2. Equity and commodity notionals at the forward price, not the current price | 10,254,272 | 4,039,673 | 8,166 | 205,722 |
| 3. S&P 500 as a single name: factor 32%, correlation 0.5 (index: 20%, 0.8) | 10,806,664 | 4,206,174 | 8,166 | 205,722 |
| 4. Option delta with expiry and volatility swapped | 10,731,867 | 4,188,593 | 8,166 | 205,722 |
| 5. Payer swaptions as puts | 10,185,111 | 4,024,711 | 8,166 | 205,722 |
| 6. Margined maturity factor in weeks / 52, not business days / 250 | 10,185,111 | 3,978,537 | 8,007 | 205,722 |
| 7. Margined EAD not capped at the unmargined EAD | 10,185,111 | 3,978,537 | 8,007 | 4,282,926 |
| 8. IR add-on: signed sum per currency, no maturity buckets | 9,517,142 | 3,782,317 | −129,696 | 4,282,926 |
| 9. Market value summed in USD, used as EUR | 9,883,317 | 4,148,492 | −129,696 | 4,282,926 |
| ORE 1.8.17 | 9,883,317 | 4,148,492 | −129,696 | 4,282,926 |

After step 9, quantparity gives ORE's EAD to within 8 × 10⁻⁸ in relative terms on all four netting sets. On each of the 39 trades it also gives ORE's δ, d and MF to within ORE's printed precision. Each of the 20 hedging-set add-ons is within 1.1 of ORE's. What is left comes from the forward prices, which ORE reports to six decimals.

Steps 1 to 6 change only quantparity's inputs. Steps 7 to 9 change its aggregation and are recomputed in `compare.py` from quantparity's trade details. Nothing in the bridge is fitted.

### What ORE does differently

The source references are to tag 1.8.17.0, `OREAnalytics/orea/engine`.

**Errors**, where ORE contradicts CRE52 or its own code:

| # | Where | What | Effect here |
|---|---|---|---|
| 4 | `saccrtradedata.cpp:1523` | The supervisory delta of single options calls `phi(P, K, σ, T)` where the function is `phi(P, K, T, σ)` (line 93). The commodity swaption path, line 2250, has the right order. | Every option delta: the FX option 0.575 for 0.524, the S&P 500 call 0.697 for 0.629, the WTI call 0.635 for 0.624. |
| 5 | `saccrtradedata.cpp:1473` | A swaption whose fixed leg is paid is taken as a put. ORE's own swaps paying fixed have δ = +1, and its price of the swaption is that of a payer. | The long payer swaption, 1.2 points in the money, has δ −0.050 instead of +0.981. The short receiver has −0.760 instead of +0.364. |
| 8 | `saccrcalculator.cpp:280` | The IR add-on is 0.5% of Σ δ·d·MF per currency: the maturity-bucket formula is commented out (lines 277–278), and the sum keeps its sign. | Negative IR add-ons (GBP −199,979 on CPTY_A). CPTY_C has a total add-on of −92,640 and a **negative EAD**. |
| 9 | `saccrcalculator.cpp:166` | Trade market values are summed in USD (`amountUsd`) without conversion to the base currency, while collateral, thresholds and add-ons are converted to EUR. | V is overstated by the EUR/USD rate, 13%: RC +261,553 on CPTY_A. ORE's own SA-CCR example has it too (NPV 56,724 for trades worth 50,094 EUR). |

**Departures from the text, or readings of it:**

| # | Where | What | Effect here |
|---|---|---|---|
| 1 | `saccrtradedata.cpp:1031` | S is the first payment date, not the start of the period the contract references (CRE52). | A 9-month swap starting in 4 days has S = 0.51 instead of 0.01. Its adjusted notional falls from 14.7M to 4.9M. |
| 2 | lines 1490, 1707, 1782 | The adjusted notional of equity and commodity trades is units × forward price. CRE52 says the current price. | WTI 1Y forward: 40.88 USD against a current price of 30.89. The notional is 32% higher. |
| 3 | line 1000 | An equity is never an index: the index flag from reference data is a TODO. | The S&P 500 gets the single-name factor and correlation: +552,392 on CPTY_A. |
| 6 | line 1688 | The margined maturity factor is 1.5 √(MPOR in weeks / 52). It ignores the remargining frequency. | 0.2942 instead of 0.3000 for 2 weeks (10 business days). |
| 7 | `saccrcalculator.cpp:394` | No cap of the margined EAD at the unmargined EAD (CRE52). | CPTY_D: 4,282,926 instead of 205,722. The 3M minimum transfer amount counts fully in the RC of a set whose unmargined exposure is 205,722. |

## Findings

1. **quantparity's SA-CCR is confirmed beyond the Basel examples, by an independent implementation.** With each of ORE's conventions, quantparity reproduces ORE to 10⁻⁷ on 39 trades, four asset classes and four margin agreements. The gaps come from ORE's conventions, not from quantparity's calculations.
2. **A reference implementation is not a reference by default.** ORE's SA-CCR has four errors in this release. Two of them change the sign of quantities: the swaption delta, and the IR add-on, which turns into a negative exposure. A validator who took ORE as the benchmark would have "corrected" a right implementation.
3. **Netting-set totals hide trade-level errors.** CPTY_A agrees within 0.85% while steps from −668,000 to +552,000 cancel out. Only a reconciliation trade by trade (δ, d, MF) and step by step (the bridge) tells the two implementations apart.
4. **Interpretations need to be stated.** The current versus forward price, S for a started period, and how an MPOR in weeks becomes business days are not errors. Each still moves the exposure by up to 5%. quantparity follows the text, and each choice is an input that can be set.

## What this experiment changed in ModelFlow

Nothing in the code: the `saccr` step reconciled the four netting sets as it is. One gap was found and added to the backlog: the step reconciles netting sets only. Trade-level reported values (δ, adjusted notional, MF), with tolerances, would have shown in the report what `compare.py` shows here.

## Limits

- One market date, ORE's example data. The prices are not the point here; the trades and their market values are the same in both engines.
- No credit derivatives in the comparison: ORE 1.8.17 does not support them in SA-CCR.
- Single-name options and plain forwards only. Basis, volatility and path-dependent trades, CDO tranches and large netting sets (ORE's 5,000-trade MPOR rule) are not compared.
- The bridge replays ORE's conventions in quantparity. Steps 7 to 9 are recomputed in Python in `compare.py`, since quantparity has no option to apply them. They are not code paths in quantparity.
