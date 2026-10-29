# Getting started locally

Duration: about 30 minutes. You play the three roles of a bank around the VaR model of a trading desk (`VAR-HS-01`) and its portfolio (`examples/portfolio-risk`):
- **alice**, model owner;
- **bob**, validator;
- **carol**, auditor.

## 0. Prerequisites

- Linux or macOS (on Windows: WSL2).
- `git` and Rust (`curl https://sh.rustup.rs -sSf | sh`, version 1.88 or newer).
- Python 3, only for the alert receiver in part B. The risk example does not need it.

## 1. Clone and build

```bash
git clone https://github.com/secke/model-flow.git
cd model-flow
cargo build --release            # 3 to 6 minutes the first time
export PATH="$PWD/target/release:$PATH"
mf                               # prints the help
```

## Part A: command line

### A1. Compute the portfolio risk
```bash
mf run examples/portfolio-risk/pipeline.toml
```
Expected result (the other steps are data):
```
var_es               v1.0.0    [WARN]
stress               v1.0.0    [FAIL]
pnl_attribution      v1.0.0    [PASS]
```
- **VaR**: the 99% VaR (€483k) reaches 88% of its €550k limit, hence the warning (`WARN`).
- **Stress**: the "Sovereign crisis" scenario loses €1.92m against a €1.5m limit, hence the failure (`FAIL`).
- **P&L attribution**: the P&L computed from sensitivities explains the full-revaluation P&L (green zone).

### A2. Read the report
```bash
mf report examples/portfolio-risk/pipeline.toml -o risk-report.html
xdg-open risk-report.html        # macOS: open risk-report.html
```
Things to look at:
- in `var_es`, the contribution of each position on the day that sets the VaR: the 10-year payer swap weighs the most;
- in `sensitivities`, the DV01 per curve pillar;
- in `stress`, the usage of each limit.

### A3. Check reproducibility
```bash
mf verify examples/portfolio-risk/pipeline.toml
# OK: run #1 replayed identically (8 steps)
```

### A4. Tamper with the data
Change a single value of the history (line 201, first SX5E shock), then replay:
```bash
sed -i.bak '201s/^200,[^,]*/200,-0.25/' examples/portfolio-risk/data/factor_history.csv   # macOS: same command
mf verify examples/portfolio-risk/pipeline.toml
```
The replay reports `MISMATCH [history] non-reproducible result`, then the downstream steps (`different inputs`). Restore the file:
```bash
mv examples/portfolio-risk/data/factor_history.csv.bak examples/portfolio-risk/data/factor_history.csv
mf verify examples/portfolio-risk/pipeline.toml   # OK again
```

### A5. Experiment
- Add your own scenario to `examples/portfolio-risk/stress.json`, for example `"SPOT:SX5E": -0.5`, then run `mf run` again.
- Change a quantity in `trades.json` and watch the VaR, the sensitivities and the stress tests.
- Run `mf runs examples/portfolio-risk/pipeline.toml` to see the run history and fingerprints.

## Part B: server, web UI and governance

### B1. Create three users and the configuration
From the repository root:
```bash
mkdir -p local
for u in alice bob carol; do mf-server new-token > local/$u.txt; done
token() { sed -n 's/^token (.*): //p' local/$1.txt; }
fingerprint() { sed -n 's/^token_sha256[^:]*: //p' local/$1.txt; }

cat > local/server.toml <<EOF
workspace = "../examples"
bind = "127.0.0.1:8080"
public_url = "http://127.0.0.1:8080"

[scheduler]
interval_secs = 10

[alerts]
webhook_url = "http://127.0.0.1:9000/"
on = ["fail", "warn", "error"]

[[user]]
name = "alice"
role = "model_owner"
token_sha256 = "$(fingerprint alice)"

[[user]]
name = "bob"
role = "validator"
token_sha256 = "$(fingerprint bob)"

[[user]]
name = "carol"
role = "auditor"
token_sha256 = "$(fingerprint carol)"
EOF
```
Only the token fingerprints are in the configuration. The tokens themselves stay in `local/*.txt`.

### B2. Start
Two terminals, at the repository root:
```bash
# Terminal 1: prints the alerts sent by ModelFlow
python3 tools/webhook-echo.py

# Terminal 2: the server (PATH as in step 1)
mf-server serve --config local/server.toml
```
Open <http://127.0.0.1:8080>. To print a token: `token alice` (in the shell where the function is defined) or `cat local/alice.txt`.

### B3. Alice, model owner
Sign in with alice's token, then:
1. **Model inventory** → **Register a model**:
   - identifier `VAR-HS-01` (the one declared by `examples/portfolio-risk/pipeline.toml`);
   - name "Desk historical VaR";
   - tier 1;
   - next review in one year.
2. On the model record: **Submit for validation**.
3. **Pipelines** → `portfolio-risk-desk` → **Trigger a run**. It is run #2, because part A created run #1. The seal shows "Fail", because of the failed stress test, and terminal 1 prints the alert.
4. **Sign out**.

### B4. Bob, validator
1. Open the run: **Replay the run**. The replay must be identical.
2. **Reject** with a comment, for example: "Sovereign crisis limit breached (128%): reduce the OAT exposure."
3. **Model inventory** → `VAR-HS-01` → **Approve the model**: refused ("latest run rejected").
4. **Sign out**.

### B5. Alice fixes, Bob approves
1. In `examples/portfolio-risk/trades.json`, change the quantity of `BD-OAT-10Y` from `100000` to `50000`.
2. As alice, trigger a new run. The stress test is back under its limit. The seal shows "Warning": the VaR rises to 92% of its limit, because the OAT was hedging part of the swaps' rate risk.
3. As bob:
   - **Replay the run**, then **Approve** with a comment on the VaR warning;
   - in the inventory, **Approve the model**, accepted this time.
4. As alice: **Put into use**.

Run #2 can no longer be replayed identically, since `trades.json` has changed: the replay says so (`positions: non-reproducible result`). This is the expected behavior. A run is only reproducible with the data that produced it. To keep both versions, version the pipeline directory with Git.

### B6. Carol, auditor
**Audit log**: every action is there, denials included. Filter by `model.status` or `run.decide`. Carol sees no action buttons.

### B7. Scheduling
Add below the `model = …` line of `examples/portfolio-risk/pipeline.toml`:
```toml
schedule = "*/2 * * * *"
```
The server rereads the pipelines on every pass: a run triggered by `scheduler` appears within four minutes (on the next even minute after the pipeline is first seen), and its alert shows up in terminal 1. Then remove the line.

### B8. The live API
```bash
curl -s -H "Authorization: Bearer $(token carol)" http://127.0.0.1:8080/api/models | python3 -m json.tool
curl -s -H "Authorization: Bearer $(token carol)" "http://127.0.0.1:8080/api/audit?limit=5" | python3 -m json.tool
```

## Cleanup
```bash
# Ctrl+C in both terminals, then:
rm -rf local risk-report.html examples/*/.modelflow examples/.modelflow-server
git checkout examples
```

## Going further
- **QuantLib examples**: `pip install QuantLib==1.44`, then `mf run examples/multicurve-benchmark/pipeline.toml`.
- **PostgreSQL**: `export MODELFLOW_DATABASE_URL=postgres://…` for the CLI, or a `[database]` section for the server.
- **Docker**: `docker compose up --build`, after filling in `deploy/server.docker.toml`.
- **SSO**: the `[oidc]` section of the [README](../README.md#single-sign-on-oidc).
