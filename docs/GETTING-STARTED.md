# Prise en main en local

Durée : environ 30 minutes. Vous jouez les trois rôles d'une banque autour du modèle de VaR d'un desk de marché (`VAR-HS-01`) et de son portefeuille (`examples/portfolio-risk`) :
- **alice**, propriétaire du modèle ;
- **bob**, validateur ;
- **carol**, auditrice.

## 0. Prérequis

- Linux ou macOS (sous Windows : WSL2).
- `git` et Rust (`curl https://sh.rustup.rs -sSf | sh`, version 1.88 ou plus récente).
- Python 3, uniquement pour le récepteur d'alertes de la partie B. L'exemple de risque n'en a pas besoin.

## 1. Récupérer et compiler

```bash
git clone https://github.com/secke/model-flow.git
cd model-flow
cargo build --release            # 3 à 6 minutes la première fois
export PATH="$PWD/target/release:$PATH"
mf                               # affiche l'aide
```

## Partie A : en ligne de commande

### A1. Calculer le risque du portefeuille
```bash
mf run examples/portfolio-risk/pipeline.toml
```
Résultat attendu (les autres étapes sont des données) :
```
var_es               v1.0.0    [WARN]
stress               v1.0.0    [FAIL]
pnl_attribution      v1.0.0    [PASS]
```
- **VaR** : la VaR 99 % (483 k€) atteint 88 % de sa limite de 550 k€, d'où la réserve (`WARN`).
- **Stress** : le scénario « Crise souveraine » perd 1,92 M€ pour une limite de 1,5 M€, d'où l'échec (`FAIL`).
- **Attribution du P&L** : le P&L calculé à partir des sensibilités explique le P&L de revalorisation complète (zone verte).

### A2. Lire le rapport
```bash
mf report examples/portfolio-risk/pipeline.toml -o rapport-risque.html
xdg-open rapport-risque.html     # macOS : open rapport-risque.html
```
À regarder :
- dans `var_es`, la contribution de chaque position au jour qui fixe la VaR : le swap payeur 10 ans pèse le plus ;
- dans `sensitivities`, les DV01 par pilier de courbe ;
- dans `stress`, l'utilisation de chaque limite.

### A3. Vérifier la reproductibilité
```bash
mf verify examples/portfolio-risk/pipeline.toml
# OK : exécution #1 rejouée à l'identique (8 étapes)
```

### A4. Provoquer un écart
Modifiez une seule valeur de l'historique (ligne 201, premier choc SX5E), puis rejouez :
```bash
sed -i.bak '201s/^200,[^,]*/200,-0.25/' examples/portfolio-risk/data/factor_history.csv   # macOS : identique
mf verify examples/portfolio-risk/pipeline.toml
```
Le rejeu signale `ÉCART [history] résultat non reproductible`, puis les étapes en aval (`entrées différentes`). Restaurez le fichier :
```bash
mv examples/portfolio-risk/data/factor_history.csv.bak examples/portfolio-risk/data/factor_history.csv
mf verify examples/portfolio-risk/pipeline.toml   # de nouveau OK
```

### A5. Expérimenter
- Ajoutez votre propre scénario dans `examples/portfolio-risk/stress.json`, par exemple `"SPOT:SX5E": -0.5`, puis relancez `mf run`.
- Changez une quantité dans `trades.json` et observez la VaR, les sensibilités et les stress.
- Relancez `mf runs examples/portfolio-risk/pipeline.toml` pour voir l'historique des exécutions et leurs empreintes.

## Partie B : serveur, interface et gouvernance

### B1. Créer trois utilisateurs et la configuration
Depuis la racine du dépôt :
```bash
mkdir -p local
for u in alice bob carol; do mf-server new-token > local/$u.txt; done
jeton() { sed -n 's/^jeton[^:]*: //p' local/$1.txt; }
empreinte() { sed -n 's/^token_sha256[^:]*: //p' local/$1.txt; }

cat > local/server.toml <<EOF
workspace = "../examples"
bind = "127.0.0.1:8080"
public_url = "http://127.0.0.1:8080"

[scheduler]
interval_secs = 10

[alerts]
webhook_url = "http://127.0.0.1:9000/"
on = ["fail", "warn", "error"]

[[user]]
name = "alice"
role = "model_owner"
token_sha256 = "$(empreinte alice)"

[[user]]
name = "bob"
role = "validator"
token_sha256 = "$(empreinte bob)"

[[user]]
name = "carol"
role = "auditor"
token_sha256 = "$(empreinte carol)"
EOF
```
Seules les empreintes des jetons figurent dans la configuration. Les jetons eux-mêmes restent dans `local/*.txt`.

