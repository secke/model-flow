# ModelFlow

Orchestration déterministe de modèles quantitatifs : DAG versionné, empreintes SHA-256 (entrées, sorties, code), rejeu vérifié, validation automatique. Rust, sans IA générative.

## Démarrage
```bash
cargo build --release
mf=target/release/mf; p=examples/var-equity/pipeline.toml
$mf run $p        # exécute et enregistre (SQLite : examples/var-equity/.modelflow/runs.db)
$mf verify $p     # rejoue la dernière exécution et signale tout écart
$mf runs $p       # historique
$mf show $p --run 1
```

## Types d'étapes
| kind | rôle |
|---|---|
| `csv` | lit une colonne numérique (`path`, `column`) |
| `var_historical` | VaR historique (`alpha`) |
| `kupiec_backtest` | backtest glissant de la VaR historique (`alpha`, `window`) |
| `backtest_forecasts` | backtest de prévisions de VaR fournies par un modèle (`returns`, `forecasts`) |
| `process` | **modèle externe, tout langage** (`command`, `code`) |

## Protocole des modèles externes
Le processus reçoit sur stdin `{"inputs": {<dep>: <sortie>}, "params": {...}}` et écrit un document JSON sur stdout ; un code de sortie non nul signale l'échec (stderr est remonté). Les fichiers listés dans `code` sont hachés : une modification du code sans changement de `version` est signalée au rejeu.

Exemple : `examples/var-equity/models/ewma_var.py` (VaR EWMA RiskMetrics) validé par `backtest_forecasts`.

## Écarts détectés par `verify`
- `code modifié sans changement de version`
- `version a -> b`
- `entrées différentes` (données ou étape amont modifiées)
- `résultat non reproductible`

Voir `docs/CADRAGE.md`.
