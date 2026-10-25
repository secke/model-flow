# ModelFlow — cadrage (option A : pricing & modèles orchestrés)

## Problème
Dans les banques d'investissement, les modèles (pricing, VaR, PD/LGD) et leurs données sont dispersés entre équipes et outils. Les versions ne sont pas tracées, les résultats ne sont pas reproductibles, et la validation (SR 26-2, SS1/23, BCBS 239) est manuelle.

## Proposition
Une couche d'orchestration **déterministe**, sans IA générative, qui se branche sur l'existant (Murex, Orchestrade, bibliothèques internes) :
- pipeline en DAG : données de marché → modèle → résultats → validation ;
- chaque étape est versionnée, ses entrées et sorties sont hachées (SHA-256) ;
- le manifeste d'exécution permet un **rejeu à l'identique** et prouve l'absence de dérive ;
- batterie de tests de validation exécutée automatiquement, avec rapport horodaté.

## État du MVP (v0.2)
- `mf-core` : DAG, hachage des entrées/sorties/code, manifeste, vérification de rejeu.
- `mf-validate` : VaR historique, backtest glissant, test de Kupiec.
- `mf-runtime` : pipelines en TOML, étapes CSV / intégrées / **processus externes (tout langage)**, stockage SQLite des exécutions.
- `mf-cli` : `mf run | verify | runs | show`.
- Exemple : desk actions, VaR historique vs. modèle EWMA en Python, tous deux backtestés.
- 13 tests unitaires + scénarios bout en bout (rejeu identique, code modifié sans version, donnée altérée).

Décision 1 tranchée : intégration par **processus externe** (JSON stdin/stdout).

## Limites connues
- Les sorties complètes sont stockées dans le manifeste (à remplacer par un stockage adressé par contenu pour les gros volumes).
- Pas encore de timeout ni d'isolation des processus externes.
- Entrées CSV simples uniquement (pas de guillemets, pas de Parquet).

## Feuille de route (8 semaines)
| Sem. | Livrable |
|---|---|
| 1-2 | ~~Étapes externes, CSV, stockage SQLite~~ (fait) ; reste : stockage par contenu, timeout/isolation, Parquet, PostgreSQL |
| 3-4 | Tests de validation : Christoffersen, traffic-light Bâle, stabilité (PSI), benchmark modèle vs. modèle challenger, PD/LGD (Gini, KS, calibration) |
| 5 | Pricing : adaptateur QuantLib, jeux de référence, tolérances par produit |
| 6 | Rapport de validation (HTML/PDF), inventaire des modèles, piste d'audit signée |
| 7 | Planification, API REST/gRPC, authentification, rôles (modèle owner / validateur) |
| 8 | Packaging Docker/Kubernetes on-premise, documentation, pilote |

## Décisions ouvertes
1. ~~Mode d'intégration~~ : processus externe retenu.
2. Stockage : SQLite (pilote) puis PostgreSQL.
3. Licence : noyau open source (Apache-2.0) + modules entreprise payants (modèle commercial).
