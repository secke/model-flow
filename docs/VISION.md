# ModelFlow : vision produit

## Positionnement

ModelFlow est la plateforme de **gouvernance, de validation et de contrôle indépendant des modèles et des valorisations** pour les banques et les gestionnaires d'actifs. Elle se branche sur les systèmes existants (Murex, Orchestrade, Calypso, bibliothèques internes) sans les remplacer.

Le produit ne vise pas le front-to-back. L'ambition est de devenir **la référence de ce créneau**, comme Orchestrade ou Murex l'ont fait sur le leur.

## Ce qui a fait le succès de ces plateformes, et notre équivalent

| Facteur de succès | Chez Orchestrade / Murex | Chez ModelFlow |
|---|---|---|
| Un modèle de données unique | Toutes les classes d'actifs dans un même modèle, au lieu d'outils juxtaposés ([The Hedge Fund Journal](https://thehedgefundjournal.com/orchestrade-cross-asset-portfolio-risk-trading/)) | Un registre scellé unique : données, code, versions et résultats de chaque modèle, quel que soit son type (marché, crédit, pricing) |
| Mise en œuvre rapide | Mises en œuvre annoncées plus rapides que les systèmes historiques, nombreuses interfaces standard | Un binaire ou une image Docker, des pipelines décrits en TOML, des modèles branchés tels quels dans leur langage |
| Ouverture | API ouvertes, intégration Excel | API REST, modèles externes dans tout langage, formats CSV, Parquet et JSON |
| Confiance réglementaire | Reporting réglementaire intégré | Reproductibilité par construction, séparation des tâches, journal en ajout seul, conformes à l'esprit de SR 26-2, SS1/23 et BCBS 239 |
| Références clients | Entrée par un segment (les fonds pour Orchestrade), puis extension | Entrée par les équipes de validation et de risque de modèle, puis extension aux équipes de risque, de valorisation et d'audit |

## Proposition de valeur par utilisateur

- **Validateur** : rejouer et comparer n'importe quelle exécution, appliquer une batterie de tests standard, statuer dans l'outil, avec un rapport prêt pour le comité.
- **Propriétaire de modèle** : brancher son modèle sans le réécrire, voir ses résultats de contrôle à chaque exécution.
- **Risque et contrôle de valorisation** : vérifier les prix du système de production contre une référence indépendante.
- **Audit interne et régulateur** : reconstituer qui a fait quoi, avec quelles données et quel code, sans dépendre de personne.

## Feuille de route

Les durées sont des ordres de grandeur, à recaler après le premier pilote.

### 1. Prêt pour un pilote (0 à 4 mois)
- **Inventaire des modèles** : fiche par modèle (propriétaire, usage, niveau de risque, date de prochaine revue), liée à ses pipelines et à ses décisions.
- **Planification** des exécutions (quotidienne, mensuelle), avec alerte en cas d'échec ou de dégradation d'un test.
- **Instantanés de données de marché** versionnés et adressés par contenu.
- **Vérification indépendante des prix (IPV)** : prix de production contre prix de référence ou de marché, seuils, réserves de valorisation.
- **Rapport de validation PDF**, signé et horodaté.

### 2. Profondeur de validation (4 à 12 mois)
- **Pricing** : swaptions et caps/floors (Black, Bachelier, SABR), change, CDS ; chaque ajout étalonné contre QuantLib.
- **Crédit** : LGD, EAD, IFRS 9 (pertes attendues, staging), backtesting des notations.
- **Tests** : stress et sensibilité des modèles, comparaison avec des modèles challengers, suivi continu de la dérive en production.
- **Connecteurs d'import** des exports Murex, Orchestrade et Calypso (transactions, prix, sensibilités).

### 3. Passage à l'échelle (12 à 24 mois)
- **Organisation** : multi-entités et multi-juridictions, délégations de validation.
- **Plateforme** : haute disponibilité et calcul distribué pour les gros portefeuilles.
- **Certifications** : SOC 2 / ISO 27001, plan de réversibilité, documentation de validation du moteur lui-même.

## Indicateurs de succès

- **Temps de mise en œuvre** : jusqu'à la première validation outillée chez un client.
- **Couverture** : part des modèles de l'inventaire suivis dans ModelFlow.
- **Efficacité** : délai moyen de revue d'un modèle, constats d'audit sur le risque de modèle levés.
- **Commercial** : nombre de références clients actives, rétention.

## Risques

- **Cycles de vente** de 12 à 24 mois en banque, avec des due diligences fournisseur exigeantes. Pour y répondre : un pilote ciblé et des certifications de sécurité dès que possible.
- **Confiance dans le moteur** : il doit être lui-même validé. L'étalonnage systématique contre des références indépendantes est la réponse.
- **Concurrence** des modules de gouvernance des grands éditeurs et des outils internes. La différence tient à l'indépendance vis-à-vis du système de production et à la reproductibilité native.
