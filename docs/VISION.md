# ModelFlow: product vision

## Positioning

ModelFlow is the platform for **governance, validation and independent control of models and valuations** for banks and asset managers. It plugs into existing systems (Murex, Orchestrade, Calypso, in-house libraries) without replacing them.

The product does not aim at front-to-back. The ambition is to become **the reference in this niche**, as Orchestrade or Murex did in theirs.

## What made these platforms successful, and our equivalent

| Success factor | At Orchestrade / Murex | At ModelFlow |
|---|---|---|
| A single data model | All asset classes in one model, instead of juxtaposed tools ([The Hedge Fund Journal](https://thehedgefundjournal.com/orchestrade-cross-asset-portfolio-risk-trading/)) | A single sealed register: data, code, versions and results of every model, whatever its type (market, credit, pricing) |
| Fast implementation | Implementations advertised as faster than legacy systems, many standard interfaces | One binary or Docker image, pipelines described in TOML, models plugged in as they are, in their own language |
| Openness | Open APIs, Excel integration | REST API, external models in any language, CSV, Parquet and JSON formats |
| Regulatory trust | Built-in regulatory reporting | Reproducibility by construction, segregation of duties, append-only log, in line with the spirit of SR 26-2, SS1/23 and BCBS 239 |
| Client references | Entry through one segment (funds for Orchestrade), then expansion | Entry through validation and model risk teams, then expansion to risk, valuation and audit teams |

## Value proposition per user

- **Validator**: replay and compare any run, apply a standard battery of tests, decide in the tool, with a committee-ready report.
- **Model owner**: plug in their model without rewriting it, see its control results on every run.
- **Risk and valuation control**: check production system prices against an independent reference.
- **Internal audit and regulator**: reconstruct who did what, with which data and which code, without depending on anyone.

## Roadmap

Durations are orders of magnitude, to be recalibrated after the first pilot.

### 1. Pilot-ready (0 to 4 months)
- ~~**Model inventory**~~ (done in v0.9): one record per model, linked to its pipelines and decisions, status workflow.
- ~~**Scheduling** of runs and alerts~~ (done in v0.9).
- Versioned, content-addressed **market data snapshots**.
- **Independent price verification (IPV)**: production prices against reference or market prices, thresholds, valuation reserves.
- Signed and timestamped **PDF validation report**.

### Portfolio risk (done in v0.10)
- Historical VaR and ES by full revaluation, sensitivities per factor and per pillar, stress tests with limits, FRTB P&L attribution test.
- Next extensions: Monte Carlo VaR, ES backtesting, FRTB standardised approach, FX risk, gamma and cross terms in the risk-theoretical P&L.

### 2. Validation depth (4 to 12 months)
- **Pricing**: swaptions and caps/floors (Black, Bachelier, SABR), FX, CDS; every addition benchmarked against QuantLib (in [quantparity](https://github.com/secke/quantparity)).
- **Credit**: LGD, EAD, IFRS 9 (expected losses, staging), rating backtesting.
- **Tests**: model stress and sensitivity, comparison with challenger models, continuous drift monitoring in production.
- **Import connectors** for Murex, Orchestrade and Calypso exports (trades, prices, sensitivities).

### 3. Scaling up (12 to 24 months)
- **Organization**: multi-entity and multi-jurisdiction, validation delegations.
- **Platform**: high availability and distributed computing for large portfolios.
- **Certifications**: SOC 2 / ISO 27001, reversibility plan, validation documentation of the engine itself.

## Success metrics

- **Time to implement**: until the first tooled validation at a client.
- **Coverage**: share of the inventory's models tracked in ModelFlow.
- **Efficiency**: average review time of a model, model risk audit findings closed.
- **Commercial**: number of active client references, retention.

## Risks

- **Sales cycles** of 12 to 24 months in banks, with demanding vendor due diligence. Response: a targeted pilot and security certifications as early as possible.
- **Trust in the engine**: it must itself be validated. Systematic benchmarking against independent references is the answer (the [quantparity](https://github.com/secke/quantparity) QuantLib parity suite).
- **Competition** from the governance modules of large vendors and from in-house tools. The difference lies in independence from the production system and native reproducibility.
