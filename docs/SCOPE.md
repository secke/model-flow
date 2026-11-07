# ModelFlow — scope (option A: orchestrated pricing & models)

## Problem
In investment banks, models (pricing, VaR, PD/LGD) and their data are scattered across teams and tools. Versions are not tracked, results are not reproducible, and validation (SR 26-2, SS1/23, BCBS 239) is manual.

## Proposal
A **deterministic** orchestration layer, without generative AI, that plugs into existing systems (Murex, Orchestrade, in-house libraries):
- DAG pipeline: market data → model → results → validation;
- every step is versioned, its inputs and outputs are hashed (SHA-256);
- the run manifest enables an **identical replay** and proves the absence of drift;
- a battery of validation tests runs automatically, with a timestamped report.

> Positioning and roadmap: [VISION.md](VISION.md).

## Status (v0.26)
- `mf-core`: DAG, fingerprints of inputs, outputs and code, manifest, replay verification.
- [`quantparity`](https://github.com/secke/quantparity) (external dependency, cross-checked against QuantLib): historical VaR; Kupiec, Christoffersen, Basel traffic light; PSI; VaR / ES forecasts (EWMA, GARCH or GJR-GARCH volatility with normal or Student-t innovations, filtered historical simulation; GARCH reconciled with the Python `arch` package, warning on fits at the stationarity bound); Acerbi-Székely expected shortfall backtests, with Z2 p-values simulated from the forecast distribution (by default for `var_model` forecasts); AUC, Gini, KS; per-grade binomial or Jeffreys calibration and Hosmer-Lemeshow; SA-CCR; IFRS 9 ECL (PD curves, migration matrices, amortizing exposures, discounting at the effective interest rate), LGD and CCF backtests (incomplete workouts left out), staging check from the SICR criteria; reference pricers (BSM and Greeks, Black-76, American CRR, bonds, implied volatility), yield curves (deposit + swap bootstrap), interest rate swaps and zero-coupons with DV01, pricer benchmark with per-product tolerances.
- `mf-runtime`: TOML pipelines, CSV steps (series or table), built-in steps, external processes with a timeout, SQLite storage, HTML validation report.
- `mf-server`: REST API, hashed tokens, roles (model owner, validator, auditor, admin), validation decisions with segregation of duties, append-only audit log.
- `mf-cli`: `mf run | verify | runs | show | report | diff | series | alerts`.
- Run comparison (`mf diff`, API, GitHub Action): value-by-value diff of two runs, origin of a change, cause (version, code or parameters, data, engine), arrays matched by identifier, tolerances in abs / rel / bp per path pattern, exit code for CI.
- Python SDK (`modelflow`, PyO3): pipelines and Python functions as steps, executed and fingerprinted by the Rust engine; runs, replay, diff, reports.
- Step parameters are part of the code fingerprint; manifests record the engine version.
- Validation over time: dated pipelines (`${as_of}` in data paths), `mf run --as-of DATE|all|new` in parallel, dated references (`@date`), `mf series` (CLI, API, SDK), dated scheduled runs.
- Monitoring: the outputs that make the run status are chosen per pipeline (`status_from`, traffic lights as verdicts); alert rules on series (`one_of`, `above`, `below`, `changes`, `consecutive`) in the CLI (`mf alerts`), the API, the SDK and the server webhooks.
- Multi-curve rates: TARGET calendar, market conventions, €STR curve and OIS-discounted Euribor 6M projection, dated swaps.
- Content-addressed output storage with integrity checks; isolation of external processes; RFC 4180 CSV and Parquet.
- API: run queue, rate limiting, native TLS, OIDC single sign-on (groups to roles, segregation of duties).
- Portfolio risk: historical VaR/ES by full revaluation, sensitivities, stress tests with limits, P&L attribution (FRTB).
- Sensitivity analysis of any step (`sensitivity`): a shocked input or parameter, the model rerun in parallel, expected directions (monotonicity) and bounds on the change.
- Counterparty credit risk: SA-CCR exposure at default of derivative netting sets (`saccr`), checked against the worked examples of BCBS 279, and reconciliation with the reported EAD, RC and PFE.
- Data quality of model inputs (`data_quality`): rules on the raw cells of a file or a step's table (completeness with sentinels, type and range, allowed values, uniqueness, consistency between columns with a tolerance, conditional rules), every violation counted with its first lines, column profiles, the SHA-256 of the file checked.
- Model inventory (statuses, risk tiers, reviews), scheduled runs and webhook alerts.
- Validation findings: severity, remediation owner and deadline, remediation evidenced by a reproducible run, closure review by a validator, escalation of overdue findings, approval blocked by critical and high findings, governance report (JSON, Markdown).
- Validator web UI (SSO PKCE, triggering, report, replay, decisions, audit), tested end to end.
- PostgreSQL storage for history and governance (audit log protected by a trigger), SQLite kept for local use.
- Deployment: non-root Docker image, hardened docker-compose, Kubernetes manifests (Pod Security restricted, NetworkPolicy).
- Ten experiments on real data (S&P 500 VaR backtest, ECB AAA curve, change impact with `mf diff`, monthly monitoring of a frozen VaR model, fat-tailed challengers with ES backtests, GARCH and GJR volatility filters, Z2 p-values by simulation, IFRS 9 validation of a consumer loan ECL model on 612,745 Lending Club loans, data quality of the 2.26 million loans of the raw file, SA-CCR of 39 trades against the Open Source Risk Engine).
- Eight examples: equity desk (VaR), PD scorecard, SA-CCR exposures, validation of QuantLib on options and bonds, on a single curve and swaps, in multi-curve, multi-asset portfolio risk, and the equity desk written with the Python SDK.
- GitHub CI: format, clippy, tests (including PostgreSQL), example runs and reports as artifacts, diff action, Python wheel and tests, Docker image, UI tests.

Decision 1 settled: existing models are integrated as **external processes** (JSON on stdin/stdout).

## Known limitations
- Several replicas with PostgreSQL: the workspace (pipelines, data) must be shared between pods.
- Network isolation of external models depends on the deployment (NetworkPolicy); when run locally, they have network access.
- Rates: no swaptions, caps/floors, FX products, historical fixing days (past fixings) or calendars other than TARGET yet.

## Roadmap (8 weeks)
| Week | Deliverable |
|---|---|
| 1-2 | ~~External steps, CSV/Parquet, SQLite and PostgreSQL, timeout, isolation, content-addressed storage~~ (done) |
| 3-4 | ~~Christoffersen, traffic light, PSI, Gini/KS, PD calibration~~ (done); ~~challenger models, LGD / CCF backtests, IFRS 9 ECL and staging, sensitivity analysis~~ (done) |
| 5 | ~~Reference pricers, QuantLib validation, curves, swaps, OIS multi-curve, market conventions~~ (done); remaining: historical fixings, swaptions, caps/floors, FX |
| 6 | ~~HTML validation report~~ (done); remaining: PDF export, model inventory, signed audit trail |
| 7 | ~~REST API, roles, decisions, audit, queue, rate limiting, TLS, OIDC SSO, web UI, inventory, scheduling, alerts~~ (done) |
| 8 | ~~Docker image, docker-compose, Kubernetes manifests, image CI~~ (done); remaining: Helm chart, pilot |

## Open decisions
1. ~~Integration mode~~: external process chosen.
2. Storage: SQLite (pilot), then PostgreSQL.
3. License: open source core (Apache-2.0) + paid enterprise modules (business model).
