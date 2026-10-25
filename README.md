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
| `examples/pricing-benchmark` | Validation de QuantLib 1.44 sur 12 transactions (options européennes et américaines, Black-76, obligations) |
| `examples/rates-benchmark` | Courbe EUR 6M (dépôts + swaps), re-pricing, validation de QuantLib sur 7 swaps et 3 zéro-coupons |

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
| `json` | lit un document JSON (transactions, prix…) | `path` |
| `price_portfolio` | valorisation de référence et sensibilités | `trades` |
| `pricing_benchmark` | pricer candidat vs. référence, tolérances par produit | `trades`, `candidate`, `reference` (optionnel), `market` (optionnel), `tolerances` |
| `curve_bootstrap` | construction de courbe, re-pricing des instruments, structure par terme | `curve`, `tolerance`, `grid_step` |

`price_portfolio` et `pricing_benchmark` acceptent aussi `market` (nom de l'étape qui fournit les courbes), requis pour les swaps et les zéro-coupons.

Les backtests de VaR combinent **Kupiec** (POF), **Christoffersen** (indépendance, couverture conditionnelle) et les **feux tricolores de Bâle** (avec majoration du multiplicateur sur 250 jours à 99 %). Chaque test renvoie un statut `pass`, `warn` ou `fail`.

## Pricing de référence (`mf-pricing`)
Black-Scholes-Merton avec dividende continu et sensibilités, Black-76, arbre CRR (exercice européen ou américain), obligations à taux fixe (prix, durations, convexité) et volatilité implicite. Les formules sont testées sur des valeurs publiées (Hull ; Haug, *The Complete Guide to Option Pricing Formulas*), sur la parité call-put et sur des différences finies.

`pricing_benchmark` compare un pricer candidat (QuantLib, bibliothèque interne, modèle challenger) soit à la référence intégrée, soit à un autre pricer. L'écart admis vaut `abs + rel × |référence|` par produit. Un écart jusqu'à 10 fois cette valeur donne `warn`, au-delà `fail`. Une transaction sans prix candidat donne `fail`.

Résultat de l'exemple : QuantLib et la référence coïncident à 1e-13 près sur les options européennes, Black-76 et les obligations. Sur les options américaines, l'écart est d'environ 5e-5 : le CRR de QuantLib utilise une probabilité linéarisée en log, ½ + ½·(r − q − σ²/2)·√Δt / σ, alors que la référence utilise la probabilité martingale exacte. La tolérance des arbres est donc fixée en conséquence.

Format des transactions : voir `examples/pricing-benchmark/trades.json`.

### Taux (`mf_pricing::rates`)
- **Courbes** : bootstrap séquentiel sur dépôts (taux simple) et swaps par, interpolation log-linéaire des facteurs d'actualisation (forwards constants par morceaux), extrapolation au dernier forward.
- **Produits** : swap de taux vanille payeur ou receveur (NPV, taux par, annuité, jambes, DV01 par déplacement de 1 pb de toutes les cotations) et zéro-coupon (taux zéro, DV01).
- **Contrôle** : `curve_bootstrap` re-price chaque instrument d'entrée (écart observé de l'ordre de 1e-16) et publie facteurs d'actualisation, taux zéro et forwards.
- **Simplifications** : une seule courbe pour projeter et actualiser, fractions d'année égales à 1/fréquence, pas de calendrier ni de jours de fixing. Le multi-courbes (OIS en actualisation) et les conventions de marché sont à venir.

Résultat de l'exemple : avec les mêmes conventions (calendrier nul, SimpleDayCounter, PiecewiseLogLinearDiscount), QuantLib et la référence coïncident à environ 1e-8 en devise sur des notionnels de 8 à 50 millions, et à 2e-7 en extrapolation à 40 ans.

Format du marché : voir `examples/rates-benchmark/market.json`.

## API REST (`mf-server`)
```bash
target/release/mf-server new-token                 # jeton + empreinte SHA-256
cp deploy/server.example.toml server.toml          # renseigner les empreintes
target/release/mf-server serve --config server.toml
curl -H "Authorization: Bearer $JETON" localhost:8080/api/pipelines
```
| Méthode | Chemin | Rôles |
|---|---|---|
| GET | `/api/health` | public |
| GET | `/api/me`, `/api/pipelines`, `/api/pipelines/{p}/runs`, `…/runs/{id}`, `…/runs/{id}/report`, `…/runs/{id}/decisions` | tous |
| POST | `/api/pipelines/{p}/runs` | `model_owner`, `admin` |
| POST | `/api/pipelines/{p}/runs/{id}/verify` | `validator`, `model_owner`, `admin` |
| POST | `/api/pipelines/{p}/runs/{id}/decisions` `{"decision": "approved" \| "rejected", "comment": "…"}` | `validator` |
| GET | `/api/audit?limit=100` | `auditor`, `admin` |

Règles de gouvernance :
- La configuration ne contient que l'empreinte SHA-256 des jetons.
- Un validateur ne peut pas statuer sur une exécution qu'il a lancée (séparation des tâches).
- Une décision n'est acceptée que si le rejeu est identique ; un rejet doit être motivé.
- Chaque action, y compris chaque refus, est inscrite dans un journal d'audit en ajout seul : la base refuse modifications et suppressions.
- Le rapport HTML inclut les décisions de validation.

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
| `mf-pricing` | pricers de référence, benchmark de pricers |
| `mf-runtime` | spécification TOML, étapes, stockage SQLite, rapport HTML |
| `mf-server` | API REST, authentification, rôles, décisions, audit |
| `mf-cli` | binaire `mf` |

Voir `docs/CADRAGE.md` pour la feuille de route.
