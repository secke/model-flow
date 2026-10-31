# ModelFlow

**Reproducible model validation for banks.** ModelFlow runs quantitative models (pricing, VaR, PD) as versioned pipelines, fingerprints every input, output and line of code with SHA-256, and replays any past run to prove that it gives exactly the same result. Validators decide in the tool, under segregation of duties, and every action lands in an append-only audit log.

Written in Rust, no generative AI. The pricing and statistics engine is [quantparity](https://github.com/secke/quantparity), an independent library cross-checked against QuantLib in CI.

[![CI](https://github.com/secke/model-flow/actions/workflows/ci.yml/badge.svg)](https://github.com/secke/model-flow/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

## The problem

In most banks, a model's validation evidence is a folder of spreadsheets and screenshots. Nobody can say for sure which data, which code version and which parameters produced the number in the committee pack, and re-running it a year later rarely gives the same answer. Supervisors ask for exactly that traceability: SR 26-2 in the US, SS1/23 in the UK, BCBS 239 for risk data.

ModelFlow makes the evidence mechanical:

- **Every run is sealed.** Inputs, outputs and code are hashed; the run manifest ties them together with a single `run_hash`.
- **Every result can be replayed.** `mf verify` re-runs the pipeline and points to the exact step whose data, code or result changed.
- **Every change is explained.** `mf diff` compares two runs value by value, finds the step where the change starts, and checks it against tolerances in absolute, relative or basis-point terms. A GitHub Action turns it into a merge gate for model code.
- **Every decision is governed.** Only a validator can approve, never their own run, and only if the replay is identical. Denials are logged too.
- **Models stay where they are.** Any executable that reads JSON on stdin and writes JSON on stdout is a model: Python, R, C++, a wrapper around a vendor library. From Python, the SDK runs plain functions as steps, in-process.

## 5-minute demo

Requires Rust 1.88+ and `git`. No Python or database needed.

```bash
git clone https://github.com/secke/model-flow.git && cd model-flow
cargo build --release && export PATH="$PWD/target/release:$PATH"

# 1. Run a multi-asset portfolio risk pipeline: full-revaluation VaR/ES, sensitivities, stress tests, FRTB PLA
mf run examples/portfolio-risk/pipeline.toml
#   var_es               v1.0.0    [WARN]     99% VaR = €483k, 88% of its €550k limit
#   stress               v1.0.0    [FAIL]     "Sovereign crisis" loses €1.92m, limit €1.5m
#   pnl_attribution      v1.0.0    [PASS]     Spearman 0.99997, KS 0.013: green zone
#   run #1  run_hash=192b2a2c6f87…

# 2. Replay it: same data, same code, same result, bit for bit
mf verify examples/portfolio-risk/pipeline.toml
#   OK: run #1 replayed identically (8 steps)

# 3. Tamper with one number in the market data history, then replay
sed -i.bak '201s/^200,[^,]*/200,-0.25/' examples/portfolio-risk/data/factor_history.csv
mf verify examples/portfolio-risk/pipeline.toml; echo "exit code: $?"
#   MISMATCH [history] non-reproducible result
#   MISMATCH [var_es] different inputs
#   MISMATCH [var_es] non-reproducible result
#   MISMATCH [pnl_attribution] different inputs
#   MISMATCH [pnl_attribution] non-reproducible result
#   exit code: 1

# 4. Ask what changed and where it started: run #1 against a run of the tampered data
mf diff examples/portfolio-risk/pipeline.toml#1 examples/portfolio-risk/pipeline.toml#fresh
#   history          changed     1 value    source data changed  ← origin
#   var_es           changed     4 values   upstream steps changed
#   pnl_attribution  changed     2 values   upstream steps changed
#   ! history["SPOT:SX5E"][199]   -0.0667063 → -0.25
#   ! var_es.es                   537,315 → 566,741   (+5.19%)
#   FAIL: 7 changes beyond tolerance in 3 steps; origin: history (source data changed)
mv examples/portfolio-risk/data/factor_history.csv.bak examples/portfolio-risk/data/factor_history.csv

# 5. Committee-ready HTML report (verdict, replay status, every test in detail)
mf report examples/portfolio-risk/pipeline.toml -o report.html
```

The full walkthrough, with the web UI, three roles (model owner, validator, auditor), the model inventory, scheduling and alerts, is in [docs/GETTING-STARTED.md](docs/GETTING-STARTED.md). It is checked end to end on a fresh clone.

## Architecture

```
            pipeline.toml (versioned steps, DAG)
                         │
   ┌─────────────────────▼──────────────────────┐
   │ mf-runtime   CSV / Parquet / JSON readers   │
   │              built-in validation steps ─────┼──► quantparity (pricing, risk, statistics)
   │              external processes (any lang.) │
   └─────────────────────┬──────────────────────┘
                         │ outputs
   ┌─────────────────────▼──────────────────────┐
   │ mf-core      SHA-256 fingerprints, manifest,│
   │              topological run, replay        │
   └─────────────────────┬──────────────────────┘
                         │
   content-addressed store (SQLite or PostgreSQL) ◄── integrity check on every read
                         │
         ┌───────────────┼───────────────────────────┐
      mf (CLI)     modelflow (Python SDK)     mf-server (REST API, web UI, OIDC,
      GitHub Action                           roles, decisions, audit, scheduler, alerts)
```

| Crate | Role |
|---|---|
| `mf-core` | DAG of versioned steps, fingerprints of inputs, outputs and code, run manifest, verified replay |
| `mf-runtime` | TOML pipelines, step kinds, external process isolation, run store (SQLite, PostgreSQL), HTML report |
| `mf-server` | REST API, bearer tokens and OIDC SSO, roles and segregation of duties, validation decisions, append-only audit log, model inventory, scheduler, webhook alerts, web UI |
| `mf-cli` | the `mf` binary: `run`, `verify`, `runs`, `show`, `report`, `diff` |
| `mf-python` | the `modelflow` Python package (PyO3): pipelines, Python functions as steps, runs, replay, diff, reports |
| [`quantparity`](https://github.com/secke/quantparity) | external dependency, pinned by git revision: BSM, Black-76, CRR, bonds, single and multi-curve (€STR / Euribor), VaR/ES, stress, FRTB PLA, Kupiec, Christoffersen, Basel traffic light, PSI, AUC/Gini/KS, PD calibration |

## Results

### Reference engine: parity with QuantLib 1.44

Reported by quantparity's own CI, which reconciles every function with QuantLib:

| Area | Cases | Max abs diff vs QuantLib |
|---|---:|---:|
| European options: price and Greeks | 432 × 6 | 3.0e-13 |
| Black-76 | 324 | 1.4e-12 |
| Implied volatility | 208 | 3.4e-12 |
| Fixed-rate bonds: price, durations, convexity | 108 × 4 | 2.6e-12 |
| €STR OIS curve, discount factors 1Y–30Y | 30 | 1.8e-13 |
| Euribor 6M swaps with €STR discounting, NPV on 10M EUR | 54 | 2.0e-6 EUR |
| American options, CRR 500 steps | 96 | 1.1e-3 ([explained](https://github.com/secke/quantparity#finding-quantlibs-crr-tree-does-not-use-the-martingale-probability)) |

### ModelFlow examples: QuantLib as the candidate pricer under validation

Each benchmark example runs QuantLib as an external process and compares it, trade by trade, with the reference engine (`pricing_benchmark` step):

| Example | Trades | Max abs diff | Verdict |
|---|---:|---:|---|
| `pricing-benchmark`: European options, Black-76, bonds | 9 | 4.5e-13 | pass |
| `pricing-benchmark`: American options (CRR, 800 steps) | 3 | 5.2e-5 | pass, gap explained by QuantLib's log-linearized tree probability |
| `rates-benchmark`: single-curve swaps, notionals 8–50m | 7 | 2.5e-6 EUR | pass |
| `rates-benchmark`: zero-coupons | 3 | 2.0e-7 | pass |
| `multicurve-benchmark`: €STR-discounted Euribor 6M swaps, 2–30Y | 8 | 3.6e-6 EUR | pass |

Moving the engine from ModelFlow's former in-house crates to quantparity changed no number: 55,731 values across the six examples are identical bit for bit. The upgrade to v0.13 was checked with `mf diff` itself: 56,668 values of the six examples and 706,000 values of the real-data experiments are identical.

### Real market data

[Experiments](experiments/README.md) run ModelFlow on public data, with every run fingerprinted and replayed: a 99% VaR backtest of the S&P 500 over ten years, the reconstruction of the ECB AAA yield curve at 58 month ends, and the impact of a challenger model, a methodology fix and an engine upgrade measured with `mf diff`. Each experiment lists what it changed in ModelFlow.

## Governance

### Roles and segregation of duties

| Role | Can |
|---|---|
| `model_owner` | register and edit their own models, trigger runs, replay |
| `validator` | replay, approve or reject runs (never their own), approve models |
| `auditor` | read everything, including the audit log; no action buttons |
| `admin` | administration; cannot hold the validator role at the same time |

- A decision is accepted only if the replay is identical; a rejection must be justified.
- A model can be approved only if the latest run of every linked pipeline is approved.
- A user who is both validator and owner/admin in the identity provider is rejected at sign-in.
- The configuration stores only SHA-256 fingerprints of service tokens.

### Audit trail

Every action, denials included, is appended to the audit log. On PostgreSQL a trigger rejects `UPDATE`, `DELETE` and `TRUNCATE`, so the log is append-only at the database level. Stored outputs are content-addressed and re-hashed on every read: tampering with the database is detected.

### Regulatory mapping

| Expectation | How ModelFlow supports it |
|---|---|
| **SR 26-2** (US model risk management): independent validation, effective challenge, model inventory | Model inventory with risk tiers and review dates; validator-only approval; challenger and benchmark pricers as pipeline steps; replayable evidence |
| **SS1/23** (UK PRA model risk management): model identification, governance, independent validation, use of model risk mitigants | Inventory with status workflow (development → in validation → approved → in use → retired); segregation of duties enforced by the server and at SSO level; overdue reviews flagged |
| **BCBS 239** (risk data aggregation and reporting): accuracy, integrity, traceability | Every input, output and piece of code fingerprinted; lineage through the DAG; integrity checks on stored results; append-only audit log |

ModelFlow is a tool that produces evidence; it does not make an institution compliant on its own.

## Pipelines

A pipeline is a TOML file of versioned steps. Bump a step's `version` when its code changes: replay flags code that changed without a version bump.

```toml
# Adapted from examples/var-equity/pipeline.toml
name = "equity-var"
model = "VAR-EQ-01"                 # links the pipeline to the model inventory
schedule = "0 6 * * 1-5"            # optional, 5-field cron, UTC

[[step]]
name = "returns"
kind = "csv"
version = "1.0.0"
path = "data/returns.csv"
column = "ret"

[[step]]
name = "ewma_model"                 # any executable: JSON on stdin, JSON on stdout
kind = "process"
version = "0.1.0"
deps = ["returns"]
command = ["python3", "models/ewma_var.py"]
code = ["models/ewma_var.py"]       # hashed into the code fingerprint
params = { lambda = 0.94, alpha = 0.99, warmup = 30 }

[[step]]
name = "backtest_ewma"
kind = "backtest_forecasts"
version = "1.0.0"
deps = ["returns", "ewma_model"]
params = { returns = "returns", forecasts = "ewma_model", alpha = 0.99 }
```

### Step kinds

| kind | Purpose | Parameters |
|---|---|---|
| `csv` | one numeric column (`column`) or a table (`columns`); RFC 4180 quoting | `path`, `delimiter` (`,`) |
| `parquet` | same for a Parquet file (numeric columns) | `path` |
| `json` | a JSON document (trades, prices, market data) | `path` |
| `process` | **external model, any language** | `command`, `code`, `timeout_secs` (300), `env`, `pass_env`, `memory_mb`, `cpu_secs`, `file_size_mb`, `max_output_mb` (256) |
| `var_historical` | historical VaR | `alpha` |
| `kupiec_backtest` | rolling backtest of historical VaR | `alpha`, `window`, `dates` (optional) |
| `backtest_forecasts` | backtest of a model's VaR forecasts | `returns`, `forecasts`, `alpha`, `dates` (optional) |
| `psi` | Population Stability Index | `baseline`, `current`, `column`, `bins` |
| `pd_discrimination` | AUC, Gini, KS | `score`, `default`, `min_gini_pass`, `min_gini_warn` |
| `pd_calibration` | per-grade binomial test, Hosmer-Lemeshow | `grade`, `pd`, `default` |
| `price_portfolio` | reference valuation and Greeks | `trades`, `market` |
| `pricing_benchmark` | candidate pricer vs. reference, per-product tolerances | `trades`, `candidate`, `reference` (optional), `market` (optional), `tolerances` |
| `curve_bootstrap` | curve construction, re-pricing of the input instruments, term structure | `curve`, `tolerance`, `grid_step` |
| `portfolio_var` | historical VaR and ES by full revaluation, contributions, limits | `trades`, `scenarios`, `market`, `label`, `var_confidence`, `es_confidence`, `var_limit`, `es_limit` |
| `sensitivities` | delta, vega, DV01 per pillar, yield sensitivity | `trades`, `market` |
| `stress_test` | named stress scenarios with loss limits | `trades`, `scenarios`, `market` |
| `pnl_attribution` | FRTB P&L attribution test (Spearman, KS, zones) | `hypothetical`, `risk_theoretical` or `sensitivities` + `scenarios` |

VaR backtests combine **Kupiec** (POF), **Christoffersen** (independence, conditional coverage) and the **Basel traffic light** (with the multiplier add-on over 250 days at 99%). Besides the summary statistics, both backtests list every exception (`exception_list`: date, return, VaR, excess loss), apply the traffic light to consecutive regulatory 250-day windows ending at the latest observation (`basel_windows`, `latest_basel_window`, with the multiplier add-on) and, when `dates` points to a step with `YYYYMMDD` dates aligned with the returns, break exceptions down by year (`by_year`). Every test returns `pass`, `warn` or `fail`. A pricing gap up to `abs + rel × |reference|` passes, up to 10 times that warns, beyond fails; a missing candidate price fails.

### External model protocol and isolation

The process receives `{"inputs": {<dep>: <output>}, "params": {...}}` on stdin and writes one JSON document on stdout. A non-zero exit code is a failure and stderr is reported.

- The environment is cleared: only `PATH`, `LANG`, `HOME`, `TMPDIR` and `PYTHONHASHSEED=0`, plus `env` (part of the fingerprint) and the variables named in `pass_env`.
- On Unix, resource limits apply: memory (`memory_mb`), CPU time (`cpu_secs`, default: the timeout), file size (`file_size_mb`), no core file.
- Output is capped (`max_output_mb`).
- The process runs in its own process group, killed when the step ends: no orphan survives.
- Network isolation comes from the deployment (egress-free NetworkPolicy).

### Mismatches reported by `verify`

| Message | Meaning |
|---|---|
| `non-reproducible result` | same inputs and code, different output |
| `different inputs` | data or an upstream step changed |
| `code or parameters changed without a version bump` | a file listed in `code`, the `params` or the engine version changed but `version` did not |
| `version a -> b` | the step version changed |

### Examples

| Directory | Content |
|---|---|
| `examples/var-equity` | historical VaR and an external EWMA model (Python), full backtests |
| `examples/pd-scorecard` | 7-grade PD model: Gini/KS, per-grade calibration, population stability |
| `examples/pricing-benchmark` | QuantLib 1.44 validated on 12 trades (European and American options, Black-76, bonds) |
| `examples/rates-benchmark` | EUR 6M single curve (deposits + swaps), re-pricing, QuantLib validated on 7 swaps and 3 zero-coupons |
| `examples/multicurve-benchmark` | €STR / Euribor 6M multi-curve with market conventions, QuantLib validated on 8 swaps (2 to 30 years) |
| `examples/portfolio-risk` | multi-asset portfolio: historical VaR/ES by full revaluation, sensitivities, stress tests with limits, PLA test |
| `examples/python-sdk` | the `var-equity` desk written in Python with the SDK, checked against the TOML pipeline with `mf.diff`, and a challenger model |

The three benchmark examples need `pip install QuantLib==1.44`.

## Compare runs (`mf diff`)

```bash
mf diff pipeline.toml                          # previous vs latest run
mf diff pipeline.toml#12 pipeline.toml#fresh   # recorded run 12 vs the code and data as they are now
mf diff champion.toml challenger.toml          # two pipelines with the same step names
mf run pipeline.toml -o baseline.json          # keep a reference manifest...
mf diff baseline.json pipeline.toml#fresh --format md   # ...and compare against it (text, md, json)
```

For each step, the diff gives its state (identical, same output, within tolerance, changed, added, removed), its cause (version bump, code or parameter change, upstream change, new source data, engine upgrade) and whether the change **starts** there: a step whose output changed although its inputs did not is the origin. Changes are summarized by field (`curve.pillars[*].zero_rate`: count, largest absolute and relative change, mean change) and changes of verdict (`status`, `zone`) are listed separately. Arrays of objects are matched by identifier (`trades[id=T1]`, `exception_list[date=2020-03-16]`, `pillars[maturity=10.0]`), so one new trade does not shift every following line.

Tolerances are declared in the pipeline (or in a file passed with `--tolerances`); the first rule whose path pattern matches applies:

```toml
[[tolerance]]
path = "*=?.0]*rate"   # rates of maturities below 10 years
bp = 1.0               # basis points (1 bp = 1e-4)

[[tolerance]]
path = "*rate"
bp = 0.5

[[tolerance]]
path = "portfolio.trades*.npv"
abs = 1.0              # or rel = 1e-6; abs and rel together: within either

[[tolerance]]
path = "*.pnl"
ignore = true
```

The exit code is 0 when every change is within tolerance, 1 otherwise, 2 on error. The server exposes the same comparison: `GET /api/pipelines/{name}/diff?left=1&right=2` (JSON, or Markdown with `&format=md`).

### GitHub Action

Re-run a pipeline on every pull request and fail it when a result moves beyond tolerance; the summary goes to the job page and, optionally, to a pull request comment:

```yaml
jobs:
  model-diff:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write        # only for comment: "true"
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: secke/model-flow@main
        with:
          pipeline: models/var/pipeline.toml
          baseline: models/var/baseline.json     # mf run models/var/pipeline.toml -o models/var/baseline.json
          comment: "true"
```

Inputs: `pipeline`, `baseline`, `tolerances`, `limit`, `fail-on-breach` (default `true`), `comment`, `mf` (a prebuilt binary). Outputs: `ok`, `verdict`, `report`.

## Python SDK

The `modelflow` package exposes the Rust engine to Python: the built-in steps, fingerprints, history, replay, diff and reports run in Rust, and Python functions are steps in their own right, executed in-process and fingerprinted with their source code and parameters.

```bash
pip install maturin && maturin develop --release      # from the repository root (wheel not on PyPI yet)
```

```python
import math
import modelflow as mf

p = mf.Pipeline("var-desk", workdir="examples/var-equity")
p.add("returns", "csv", path="data/returns.csv", column="ret")
p.add("var_99", "var_historical", deps=["returns"], params={"alpha": 0.99})

@p.step(version="1.0.0", params={"lam": 0.94})
def ewma(returns, lam):                  # dependencies arrive as arguments, params too
    s2, var = sum(x * x for x in returns[:30]) / 30, []
    for x in returns[30:]:
        var.append(2.326 * math.sqrt(s2))
        s2 = lam * s2 + (1 - lam) * x * x
    return {"start": 30, "var": var}     # lists, dicts, NumPy arrays, pandas objects

p.add("backtest", "backtest_forecasts", deps=["returns", "ewma"],
      params={"alpha": 0.99, "returns": "returns", "forecasts": "ewma"})

run = p.run()                            # executed by the engine, recorded in .modelflow/runs.db
run.outputs["backtest"]["traffic_light"]  # {'zone': 'yellow', ...}
p.verify()                               # [] : replayed identically
d = mf.diff(p.get(1), run, tolerances=[{"path": "ewma.var*", "rel": 1e-12}])
d.ok, d.verdict, d.origins               # also d.summary(), d.changes() (rows for pandas), d.to_markdown()
p.report("report.html")
mf.load("examples/var-equity/pipeline.toml").run()   # TOML pipelines too
```

Changing a function's code or `params` without changing `version` is reported by the replay and the diff, as for any model. A diff renders as Markdown in Jupyter. [`examples/python-sdk/var_desk.py`](examples/python-sdk/var_desk.py) rebuilds the `var-equity` pipeline in Python and checks with `mf.diff` that it reproduces the TOML pipeline to 1e-12, then compares a challenger.

Performance: a Python step returning one million values, its fingerprint and a historical VaR on it run in about 0.4 s; comparing two runs of one million values takes about 0.3 s (0.6 s with tolerances).

## Server, web UI and API

```bash
mf-server new-token                               # token + SHA-256 fingerprint
cp deploy/server.example.toml server.toml         # paste the fingerprints
mf-server serve --config server.toml              # web UI on http://127.0.0.1:8080/
curl -H "Authorization: Bearer $TOKEN" localhost:8080/api/pipelines
```

The web UI has no external dependency (no CDN, no remote font) and works on a closed bank network. It covers pipelines, runs with queue tracking, the run page (verdict seal, report, replay, decisions), the model inventory and the filterable audit log. Content is inserted as text only, under a strict CSP (`script-src 'self'`), and the report is shown in a sandboxed frame.

| Method | Path | Roles |
|---|---|---|
| GET | `/api/health`, `/api/ui-config` | public |
| GET | `/api/me`, `/api/pipelines`, `/api/pipelines/{p}/runs`, `…/runs/{id}`, `…/runs/{id}/report`, `…/runs/{id}/decisions`, `/api/jobs/{id}` | all |
| POST | `/api/pipelines/{p}/runs` (`?wait=true` to block) | `model_owner`, `admin` |
| POST | `/api/pipelines/{p}/runs/{id}/verify` | `validator`, `model_owner`, `admin` |
| POST | `/api/pipelines/{p}/runs/{id}/decisions` `{"decision": "approved" \| "rejected", "comment": "…"}` | `validator` |
| GET, POST | `/api/models`; PUT `/api/models/{id}`; POST `/api/models/{id}/status` | read: all; write: owner, admin; `approved`: validator |
| GET | `/api/audit?limit=100` | `auditor`, `admin` |

Runs are queued (`max_concurrent_runs`, default 2) and rate-limited per user (`rate_limit_per_minute`, default 120, `429` beyond). Jobs interrupted by a shutdown are marked failed at restart.

### Single sign-on (OIDC)

```toml
[oidc]
issuer = "https://login.microsoftonline.com/<tenant>/v2.0"   # Entra ID, ADFS, Okta, Keycloak…
audience = "api://modelflow"
roles_claim = "roles"            # or "groups", or a dotted path ("realm_access.roles" for Keycloak)
username_claim = "preferred_username"
client_id = "modelflow-web"      # web UI: authorization code + PKCE, public client, no secret
scopes = "openid profile api://modelflow/.default"
# jwks_file = "jwks.json"        # without network access to the identity provider
[oidc.role_mapping]
"ModelFlow.Owners" = "model_owner"
"ModelFlow.Validators" = "validator"
"ModelFlow.Auditors" = "auditor"
```

- Signatures are verified with the provider's keys (discovery, `jwks_url` or `jwks_file`); an unknown `kid` triggers a reload, at most every `jwks_refresh_secs` (60 s).
- RS256, PS256 and ES256 only; HMAC and `none` are rejected, which blocks algorithm-confusion attacks.
- Issuer, audience, expiry and `nbf` are checked, with a `leeway_secs` tolerance (60 s).
- LDAP / Active Directory goes through the identity provider's federation. Service tokens (`[[user]]`) keep working alongside SSO.

### Scheduling and alerts

```toml
# pipeline.toml
schedule = "0 6 * * 1-5"         # 06:00 UTC on weekdays

# server.toml
public_url = "https://modelflow.bank.example"   # links in alerts
[scheduler]
enabled = true
interval_secs = 30
[alerts]
webhook_url_env = "MODELFLOW_ALERT_WEBHOOK"    # or webhook_url = "…"
on = ["fail", "error"]                         # add "warn" for warnings
```

Scheduled runs are triggered as user `scheduler`. Each slot is claimed in the database, so with several instances on PostgreSQL exactly one runs it; after an outage only the latest missed slot is caught up. Alerts are JSON (`text`, `pipeline`, `run_id`, `status`, `link`) accepted by Slack, Mattermost and Teams Workflows, and every send is audited.

### Database

Without a `[database]` section the server uses SQLite (one database per pipeline plus a governance database, single instance). For production, PostgreSQL:

```toml
[database]
url_env = "MODELFLOW_DATABASE_URL"   # or url = "postgres://…"; sslmode=require or verify-full for TLS
max_connections = 10
```

The schema is created at startup under an advisory lock, so several instances can start together. The CLI also writes to PostgreSQL when `MODELFLOW_DATABASE_URL` is set.

## Deployment

```bash
docker build -t modelflow:0.13.0 .
docker run --rm modelflow:0.13.0 new-token          # fill in deploy/server.docker.toml
docker compose up --build                          # http://127.0.0.1:8080
kubectl apply -k deploy/k8s                        # after filling in deploy/k8s/secret.yaml
```

- **Image**: `mf` and `mf-server`, Python 3.12 and QuantLib, non-root user (10001); examples in `/opt/modelflow/examples`.
- **docker-compose**: read-only file system, no capabilities, `no-new-privileges`, port bound to 127.0.0.1.
- **Kubernetes** (`deploy/k8s`): Pod Security `restricted`, read-only non-root container, seccomp `RuntimeDefault`, probes on `/api/health`, configuration in a Secret, workspace on a persistent volume, NetworkPolicy with ingress from the ingress controller only and no egress. `networkpolicy-egress.yaml` (opt-in) opens DNS, PostgreSQL and the identity provider. Serve HTTPS directly with a `[tls]` section (`cert`, `key`) or put the server behind an HTTPS proxy.

## Development

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
MODELFLOW_TEST_DATABASE_URL=postgres://postgres:pw@127.0.0.1:5432/postgres cargo test --workspace
pip install "maturin[patchelf]" pytest numpy pandas && maturin develop --release && pytest python/tests
```

CI runs formatting, clippy, the tests on SQLite and PostgreSQL 16, the six examples end to end, the CLI against PostgreSQL, the diff action on an unchanged and a changed model, the Python wheel with its tests and the SDK example, the Docker image (read-only, no capabilities), Kubernetes manifest validation and the Playwright UI test (simulated SSO, owner → validator → auditor). See [CONTRIBUTING.md](CONTRIBUTING.md).

Further reading: [experiments on real market data](experiments/README.md), [product vision](docs/VISION.md), [scope and roadmap](docs/SCOPE.md).

## License

[Apache-2.0](LICENSE).
