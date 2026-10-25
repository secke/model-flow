# ModelFlow

Orchestration déterministe de modèles quantitatifs : DAG versionné, empreintes SHA-256, rejeu vérifié, validation automatique. Rust, sans IA générative.

```bash
cargo test --workspace
cargo run -p mf-cli -- demo manifest.json     # exécute VaR 99 % + backtest Kupiec
cargo run -p mf-cli -- verify manifest.json   # rejoue et compare les empreintes
```
Voir `docs/CADRAGE.md`.
