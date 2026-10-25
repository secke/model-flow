# ModelFlow

Orchestration déterministe et validation de modèles quantitatifs pour les banques : pipelines versionnés, empreintes SHA-256 des données, du code et des résultats, rejeu vérifié et rapport de validation. Écrit en Rust, sans IA générative.

## Démarrage
```bash
cargo build --release
mf=target/release/mf
p=examples/pd-scorecard/pipeline.toml
$mf run $p                 # exécute et enregistre (SQLite : <dossier du pipeline>/.modelflow/runs.db)
$mf verify $p              # rejoue la dernière exécution et signale tout écart
$mf report $p -o r.html    # rapport de validation HTML (inclut le rejeu)
$mf runs $p                # historique
$mf show $p --run 1        # manifeste JSON complet
```

## Exemples
| Dossier | Contenu |
|---|---|
| `examples/var-equity` | VaR historique et modèle EWMA externe (Python), backtests complets |
| `examples/pd-scorecard` | Modèle de PD à 7 classes : Gini/KS, calibration par classe, stabilité de la population |

## Types d'étapes
| kind | rôle | paramètres |
|---|---|---|
| `csv` | lit une colonne (`column`) ou une table (`columns`) numérique | `path` |
| `process` | **modèle externe, tout langage** | `command`, `code`, `timeout_secs` (300) |
| `var_historical` | VaR historique | `alpha` |
| `kupiec_backtest` | backtest glissant de la VaR historique | `alpha`, `window` |
| `backtest_forecasts` | backtest de prévisions de VaR d'un modèle | `returns`, `forecasts`, `alpha` |
| `psi` | Population Stability Index | `baseline`, `current`, `column`, `bins` |
| `pd_discrimination` | AUC, Gini, KS | `score`, `default`, `min_gini_pass`, `min_gini_warn` |
| `pd_calibration` | test binomial par classe, Hosmer-Lemeshow | `grade`, `pd`, `default` |

Les backtests de VaR combinent **Kupiec** (POF), **Christoffersen** (indépendance, couverture conditionnelle) et les **feux tricolores de Bâle** (avec majoration du multiplicateur sur 250 jours à 99 %). Chaque test renvoie un statut `pass`, `warn` ou `fail`.

## Protocole des modèles externes
Le processus reçoit sur stdin `{"inputs": {<dep>: <sortie>}, "params": {...}}` et écrit un document JSON sur stdout. Un code de sortie non nul signale un échec, et stderr est remonté. Les fichiers listés dans `code` sont hachés : une modification du code sans changement de `version` est signalée au rejeu.

## Écarts détectés par `verify`
- `code modifié sans changement de version`
- `version a -> b`
- `entrées différentes` (données ou étape amont modifiées)
- `résultat non reproductible`

## Architecture
| Crate | Rôle |
|---|---|
| `mf-core` | DAG d'étapes, empreintes, manifeste, rejeu |
| `mf-validate` | tests statistiques (sans dépendance externe) |
| `mf-runtime` | spécification TOML, étapes, stockage SQLite, rapport HTML |
| `mf-cli` | binaire `mf` |

Voir `docs/CADRAGE.md` pour la feuille de route.
