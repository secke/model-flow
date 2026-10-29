# Contributing to ModelFlow

Thanks for your interest. ModelFlow is used to produce validation evidence, so changes are held to two rules: results stay reproducible, and governance rules stay enforced by the server, not by convention.

## Ground rules

- **No silent change to a number.** Pricing, risk and statistical algorithms live in [quantparity](https://github.com/secke/quantparity), where every function is reconciled with QuantLib. Changes to an algorithm go there first, with a parity test; ModelFlow then bumps the pinned revision in `Cargo.toml`.
- **Determinism.** A step must give the same output for the same inputs and code. No wall-clock time, randomness without a fixed seed, or iteration over unordered maps in outputs.
- **Governance is code.** Segregation of duties, the append-only audit log and replay-before-decision are enforced in `mf-server` and covered by tests. A change that weakens one of them needs an explicit discussion in an issue first.
- **Built-in step changes.** The code fingerprint of built-in steps is derived from the `mf-runtime` version: bump the workspace version when a built-in step's behavior changes, so replays of older runs report it.
- **English** for code, comments, messages, UI text and documentation.

## Development setup

Requirements: Rust 1.88+, Python 3 with `QuantLib==1.44` for the benchmark examples, Docker for PostgreSQL, Node.js 20 for the UI tests.

```bash
# PostgreSQL for the database tests
docker run -d --name mf-pg -e POSTGRES_PASSWORD=ci -p 5432:5432 postgres:16
export MODELFLOW_TEST_DATABASE_URL=postgres://postgres:ci@127.0.0.1:5432/postgres

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # PostgreSQL tests are skipped if the variable is not set

# Examples end to end
cargo build --release
for e in var-equity pd-scorecard pricing-benchmark rates-benchmark multicurve-benchmark portfolio-risk; do
  target/release/mf run examples/$e/pipeline.toml && target/release/mf verify examples/$e/pipeline.toml
done

# UI end to end (Playwright, simulated OIDC provider)
npm install --no-save --prefix ui-tests playwright@1.63.0
npx --prefix ui-tests playwright install chromium
NODE_PATH=ui-tests/node_modules node ui-tests/e2e.cjs target/release/mf-server
```

CI runs all of the above, plus the Docker image build, an end-to-end run inside the read-only container and Kubernetes manifest validation. A pull request must be green before review.

## Pull requests

- Keep each pull request focused on one change, with tests.
- Describe what changed and why, and how you verified it.
- Update the README or `docs/` when behavior, configuration or output changes. `docs/GETTING-STARTED.md` must stay true on a fresh clone: if you change an output it quotes, rerun it.
- By submitting a contribution, you agree that it is licensed under the [Apache License 2.0](LICENSE).

## Reporting issues

Open a GitHub issue with the pipeline (or a minimal one), the command, the expected and actual output, and `mf-server`/`mf` version. For security issues (authentication bypass, audit log tampering, sandbox escape), do not open a public issue: contact the maintainer privately through GitHub.
