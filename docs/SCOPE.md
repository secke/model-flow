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

## Status (v0.12)
- `mf-core`: DAG, fingerprints of inputs, outputs and code, manifest, replay verification.
- [`quantparity`](https://github.com/secke/quantparity) (external dependency, cross-checked against QuantLib): historical VaR; Kupiec, Christoffersen, Basel traffic light; PSI; AUC, Gini, KS; per-grade binomial calibration and Hosmer-Lemeshow; reference pricers (BSM and Greeks, Black-76, American CRR, bonds, implied volatility), yield curves (deposit + swap bootstrap), interest rate swaps and zero-coupons with DV01, pricer benchmark with per-product tolerances.
- `mf-runtime`: TOML pipelines, CSV steps (series or table), built-in steps, external processes with a timeout, SQLite storage, HTML validation report.
- `mf-server`: REST API, hashed tokens, roles (model owner, validator, auditor, admin), validation decisions with segregation of duties, append-only audit log.
- `mf-cli`: `mf run | verify | runs | show | report`.
- Multi-curve rates: TARGET calendar, market conventions, €STR curve and OIS-discounted Euribor 6M projection, dated swaps.
- Content-addressed output storage with integrity checks; isolation of external processes; RFC 4180 CSV and Parquet.
- API: run queue, rate limiting, native TLS, OIDC single sign-on (groups to roles, segregation of duties).
- Portfolio risk: historical VaR/ES by full revaluation, sensitivities, stress tests with limits, P&L attribution (FRTB).
- Model inventory (statuses, risk tiers, reviews), scheduled runs and webhook alerts.
- Validator web UI (SSO PKCE, triggering, report, replay, decisions, audit), tested end to end.
- PostgreSQL storage for history and governance (audit log protected by a trigger), SQLite kept for local use.
- Deployment: non-root Docker image, hardened docker-compose, Kubernetes manifests (Pod Security restricted, NetworkPolicy).
- Six examples: equity desk (VaR), PD scorecard, validation of QuantLib on options and bonds, on a single curve and swaps, in multi-curve, and multi-asset portfolio risk.
- GitHub CI: format, clippy, tests (including PostgreSQL), example runs and reports as artifacts, Docker image, UI tests.

Decision 1 settled: existing models are integrated as **external processes** (JSON on stdin/stdout).

## Known limitations
- Several replicas with PostgreSQL: the workspace (pipelines, data) must be shared between pods.
- Network isolation of external models depends on the deployment (NetworkPolicy); when run locally, they have network access.
- Rates: no swaptions, caps/floors, FX products, historical fixing days (past fixings) or calendars other than TARGET yet.

## Roadmap (8 weeks)
| Week | Deliverable |
|---|---|
| 1-2 | ~~External steps, CSV/Parquet, SQLite and PostgreSQL, timeout, isolation, content-addressed storage~~ (done) |
| 3-4 | ~~Christoffersen, traffic light, PSI, Gini/KS, PD calibration~~ (done); remaining: challenger model, LGD/EAD, sensitivity tests |
| 5 | ~~Reference pricers, QuantLib validation, curves, swaps, OIS multi-curve, market conventions~~ (done); remaining: historical fixings, swaptions, caps/floors, FX |
| 6 | ~~HTML validation report~~ (done); remaining: PDF export, model inventory, signed audit trail |
| 7 | ~~REST API, roles, decisions, audit, queue, rate limiting, TLS, OIDC SSO, web UI, inventory, scheduling, alerts~~ (done) |
| 8 | ~~Docker image, docker-compose, Kubernetes manifests, image CI~~ (done); remaining: Helm chart, pilot |

## Open decisions
1. ~~Integration mode~~: external process chosen.
2. Storage: SQLite (pilot), then PostgreSQL.
3. License: open source core (Apache-2.0) + paid enterprise modules (business model).
