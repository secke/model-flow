Run `2d47b2234c6f1f5c`, modelflow 0.25.0.

## raw: accepted.csv, 2,260,701 rows, status fail

SHA-256 `3eae03c28fd9d2e8a076ebeb73507e8d4d0f44d90500decdb0936e0933d1f36a`

| Rule | Rows | Check | Count | Rate | Limit | First rows | Status |
|---|---:|---|---:|---:|---:|---|---|
| loan id |  | unique | 0 | 0.0000% | 0.00% |  | pass |
| loan id |  | missing | 0 | 0.0000% | 0.00% |  | pass |
| loan id |  | numeric | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| member_id |  | missing | 2,260,701 | 100.0000% | 0.00% | 2, 3, 4, 5, 6, … | warn |
| loan amount (Lending Club: 500 to 40,000 USD) |  | missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| loan amount (Lending Club: 500 to 40,000 USD) |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| loan amount (Lending Club: 500 to 40,000 USD) |  | min | 0 | 0.0000% | 0.00% |  | pass |
| loan amount (Lending Club: 500 to 40,000 USD) |  | max | 0 | 0.0000% | 0.00% |  | pass |
| funded_amnt |  | missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| funded_amnt |  | not_above loan_amnt | 0 | 0.0000% | 0.00% |  | pass |
| funded_amnt_inv |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| funded_amnt_inv |  | min | 0 | 0.0000% | 0.00% |  | pass |
| funded_amnt_inv |  | not_above funded_amnt | 0 | 0.0000% | 0.00% |  | pass |
| term |  | missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| term |  | one_of | 0 | 0.0000% | 0.00% |  | pass |
| interest rate (5% to 31%) |  | missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| interest rate (5% to 31%) |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| interest rate (5% to 31%) |  | min | 0 | 0.0000% | 0.00% |  | pass |
| interest rate (5% to 31%) |  | max | 0 | 0.0000% | 0.00% |  | pass |
| grade |  | missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| grade |  | one_of | 0 | 0.0000% | 0.00% |  | pass |
| sub_grade |  | missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| sub_grade |  | one_of | 0 | 0.0000% | 0.00% |  | pass |
| loan_status |  | missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| loan_status |  | one_of | 0 | 0.0000% | 0.00% |  | pass |
| issue_d |  | missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| annual_inc |  | missing | 37 | 0.0016% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| annual_inc |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| annual_inc |  | min | 0 | 0.0000% | 0.00% |  | pass |
| positive annual income |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| positive annual income |  | min | 1,668 | 0.0738% | 0.00% | 18204, 65622, 421707, 422323, 422381, … | warn |
| positive annual income, individual applications | 2,139,958 | numeric | 0 | 0.0000% | 0.00% |  | pass |
| positive annual income, individual applications | 2,139,958 | min | 0 | 0.0000% | 0.00% |  | pass |
| debt-to-income ratio, every loan (0 to 100%) |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| debt-to-income ratio, every loan (0 to 100%) |  | min | 2 | 0.0001% | 0.00% | 1014617, 1681350 | warn |
| debt-to-income ratio, every loan (0 to 100%) |  | max | 2,561 | 0.1133% | 0.00% | 8723, 10550, 13702, 22843, 55650, … | warn |
| debt-to-income ratio, individual applications | 2,139,958 | missing | 1 | 0.0000% | 0.10% | 1014617 | pass |
| debt-to-income ratio, individual applications | 2,139,958 | numeric | 0 | 0.0000% | 0.00% |  | pass |
| debt-to-income ratio, individual applications | 2,139,958 | min | 0 | 0.0000% | 0.00% |  | pass |
| debt-to-income ratio, individual applications | 2,139,958 | max | 0 | 0.0000% | 0.00% |  | pass |
| debt-to-income ratio, joint applications | 120,710 | missing | 4 | 0.0033% | 0.00% | 23852, 62798, 1039795, 1055330 | fail |
| debt-to-income ratio, joint applications | 120,710 | numeric | 0 | 0.0000% | 0.00% |  | pass |
| debt-to-income ratio, joint applications | 120,710 | min | 0 | 0.0000% | 0.00% |  | pass |
| debt-to-income ratio, joint applications | 120,710 | max | 0 | 0.0000% | 0.00% |  | pass |
| FICO range |  | fico_range_low: missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| FICO range |  | fico_range_low: numeric | 0 | 0.0000% | 0.00% |  | pass |
| FICO range |  | fico_range_low: min | 0 | 0.0000% | 0.00% |  | pass |
| FICO range |  | fico_range_low: max | 0 | 0.0000% | 0.00% |  | pass |
| FICO range |  | fico_range_high: missing | 33 | 0.0015% | 0.00% | 421097, 421098, 528963, 528964, 651666, … | fail |
| FICO range |  | fico_range_high: numeric | 0 | 0.0000% | 0.00% |  | pass |
| FICO range |  | fico_range_high: min | 0 | 0.0000% | 0.00% |  | pass |
| FICO range |  | fico_range_high: max | 0 | 0.0000% | 0.00% |  | pass |
| fico_range_low |  | not_above fico_range_high | 0 | 0.0000% | 0.00% |  | pass |
| revolving utilization (0 to 100%) |  | missing | 1,835 | 0.0812% | 0.10% | 1539, 3579, 5676, 6783, 8118, … | pass |
| revolving utilization (0 to 100%) |  | numeric | 0 | 0.0000% | 0.10% |  | pass |
| revolving utilization (0 to 100%) |  | min | 0 | 0.0000% | 0.10% |  | pass |
| revolving utilization (0 to 100%) |  | max | 7,343 | 0.3248% | 0.10% | 25, 50, 71, 204, 251, … | warn |
| emp_length |  | missing | 146,940 | 6.4998% | 5.00% | 57, 77, 95, 141, 143, … | warn |
| principal repaid within the funded amount, exactly |  | not_above funded_amnt | 136 | 0.0060% | 0.00% | 1326963, 1327558, 1327575, 1327785, 1328146, … | warn |
| principal repaid within the funded amount, to 0.50 USD |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| principal repaid within the funded amount, to 0.50 USD |  | min | 0 | 0.0000% | 0.00% |  | pass |
| principal repaid within the funded amount, to 0.50 USD |  | not_above funded_amnt | 0 | 0.0000% | 0.00% |  | pass |
| out_prncp |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| out_prncp |  | min | 0 | 0.0000% | 0.00% |  | pass |
| out_prncp |  | not_above funded_amnt | 0 | 0.0000% | 0.00% |  | pass |
| recoveries, and their collection fee within them |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| recoveries, and their collection fee within them |  | min | 0 | 0.0000% | 0.00% |  | pass |
| recoveries, and their collection fee within them |  | not_above recoveries | 0 | 0.0000% | 0.00% |  | pass |