### B2. Démarrer
Deux terminaux, à la racine du dépôt :
```bash
# Terminal 1 : affiche les alertes envoyées par ModelFlow
python3 tools/webhook-echo.py

# Terminal 2 : le serveur (PATH comme à l'étape 1)
mf-server serve --config local/server.toml
```
Ouvrez <http://127.0.0.1:8080>. Pour afficher un jeton : `jeton alice` (dans le shell où la fonction est définie) ou `cat local/alice.txt`.

### B3. Alice, propriétaire du modèle
Connectez-vous avec le jeton d'alice, puis :
1. **Inventaire des modèles** → **Enregistrer un modèle** :
   - identifiant `VAR-HS-01` (celui que déclare `examples/portfolio-risk/pipeline.toml`) ;
   - nom « VaR historique du desk » ;
   - niveau 1 ;
   - prochaine revue dans un an.
2. Sur la fiche du modèle : **Soumettre à la validation**.
3. **Pipelines** → `portfolio-risk-desk` → **Lancer une exécution**. Elle porte le numéro 2, car la partie A a créé l'exécution n° 1. Le sceau affiche « Non conforme », à cause du stress en échec, et le terminal 1 affiche l'alerte.
4. **Se déconnecter**.

### B4. Bob, validateur
1. Ouvrez l'exécution : **Rejouer l'exécution**. Le rejeu doit être identique.
2. **Rejeter** avec un commentaire, par exemple : « Dépassement de la limite Crise souveraine (128 %) : réduire l'exposition OAT. »
3. **Inventaire** → `VAR-HS-01` → **Approuver le modèle** : refusé (« dernière exécution rejetée »).
4. **Se déconnecter**.

### B5. Alice corrige, Bob approuve
1. Dans `examples/portfolio-risk/trades.json`, passez la quantité de `BD-OAT-10Y` de `100000` à `50000`.
2. En alice, relancez une exécution. Le stress repasse sous la limite. Le sceau affiche « Conforme avec réserves » : la VaR monte à 92 % de sa limite, car l'OAT couvrait une partie du risque de taux des swaps.
3. En bob :
   - **Rejouer**, puis **Approuver** avec un commentaire sur la réserve de VaR ;
   - dans l'inventaire, **Approuver le modèle**, accepté cette fois.
4. En alice : **Mettre en usage**.

L'exécution n° 2 ne peut plus être rejouée à l'identique, puisque `trades.json` a changé : le rejeu l'indique (`positions : résultat non reproductible`). C'est le comportement attendu. Une exécution n'est reproductible qu'avec les données qui l'ont produite. Pour conserver les deux versions, versionnez le dossier du pipeline avec Git.

### B6. Carol, auditrice
**Journal d'audit** : chaque action y figure, refus compris. Filtrez par `model.status` ou `run.decide`. Carol ne voit aucun bouton d'action.

### B7. Planification
Ajoutez sous la ligne `model = …` de `examples/portfolio-risk/pipeline.toml` :
```toml
schedule = "*/2 * * * *"
```
Le serveur relit les pipelines à chaque passage : une exécution lancée par `planificateur` apparaît dans les deux à quatre minutes, et son alerte arrive dans le terminal 1. Retirez ensuite la ligne.

### B8. L'API en direct
```bash
curl -s -H "Authorization: Bearer $(jeton carol)" http://127.0.0.1:8080/api/models | python3 -m json.tool
curl -s -H "Authorization: Bearer $(jeton carol)" "http://127.0.0.1:8080/api/audit?limit=5" | python3 -m json.tool
```

## Nettoyage
```bash
# Ctrl+C dans les deux terminaux, puis :
rm -rf local rapport-risque.html examples/*/.modelflow examples/.modelflow-server
git checkout examples
```

## Pour aller plus loin
- **Exemples QuantLib** : `pip install QuantLib==1.44`, puis `mf run examples/multicurve-benchmark/pipeline.toml`.
- **PostgreSQL** : `export MODELFLOW_DATABASE_URL=postgres://…` pour la CLI, ou une section `[database]` pour le serveur.
- **Docker** : `docker compose up --build`, après avoir renseigné `deploy/server.docker.toml`.
- **SSO** : la section `[oidc]` du README.
