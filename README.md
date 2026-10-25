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
| `examples/rates-benchmark` | Courbe simple EUR 6M (dépôts + swaps), re-pricing, validation de QuantLib sur 7 swaps et 3 zéro-coupons |
| `examples/multicurve-benchmark` | Multi-courbes €STR / Euribor 6M aux conventions de marché, validation de QuantLib sur 8 swaps (2 à 30 ans) |

## Types d'étapes
| kind | rôle | paramètres |
|---|---|---|
| `csv` | lit une colonne (`column`) ou une table (`columns`) numérique ; guillemets RFC 4180 | `path`, `delimiter` (`,`) |
| `parquet` | idem pour un fichier Parquet (colonnes numériques) | `path` |
| `process` | **modèle externe, tout langage** | `command`, `code`, `timeout_secs` (300), `env`, `pass_env`, `memory_mb`, `cpu_secs`, `file_size_mb`, `max_output_mb` (256) |
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
- **Simplifications de ce mode** : une seule courbe pour projeter et actualiser, fractions d'année égales à 1/fréquence, pas de calendrier. Pour la production, utiliser les courbes datées ci-dessous.

Résultat de l'exemple : avec les mêmes conventions (calendrier nul, SimpleDayCounter, PiecewiseLogLinearDiscount), QuantLib et la référence coïncident à environ 1e-8 en devise sur des notionnels de 8 à 50 millions, et à 2e-7 en extrapolation à 40 ans.

Format du marché : voir `examples/rates-benchmark/market.json`.

### Multi-courbes et conventions de marché (`mf_pricing::curves`, `mf_pricing::dates`)
- **Conventions** : calendrier TARGET (Pâques calculée), décalage spot en jours ouvrés, Following / Modified Following / Preceding, bases Act/360, Act/365F et 30/360, règle de fin de mois des indices, échéanciers générés à rebours.
- **Courbe OIS** (`"kind": "ois"`) : swaps €STR au pair, jambe fixe annuelle (paiement unique jusqu'à 1 an), jambe variable capitalisée.
- **Courbe de projection** (`"kind": "ibor"`) : dépôt et swaps Euribor actualisés sur la courbe OIS désignée par `discount_curve`. Les courbes sont construites dans l'ordre de leurs dépendances.
- **Swap daté** (`"product": "vanilla_swap"`) : départ spot, conventions de la courbe de projection, actualisation OIS ; NPV, taux par, annuité, jambes et DV01 (toutes les cotations déplacées de 1 pb, courbes reconstruites).

Résultat de l'exemple : la courbe €STR et la courbe Euribor 6M re-pricent leurs instruments à 1e-16 près. Sur 8 swaps de 8 à 50 millions de notionnel et de 2 à 30 ans, QuantLib (OISRateHelper, SwapRateHelper actualisé €STR, MakeVanillaSwap) et la référence coïncident à moins de 4e-6 en devise.

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

Fonctionnement :
- `POST …/runs` place l'exécution en file d'attente et répond `202` avec un travail à suivre sur `GET /api/jobs/{id}` (`queued`, `running`, `succeeded`, `failed`). `?wait=true` attend la fin et renvoie le résultat.
- `max_concurrent_runs` (2) borne les exécutions simultanées. Les travaux interrompus par un arrêt du serveur sont marqués en échec au redémarrage.
- `rate_limit_per_minute` (120) limite les requêtes par utilisateur (réponse `429` au-delà).
- Section `[tls]` (`cert`, `key`, fichiers PEM) pour servir directement en HTTPS ; sinon, placer le serveur derrière un proxy HTTPS.

Règles de gouvernance :
- La configuration ne contient que l'empreinte SHA-256 des jetons.
- Un validateur ne peut pas statuer sur une exécution qu'il a lancée (séparation des tâches).
- Une décision n'est acceptée que si le rejeu est identique ; un rejet doit être motivé.
- Chaque action, y compris chaque refus, est inscrite dans un journal d'audit en ajout seul : la base refuse modifications et suppressions.
- Le rapport HTML inclut les décisions de validation.

## Protocole des modèles externes
Le processus reçoit sur stdin `{"inputs": {<dep>: <sortie>}, "params": {...}}` et écrit un document JSON sur stdout. Un code de sortie non nul signale un échec, et stderr est remonté. Les fichiers listés dans `code` sont hachés : une modification du code sans changement de `version` est signalée au rejeu.

Isolation :
- L'environnement est vidé. Seuls `PATH`, `LANG`, `HOME`, `TMPDIR` et `PYTHONHASHSEED=0` sont fournis, plus les variables `env` (prises en compte dans l'empreinte) et celles nommées dans `pass_env`.
- Sous Unix, des limites de ressources s'appliquent : mémoire (`memory_mb`), temps CPU (`cpu_secs`, par défaut le délai), taille des fichiers (`file_size_mb`), pas de fichier core.
- La sortie est plafonnée (`max_output_mb`).
- Le processus tourne dans son propre groupe, tué à la fin de l'étape : aucun processus orphelin ne survit.
- L'isolation réseau est assurée par le déploiement (NetworkPolicy sans sortie, voir plus bas).

## Stockage et intégrité
- Les sorties des étapes sont stockées une seule fois, adressées par leur empreinte SHA-256 : deux exécutions identiques ne dupliquent pas les données.
- Le manifeste ne garde que les empreintes. À la relecture, chaque sortie est re-hachée et toute altération de la base est détectée.
- Les étapes intégrées portent une empreinte de l'implémentation (version du moteur). Une montée de version du moteur est donc signalée au rejeu des anciennes exécutions.

## Déploiement
```bash
docker build -t modelflow:0.6.0 .
docker run --rm modelflow:0.6.0 new-token          # renseigner deploy/server.docker.toml
docker compose up --build                          # http://127.0.0.1:8080
kubectl apply -k deploy/k8s                        # après avoir renseigné deploy/k8s/secret.yaml
```
- **Image** : binaires `mf` et `mf-server`, Python 3.12 et QuantLib, utilisateur non root (10001). Les exemples sont livrés dans `/opt/modelflow/examples`.
- **docker-compose** : système de fichiers en lecture seule, aucune capacité, `no-new-privileges`, port lié à 127.0.0.1.
- **Kubernetes** (`deploy/k8s`) :
  - espace de noms en Pod Security `restricted`, conteneur non root en lecture seule, seccomp `RuntimeDefault` ;
  - sondes sur `/api/health`, limites de ressources ;
  - configuration dans un Secret, espace de travail sur un volume persistant ;
  - NetworkPolicy : entrée depuis l'ingress uniquement, aucune sortie ;
  - une seule réplique, car SQLite n'accepte qu'un écrivain ;
  - en production, alimenter `/workspace` depuis Git (git-sync ou CI) plutôt que par l'initContainer de démonstration.
- **CI** : construction de l'image, essai de bout en bout dans le conteneur (lecture seule, sans capacités) et validation des manifestes avec kubeconform.

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