## extract: loans.csv, 612,745 rows, status warn

SHA-256 `65bb64b0d7f8e987e9e8c1e08f6d6f9266ad3ca6083b516274492bf4cdf639ea`

| Rule | Rows | Check | Count | Rate | Limit | First rows | Status |
|---|---:|---|---:|---:|---:|---|---|
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | vintage: missing | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | vintage: numeric | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | issue_month: missing | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | issue_month: numeric | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | grade: missing | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | grade: numeric | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | funded: missing | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | funded: numeric | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | int_rate: missing | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | int_rate: numeric | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | defaulted: missing | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | defaulted: numeric | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | total_rec_prncp: missing | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | total_rec_prncp: numeric | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | recoveries: missing | 0 | 0.0000% | 0.00% |  | pass |
| vintage, issue_month, grade, funded, int_rate, defaulted, total_rec_prncp, recoveries |  | recoveries: numeric | 0 | 0.0000% | 0.00% |  | pass |
| vintage |  | one_of | 0 | 0.0000% | 0.00% |  | pass |
| grade |  | one_of | 0 | 0.0000% | 0.00% |  | pass |
| defaulted |  | one_of | 0 | 0.0000% | 0.00% |  | pass |
| int_rate |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| int_rate |  | min | 0 | 0.0000% | 0.00% |  | pass |
| int_rate |  | max | 0 | 0.0000% | 0.00% |  | pass |
| funded |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| funded |  | min | 0 | 0.0000% | 0.00% |  | pass |
| funded |  | max | 0 | 0.0000% | 0.00% |  | pass |
| last payment after issue |  | missing | 0 | 0.0000% | 0.00% |  | pass |
| last payment after issue |  | not_below issue_month | 470 | 0.0767% | 0.00% | 3278, 4554, 5323, 5354, 6471, … | warn |
| last payment after issue, -1 for no payment |  | missing | 470 | 0.0767% | 0.10% | 3278, 4554, 5323, 5354, 6471, … | pass |
| last payment after issue, -1 for no payment |  | not_below issue_month | 0 | 0.0000% | 0.00% |  | pass |
| a repaid loan has a last payment | 527,518 | missing | 0 | 0.0000% | 0.00% |  | pass |
| total_rec_prncp |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| total_rec_prncp |  | min | 0 | 0.0000% | 0.00% |  | pass |
| total_rec_prncp |  | not_above funded | 0 | 0.0000% | 0.00% |  | pass |
| recoveries |  | numeric | 0 | 0.0000% | 0.00% |  | pass |
| recoveries |  | min | 0 | 0.0000% | 0.00% |  | pass |
