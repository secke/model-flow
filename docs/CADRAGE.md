# ModelFlow — cadrage (option A : pricing & modèles orchestrés)

## Problème
Dans les banques d'investissement, les modèles (pricing, VaR, PD/LGD) et leurs données sont dispersés entre équipes et outils. Les versions ne sont pas tracées, les résultats ne sont pas reproductibles, et la validation (SR 26-2, SS1/23, BCBS 239) est manuelle.

## Proposition
Une couche d'orchestration **déterministe**, sans IA générative, qui se branche sur l'existant (Murex, Orchestrade, bibliothèques internes) :
- pipeline en DAG : données de marché → modèle → résultats → validation ;
- chaque étape est versionnée, ses entrées et sorties sont hachées (SHA-256) ;
- le manifeste d'exécution permet un **rejeu à l'identique** et prouve l'absence de dérive ;
- batterie de tests de validation exécutée automatiquement, avec rapport horodaté.

## État (v0.4)
- `mf-core` : DAG, empreintes des entrées, des sorties et du code, manifeste, vérification du rejeu.
- `mf-validate` : VaR historique ; Kupiec, Christoffersen, feux tricolores de Bâle ; PSI ; AUC, Gini, KS ; calibration binomiale par classe et Hosmer-Lemeshow.
- `mf-runtime` : pipelines TOML, étapes CSV (série ou table), étapes intégrées, processus externes avec délai maximal, stockage SQLite, rapport de validation HTML.
- `mf-pricing` : pricers de référence (BSM et sensibilités, Black-76, CRR américain, obligations, volatilité implicite) et benchmark de pricers avec tolérances par produit.
- `mf-cli` : `mf run | verify | runs | show | report`.
- Trois exemples : desk actions (VaR), scorecard PD, validation de QuantLib.
- CI GitHub : format, clippy, tests, exécution des exemples et rapports en artefacts.

Décision 1 tranchée : les modèles existants sont intégrés comme **processus externes** (JSON sur stdin/stdout).

## Limites connues
- Les sorties complètes sont stockées dans le manifeste : à remplacer par un stockage adressé par contenu pour les gros volumes.
- Les processus externes ont un délai maximal mais ne sont pas isolés (pas de bac à sable).
- Le lecteur CSV est simple (pas de guillemets) ; Parquet n'est pas encore pris en charge.
- Les étapes intégrées sont versionnées manuellement : à lier à la version du moteur.

## Feuille de route (8 semaines)
| Sem. | Livrable |
|---|---|
| 1-2 | ~~Étapes externes, CSV, SQLite, délai maximal~~ (fait) ; reste : stockage par contenu, isolation, Parquet, PostgreSQL |
| 3-4 | ~~Christoffersen, feux tricolores, PSI, Gini/KS, calibration PD~~ (fait) ; reste : modèle challenger, LGD/EAD, tests de sensibilité |
| 5 | ~~Pricers de référence, validation de QuantLib, tolérances par produit~~ (fait) ; reste : courbes de taux, swaps, produits de taux et de change |
| 6 | ~~Rapport de validation HTML~~ (fait) ; reste : export PDF, inventaire des modèles, piste d'audit signée |
| 7 | Planification, API REST/gRPC, authentification, rôles (modèle owner / validateur) |
| 8 | Packaging Docker/Kubernetes on-premise, documentation, pilote |

## Décisions ouvertes
1. ~~Mode d'intégration~~ : processus externe retenu.
2. Stockage : SQLite (pilote) puis PostgreSQL.
3. Licence : noyau open source (Apache-2.0) + modules entreprise payants (modèle commercial).
