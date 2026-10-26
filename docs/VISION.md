# ModelFlow : vision produit

## Ambition

Devenir une plateforme front-to-back des marchés de capitaux du niveau d'Orchestrade ou de Murex, avec une différence de conception : **chaque chiffre produit est rejouable et auditable par construction.** Une valorisation, une sensibilité, un P&L ou une VaR portent l'empreinte des données de marché, des transactions, du code du modèle et de la version du moteur qui les ont produits.

Ce qui existe aujourd'hui (v0.7) est la couche de gouvernance et de contrôle de valorisation. C'est le socle de cette ambition, pas encore la plateforme.

## Le marché, sans complaisance

- **Orchestrade**, fondé en 2009, se présente comme une plateforme « nativement multi-actifs » bâtie sur un modèle de données unique. Elle couvre :
  - la saisie des transactions et un registre de leur cycle de vie (exercices, résiliations, novations) ;
  - le risque et le P&L en temps réel, avec un moteur de VaR ;
  - la marge et le collatéral, le reporting réglementaire (MiFID/MiFIR, EMIR, Dodd-Frank) ;
  - plus de 140 interfaces de marché et l'énergie et les matières premières.

  Elle met aussi en avant un code plus compact et des mises en œuvre plus rapides que les systèmes historiques. ([The Hedge Fund Journal](https://thehedgefundjournal.com/orchestrade-cross-asset-portfolio-risk-trading/))
- **Murex, Nasdaq Calypso et ION** dominent les banques. Les remplacer prend des années. La dépendance à un fournisseur dominant pèse assez lourd pour que des banques européennes aient étudié en 2019 un développement en consortium pour s'affranchir d'ION. ([Risk.net](https://risk.net/node/7174836))
- **Conséquence** : une nouvelle plateforme ne gagne pas un appel d'offres front-to-back dès son lancement. Elle entre par un besoin précis, prouve sa fiabilité en production, puis étend son périmètre. C'est la trajectoire suivie par Orchestrade lui-même, d'abord auprès des fonds.

## Ce qui nous différencie

1. **Reproductibilité native.** Les régulateurs exigent de pouvoir reconstituer un résultat : SR 26-2 et SS1/23 pour le risque de modèle, BCBS 239 pour les données, les tests d'attribution du P&L du FRTB. Chez les éditeurs historiques, cette reconstitution est un projet. Chez nous, c'est une propriété du moteur : rejeu vérifié, sorties adressées par contenu, journal en ajout seul.
2. **Modèles ouverts.** Les quants branchent leurs modèles dans le langage de leur choix (Python, C++, R), sous contrôle : versions, empreinte du code, isolation, limites de ressources. La plateforme ne les enferme pas dans un langage propriétaire.
3. **Gouvernance intégrée.** Propriétaire, validateur et auditeur sont des rôles du produit, et la séparation des tâches est appliquée jusqu'au SSO. Ce n'est pas un module ajouté ensuite.
4. **Sobriété technique.** Le moteur est en Rust, avec un déploiement sur site durci (conteneur non root, aucune sortie réseau, PostgreSQL) et la même base de code du poste du quant au cluster.

## Architecture cible

```
                ┌──────────────────────── Interface web · API · Excel ────────────────────────┐
                │                                                                              │
  Référentiel ──┤  Transactions et cycle de vie ── Positions ── Valorisation et risque ── P&L  │
  (instruments, │  (saisie, événements, fixings,    (temps réel  (EOD et intraday,      (explain,│
  contreparties,│   exercices, novations)            et EOD)      sensibilités, VaR/ES,  attri-  │
  calendriers)  │                                                FRTB, XVA)             bution)│
                │                                                                              │
  Données de ───┤  Limites et alertes ── Collatéral ── Opérations (confirmations, règlements    │
  marché        │                                      ISO 20022) ── Comptabilité ── Réglementaire│
  (instantanés  │                                                                              │
  versionnés)   └──────────────── Gouvernance : rejeu, empreintes, décisions, audit ───────────┘
                                         (socle existant : mf-core, mf-runtime, mf-server)
```

Choix structurants :
- **Un seul modèle de données, transactions en événements.** Le registre des transactions est un journal d'événements (création, amendement, fixing, exercice, novation). La position à une date est une projection de ce journal, et l'audit est le journal lui-même. Modèle d'échange : ISDA CDM et FpML.
- **Données de marché en instantanés adressés par contenu.** Chaque courbe, surface ou fixing utilisé est figé et haché. Le mécanisme existe déjà pour les sorties d'étapes ; il sera étendu aux entrées.
- **Calcul distribué par partition.** Les portefeuilles sont découpés en lots déterministes valorisés en parallèle. Le résultat agrégé garde la traçabilité de chaque lot.
- **Bibliothèque de pricing propre, étalonnée.** Chaque modèle ajouté est validé contre une référence indépendante, comme aujourd'hui contre QuantLib : écarts mesurés et documentés.

## État actuel par rapport à la cible

| Domaine | Aujourd'hui (v0.7) | Cible |
|---|---|---|
| Gouvernance des modèles | Rejeu vérifié, empreintes, décisions, séparation des tâches, audit en ajout seul, interface web, SSO | Inventaire des modèles, niveaux de risque, revues périodiques, rapport PDF signé |
| Pricing | Options (BSM, Black-76, américaines), obligations, courbes simples et multi-courbes €STR/Euribor, swaps datés, DV01 | Swaptions et caps/floors (Black, Bachelier, SABR), change, inflation, crédit (CDS), autocalls par Monte-Carlo |
| Données de marché | Fichiers JSON/CSV/Parquet par pipeline | Service d'instantanés versionnés, surfaces de volatilité, contrôles de qualité, fixings historiques |
| Risque | VaR historique, backtests Kupiec, Christoffersen et feux tricolores | VaR/ES de portefeuille, sensibilités par buckets, FRTB SA, SIMM, stress tests, XVA, SA-CCR |
| Crédit | PD : Gini/KS, calibration, PSI | LGD/EAD, IFRS 9 (ECL, staging), notations internes |
| Transactions | Fichier de transactions en entrée | Saisie, cycle de vie en événements, positions temps réel, import FpML/CDM |
| P&L | — | P&L quotidien, explain par facteur, test d'attribution FRTB |
| Opérations | — | Confirmations, règlements ISO 20022, collatéral, comptabilité |
| Plateforme | CLI, API, file d'attente, PostgreSQL, Docker/Kubernetes | Planification, calcul distribué, haute disponibilité, multi-entités |

## Trajectoire

Les durées et les effectifs sont des ordres de grandeur, à recaler après le premier pilote.

### Phase A : contrôle de valorisation et risque de modèle (0 à 6 mois)
- **Proposition** : se placer à côté du système existant de la banque (Murex, Orchestrade, systèmes internes) pour valider ses modèles et vérifier ses prix de manière indépendante (IPV).
- **À construire** :
  - instantanés de données de marché versionnés ;
  - module IPV (prix du système de la banque comparés aux prix de marché ou de référence, avec seuils et réserves de valorisation) ;
  - swaptions et caps/floors ;
  - change ;
  - planification des exécutions ;
  - inventaire des modèles ;
  - rapport PDF signé.
- **Critère de sortie** : un pilote en banque, avec un cas d'usage réel de validation ou d'IPV.
- **Équipe** : 2 à 4 personnes, dont un profil quant.

### Phase B : moteur de risque (6 à 18 mois)
- **À construire** :
  - import des positions (FpML, CSV) ;
  - valorisation de portefeuille de fin de journée en calcul distribué ;
  - sensibilités par buckets (format CRIF) ;
  - VaR et ES historiques et par Monte-Carlo ;
  - FRTB SA ;
  - P&L explain et test d'attribution.
- **Critère de sortie** : le moteur tourne en parallèle du moteur de risque d'une banque et reproduit ses chiffres dans des tolérances convenues.
- **Équipe** : 6 à 10 personnes (quants, ingénieurs plateforme, données).

### Phase C : transactions et cycle de vie (18 à 36 mois)
- **À construire** :
  - saisie des transactions ;
  - registre en événements selon ISDA CDM (fixings, resets, exercices, novations) ;
  - positions et P&L en temps réel ;
  - limites ;
  - XVA et SA-CCR.
- **Critère de sortie** : un premier desk en production, sur un périmètre limité (par exemple les dérivés de taux vanille).

### Phase D : front-to-back (36 mois et plus)
- **À construire** :
  - opérations : confirmations, règlements ISO 20022, collatéral ;
  - comptabilité ;
  - reporting réglementaire (EMIR, MiFIR) ;
  - nouvelles classes d'actifs.
- **Critère de sortie** : capacité à répondre à un appel d'offres front-to-back.

## Risques principaux

- **Cycles de vente** : 12 à 24 mois en banque, avec des due diligences fournisseur exigeantes (sécurité, continuité, solidité financière). Il faudra viser SOC 2 / ISO 27001, et un plan de réversibilité.
- **Confiance dans les chiffres** : un moteur de valorisation doit être lui-même validé. L'étalonnage systématique contre des références et la reproductibilité sont des arguments, pas une dispense.
- **Ampleur** : le front-to-back représente des années d'effort d'équipes entières. Sans la phase A comme source de revenus et de références, les phases suivantes ne sont pas finançables.
- **Accès aux données** : flux de données de marché sous licence (Bloomberg, Refinitiv) et formats propres à chaque banque.

## Prochaines livraisons

1. Instantanés de données de marché versionnés et module IPV (seuils, réserves, rapport).
2. Swaptions et caps/floors (Black, Bachelier) validés contre QuantLib.
3. Planification des exécutions et inventaire des modèles dans l'interface.
