## EAD, from Basel CRE52 to ORE 1.8.17, one convention at a time

| Step | CPTY_A | CPTY_B | CPTY_C | CPTY_D |
|---|---:|---:|---:|---:|
| Basel CRE52 (quantparity) | 9,967,637 | 3,952,441 | 8,861 | 205,722 |
| S and E: first and last payment dates, not the start and end of the swap | 9,808,615 | 3,902,701 | 8,166 | 205,722 |
| Equity and commodity notionals at the forward price, not the current price | 10,254,272 | 4,039,673 | 8,166 | 205,722 |
| S&P 500 as a single name: factor 32%, correlation 0.5 (index: 20%, 0.8) | 10,806,664 | 4,206,174 | 8,166 | 205,722 |
| Option delta with expiry and volatility swapped | 10,731,867 | 4,188,593 | 8,166 | 205,722 |
| Payer swaptions as puts | 10,185,111 | 4,024,711 | 8,166 | 205,722 |
| Margined maturity factor in weeks / 52, not business days / 250 | 10,185,111 | 3,978,537 | 8,007 | 205,722 |
| Margined EAD not capped at the unmargined EAD | 10,185,111 | 3,978,537 | 8,007 | 4,282,926 |
| IR add-on: signed sum per currency, no maturity buckets | 9,517,142 | 3,782,317 | -129,696 | 4,282,926 |
| Market value summed in USD, used as EUR | 9,883,317 | 4,148,492 | -129,696 | 4,282,926 |
| ORE 1.8.17 | 9,883,317 | 4,148,492 | -129,696 | 4,282,926 |

## Basel exposure by netting set (quantparity)

| Netting set | V | C | RC | Add-on | Multiplier | PFE | EAD | Unmargined EAD | ORE EAD |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| CPTY_A | 1,976,422 | 0 | 1,976,422 | 5,143,319 | 1.0000 | 5,143,319 | 9,967,637 | 9,967,637 | 9,883,317 |
| CPTY_B | 1,976,422 | 800,000 | 1,176,422 | 1,646,750 | 1.0000 | 1,646,750 | 3,952,441 | 8,847,637 | 4,148,492 |
| CPTY_C | -1,030,916 | 2,000,000 | 0 | 126,583 | 0.0500 | 6,330 | 8,861 | 42,336 | -129,696 |
| CPTY_D | 3,790 | 0 | 3,000,000 | 60,406 | 1.0000 | 60,406 | 205,722 | 205,722 | 4,282,926 |

## Hedging set add-ons: quantparity with ORE's conventions against ORE

| Netting set | Class | Hedging set | quantparity | ORE | Difference |
|---|---|---|---:|---:|---:|
| CPTY_A | IR | EUR | 82,407.95 | 82,408.62 | -6.73e-01 |
| CPTY_A | IR | GBP | -199,978.89 | -199,978.89 | -2.74e-07 |
| CPTY_A | IR | USD | -10,141.85 | -10,142.92 | 1.07e+00 |
| CPTY_A | FX | EURGBP | 143,603.92 | 143,603.92 | -6.96e-09 |
| CPTY_A | FX | EURUSD | 219,397.02 | 219,396.90 | 1.12e-01 |
| CPTY_A | FX | GBPUSD | 155,430.76 | 155,430.76 | -3.07e-07 |
| CPTY_A | Equity | Equity | 2,347,625.27 | 2,347,625.24 | 2.63e-02 |
| CPTY_A | Commodity | Energy | 1,341,861.72 | 1,341,861.71 | 5.71e-03 |
| CPTY_A | Commodity | Metal | 741,331.36 | 741,331.36 | -2.45e-04 |
| CPTY_B | IR | EUR | 25,162.65 | 25,162.84 | -1.98e-01 |
| CPTY_B | IR | GBP | -58,828.63 | -58,828.63 | -1.30e-07 |
| CPTY_B | IR | USD | -2,983.47 | -2,983.78 | 3.15e-01 |
| CPTY_B | FX | EURGBP | 59,419.03 | 59,419.03 | -6.01e-08 |
| CPTY_B | FX | EURUSD | 64,540.94 | 64,540.91 | 3.28e-02 |
| CPTY_B | FX | GBPUSD | 45,723.72 | 45,723.72 | -1.05e-07 |
| CPTY_B | Equity | Equity | 768,878.57 | 768,878.56 | 9.08e-03 |
| CPTY_B | Commodity | Energy | 405,239.80 | 405,239.80 | 1.65e-03 |
| CPTY_B | Commodity | Metal | 218,080.56 | 218,080.56 | -7.20e-05 |
| CPTY_C | IR | EUR | -92,639.83 | -92,639.83 | -4.06e-08 |
| CPTY_D | FX | EURUSD | 59,233.00 | 59,233.00 | -1.13e-07 |

## Trades, netting set CPTY_A: Basel against ORE

| Trade | δ Basel | δ ORE | d Basel | d ORE | MF ORE |
|---|---:|---:|---:|---:|---:|
| IR_EUR_9M | 1.0000 | 1.0000 | 14,687,887 | 4,870,517 | 0.8715 |
| IR_EUR_3Y | 1.0000 | 1.0000 | 41,839,239 | 34,476,212 | 1.0000 |
| IR_EUR_10Y | -1.0000 | -1.0000 | 78,652,647 | 73,743,963 | 1.0000 |
| IR_EUR_FWD | 1.0000 | 1.0000 | 56,930,894 | 53,387,416 | 1.0000 |
| IR_USD_5Y | 1.0000 | 1.0000 | 46,860,199 | 44,271,613 | 1.0000 |
| IR_GBP_7Y | -1.0000 | -1.0000 | 43,623,163 | 39,995,779 | 1.0000 |
| IR_SWPT_EUR | 0.9810 | -0.0503 | 42,058,637 | 37,402,193 | 1.0000 |
| IR_SWPT_USD | 0.3635 | -0.7601 | 62,846,676 | 60,911,130 | 1.0000 |
| FX_EURUSD_1Y | -1.0000 | -1.0000 | 10,155,987 | 10,155,987 | 1.0000 |
| FX_EURGBP_6M | 1.0000 | 1.0000 | 5,049,647 | 5,049,647 | 0.7110 |
| FX_GBPUSD_2Y | 1.0000 | 1.0000 | 3,885,769 | 3,885,769 | 1.0000 |
| FX_OPT_EURUSD | 0.5239 | 0.5749 | 8,124,790 | 8,124,790 | 1.0000 |
| EQ_OPT_SP5_C | 0.6286 | 0.6974 | 5,689,720 | 5,625,213 | 1.0000 |
| EQ_OPT_SP5_P | 0.3478 | 0.3462 | 3,793,146 | 3,777,704 | 0.7052 |
| EQ_FWD_LHA | 1.0000 | 1.0000 | 5,100,000 | 5,099,883 | 0.8684 |
| CO_FWD_WTI | 1.0000 | 1.0000 | 4,091,980 | 5,415,917 | 1.0000 |
| CO_FWD_GOLD | -1.0000 | -1.0000 | 4,089,948 | 4,118,508 | 1.0000 |
| CO_OPT_WTI | 0.6238 | 0.6347 | 2,727,986 | 3,524,769 | 0.9114 |

Per trade, quantparity with ORE's conventions against ORE's detail report (39 trades): largest difference 4.1e-05 in δ and 4.9e-08 in MF (ORE prints 4 decimals), 2.4e-08 relative in d.
