# ModelFlow — cadrage (option A : pricing & modèles orchestrés)

## Problème
Dans les banques d'investissement, les modèles (pricing, VaR, PD/LGD) et leurs données sont dispersés entre équipes et outils. Les versions ne sont pas tracées, les résultats ne sont pas reproductibles, et la validation (SR 26-2, SS1/23, BCBS 239) est manuelle.

## Proposition
Une couche d'orchestration **déterministe**, sans IA générative, qui se branche sur l'existant (Murex, Orchestrade, bibliothèques internes) :
- pipeline en DAG : données de marché → modèle → résultats → validation ;
- chaque étape est versionnée, ses entrées et sorties sont hachées (SHA-256) ;
- le manifeste d'exécution permet un **rejeu à l'identique** et prouve l'absence de dérive ;
- batterie de tests de validation exécutée automatiquement, avec rapport horodaté.

> Vision à long terme et trajectoire front-to-back : [VISION.md](VISION.md).

## État (v0.8)
- `mf-core` : DAG, empreintes des entrées, des sorties et du code, manifeste, vérification du rejeu.
- `mf-validate` : VaR historique ; Kupiec, Christoffersen, feux tricolores de Bâle ; PSI ; AUC, Gini, KS ; calibration binomiale par classe et Hosmer-Lemeshow.
- `mf-runtime` : pipelines TOML, étapes CSV (série ou table), étapes intégrées, processus externes avec délai maximal, stockage SQLite, rapport de validation HTML.
- `mf-pricing` : pricers de référence (BSM et sensibilités, Black-76, CRR américain, obligations, volatilité implicite), courbes de taux (bootstrap dépôts + swaps), swaps de taux et zéro-coupons avec DV01, benchmark de pricers avec tolérances par produit.
- `mf-server` : API REST, jetons hachés, rôles (propriétaire du modèle, validateur, auditeur, admin), décisions de validation avec séparation des tâches, journal d'audit en ajout seul.
- `mf-cli` : `mf run | verify | runs | show | report`.
- Taux multi-courbes : calendrier TARGET, conventions de marché, courbe €STR et projection Euribor 6M actualisée OIS, swaps datés.
- Stockage des sorties adressé par contenu, avec contrôle d'intégrité ; isolation des processus externes ; CSV RFC 4180 et Parquet.
- API : file d'attente des exécutions, limitation de débit, TLS natif, authentification unique OIDC (groupes vers rôles, séparation des tâches).
- Interface web des validateurs (SSO PKCE, lancement, rapport, rejeu, décisions, audit), testée de bout en bout.
- Stockage PostgreSQL pour l'historique et la gouvernance (journal d'audit protégé par trigger), SQLite conservé pour l'usage local.
- Déploiement : image Docker non root, docker-compose durci, manifestes Kubernetes (Pod Security restricted, NetworkPolicy).
- Cinq exemples : desk actions (VaR), scorecard PD, validation de QuantLib sur options et obligations, sur courbe simple et swaps, et en multi-courbes.
- CI GitHub : format, clippy, tests, exécution des exemples et rapports en artefacts.

Décision 1 tranchée : les modèles existants sont intégrés comme **processus externes** (JSON sur stdin/stdout).

## Limites connues
- Plusieurs répliques avec PostgreSQL : l'espace de travail (pipelines, données) doit être partagé entre pods.
- L'isolation réseau des modèles externes dépend du déploiement (NetworkPolicy) ; en exécution locale, ils ont accès au réseau.
- Taux : pas encore de swaptions, caps/floors, produits de change, jours de fixing historiques (fixings passés) ni d'autres calendriers que TARGET.
- Exécutions lancées à la demande uniquement : pas encore de planification intégrée (passer par la CI ou un CronJob avec un compte de service).

## Feuille de route (8 semaines)
| Sem. | Livrable |
|---|---|
| 1-2 | ~~Étapes externes, CSV/Parquet, SQLite et PostgreSQL, délai, isolation, stockage par contenu~~ (fait) |
| 3-4 | ~~Christoffersen, feux tricolores, PSI, Gini/KS, calibration PD~~ (fait) ; reste : modèle challenger, LGD/EAD, tests de sensibilité |
| 5 | ~~Pricers de référence, validation de QuantLib, courbes, swaps, multi-courbes OIS, conventions de marché~~ (fait) ; reste : fixings historiques, swaptions, caps/floors, change |
| 6 | ~~Rapport de validation HTML~~ (fait) ; reste : export PDF, inventaire des modèles, piste d'audit signée |
| 7 | ~~API REST, rôles, décisions, audit, file d'attente, limitation, TLS, SSO OIDC, interface web~~ (fait) ; reste : planification, inventaire des modèles |
| 8 | ~~Image Docker, docker-compose, manifestes Kubernetes, CI de l'image~~ (fait) ; reste : chart Helm, pilote |

## Décisions ouvertes
1. ~~Mode d'intégration~~ : processus externe retenu.
2. Stockage : SQLite (pilote) puis PostgreSQL.
3. Licence : noyau open source (Apache-2.0) + modules entreprise payants (modèle commercial).
