// Test de bout en bout de l'interface : SSO (code + PKCE) avec un fournisseur simulé, puis
// propriétaire → lance, validateur → rejoue et statue, auditeur → consulte le journal.
// Usage : node ui-tests/e2e.cjs [chemin/vers/mf-server]   (captures dans $SHOTS, défaut ui-tests/screenshots)
"use strict";
const { chromium } = require("playwright");
const { spawn } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");
const net = require("net");
const idp = require("./mock-idp.cjs");

const ROOT = path.resolve(__dirname, "..");
const BIN = process.argv[2] || path.join(ROOT, "target/release/mf-server");
const SHOTS = process.env.SHOTS || path.join(__dirname, "screenshots");
const AUD = "modelflow-api";
const CLIENT = "modelflow-web";

function freePort() {
  return new Promise((resolve) => {
    const s = net.createServer().listen(0, "127.0.0.1", () => {
      const p = s.address().port;
      s.close(() => resolve(p));
    });
  });
}

function check(cond, msg) {
  if (!cond) throw new Error(`Échec : ${msg}`);
  console.log(`  ok  ${msg}`);
}

async function main() {
  fs.mkdirSync(SHOTS, { recursive: true });
  const provider = await idp.start({ audience: AUD, clientId: CLIENT });
  const ws = fs.mkdtempSync(path.join(os.tmpdir(), "mf-ui-"));
  fs.cpSync(path.join(ROOT, "examples/pd-scorecard"), path.join(ws, "pd-scorecard"), { recursive: true });
  fs.rmSync(path.join(ws, "pd-scorecard/.modelflow"), { recursive: true, force: true });
  const toml = path.join(ws, "pd-scorecard/pipeline.toml");
  fs.writeFileSync(toml, fs.readFileSync(toml, "utf8").replace(/^(name = .*)$/m, '$1\nmodel = "PD-RETAIL-01"\nschedule = "0 6 * * 1-5"'));
  const port = await freePort();
  const cfg = path.join(ws, "server.toml");
  fs.writeFileSync(cfg, `workspace = "${ws}"
bind = "127.0.0.1:${port}"
[scheduler]
enabled = false
[oidc]
issuer = "${provider.base}"
audience = "${AUD}"
client_id = "${CLIENT}"
scopes = "openid profile"
roles_claim = "groups"
[oidc.role_mapping]
"MF-Owners" = "model_owner"
"MF-Validators" = "validator"
"MF-Auditors" = "auditor"
`);
  const server = spawn(BIN, ["serve", "--config", cfg], { stdio: ["ignore", "inherit", "inherit"] });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 50; i++) {
    try { if ((await fetch(`${origin}/api/health`)).ok) break; } catch {}
    await new Promise((r) => setTimeout(r, 200));
  }

  const browser = await chromium.launch();
  const problems = [];
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await context.newPage();
  page.on("console", (m) => m.type() === "error" && problems.push(m.text()));
  page.on("pageerror", (e) => problems.push(e.message));

  const login = async (user) => {
    await page.goto(origin + "/");
    await page.click("#sso");
    await page.click(`#user-${user}`);
    await page.waitForSelector("nav.nav");
  };
  const logout = async () => {
    await page.click("text=Se déconnecter");
    await page.waitForSelector("#sso");
  };

  try {
    console.log("En-têtes de sécurité");
    const h = (await fetch(origin + "/")).headers;
    check((h.get("content-security-policy") || "").includes("script-src 'self'"), "CSP restreint les scripts à l'origine");
    check(h.get("x-frame-options") === "DENY", "l'interface ne peut pas être encadrée");

    console.log("Propriétaire du modèle");
    await login("dana");
    check((await page.textContent(".who")).includes("Propriétaire de modèle"), "rôle issu du groupe SSO");
    await page.screenshot({ path: path.join(SHOTS, "1-pipelines.png"), fullPage: true });

    await page.click("text=Inventaire des modèles");
    await page.waitForSelector("text=Pipelines rattachés à un modèle absent");
    check(true, "un pipeline rattaché à un modèle non inventorié est signalé");
    await page.click("#new-model");
    await page.fill("#m-id", "PD-RETAIL-01");
    await page.fill("#m-name", "Scorecard PD particuliers");
    await page.fill("#m-purpose", "Probabilité de défaut à un an, portefeuille de détail (IRB).");
    await page.selectOption("#m-tier", "1");
    await page.fill("#m-review", "2027-03-31");
    await page.click("#m-save");
    await page.waitForSelector("text=Scorecard PD particuliers");
    check((await page.textContent(".facts")).includes("En développement"), "modèle enregistré en développement");
    check((await page.$("#status-approved")) === null, "un propriétaire ne peut pas approuver son modèle");
    await page.click("#status-in_validation");
    await page.waitForSelector("text=En validation");
    check(true, "modèle soumis à la validation");

    await page.goto(origin + "/#/p/pd-retail-scorecard");
    await page.waitForSelector("text=Prochaine exécution");
    check((await page.textContent(".facts")).includes("0 6 * * 1-5"), "planification affichée sur le pipeline");
    await page.goto(origin + "/#/");
    await page.waitForSelector("text=pd-retail-scorecard");
    await page.click("text=pd-retail-scorecard");
    await page.waitForSelector("#launch");
    await page.click("#launch");
    await page.waitForURL(/#\/p\/.+\/r\/1$/, { timeout: 60000 });
    await page.waitForSelector(".seal");
    check((await page.textContent(".seal .verdict")) === "Non conforme", "le sceau affiche le verdict de l'exécution");
    check((await page.$("#approve")) === null, "un propriétaire ne peut pas statuer");
    const frame = page.frameLocator("iframe.report");
    await frame.locator("text=Non conforme — actions requises").waitFor();
    check(true, "le rapport de validation est affiché");
    await logout();

    console.log("Validateur");
    await login("eli");
    await page.goto(origin + "/#/p/pd-retail-scorecard/r/1");
    await page.waitForSelector("#verify");
    await page.click("#verify");
    await page.waitForSelector("text=Rejeu identique", { timeout: 60000 });
    check(true, "le rejeu confirme la reproductibilité");
    await page.click("#reject");
    await page.waitForSelector(".notice.fail");
    check((await page.textContent(".notice.fail")).includes("motivé"), "un rejet sans commentaire est refusé");
    await page.fill("#comment", "Classe 6 sous-estimée (taux observé 14,9 % pour une PD de 10 %) : recalibrage requis avant usage.");
    await page.click("#reject");
    await page.waitForSelector(".timeline li.rejected");
    check((await page.textContent(".timeline")).includes("eli"), "la décision est enregistrée et attribuée");
    await page.screenshot({ path: path.join(SHOTS, "2-run-validator.png"), fullPage: true });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: path.join(SHOTS, "3-run-mobile.png"), fullPage: true });
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    check(overflow <= 0, `pas de défilement horizontal sur mobile (${overflow}px)`);
    await page.setViewportSize({ width: 1280, height: 900 });

    await page.goto(origin + "/#/models/PD-RETAIL-01");
    await page.waitForSelector("#status-approved");
    check((await page.$("#edit-model")) === null, "un validateur ne modifie pas la fiche");
    await page.click("#status-approved");
    await page.waitForSelector(".notice.fail");
    check((await page.textContent(".notice.fail")).includes("dernière exécution rejetée"), "modèle non approuvable tant que sa dernière exécution est rejetée");
    await page.goto(origin + "/#/models");
    await page.waitForSelector("text=Scorecard PD particuliers");
    await page.screenshot({ path: path.join(SHOTS, "5-inventaire.png"), fullPage: true });
    const stray = await page.evaluate(() => [...document.querySelector("main").childNodes].some((n) => n.nodeType === 3 && n.textContent.trim() !== ""));
    check(!stray, "aucun texte parasite dans la page");
    await logout();

    console.log("Auditeur");
    await login("carol");
    check((await page.$("text=Journal d'audit")) !== null, "le journal d'audit est accessible");
    await page.goto(origin + "/#/p/pd-retail-scorecard");
    await page.waitForSelector("h1");
    check((await page.$("#launch")) === null, "un auditeur ne peut pas lancer d'exécution");
    await page.goto(origin + "/#/audit");
    await page.waitForSelector("#filter");
    await page.fill("#filter", "run.decide");
    const text = await page.textContent("tbody");
    check(text.includes("eli") && text.includes("Rejetée"), "le journal trace la décision du validateur");
    await page.fill("#filter", "model.status");
    check((await page.textContent("tbody")).includes("refused: in_validation -> approved"), "le journal trace le refus d'approbation du modèle");
    await page.fill("#filter", "valeur-introuvable");
    check((await page.textContent("tbody")).includes("Aucune entrée"), "le filtre du journal signale l'absence de résultat");
    await page.fill("#filter", "");
    await page.screenshot({ path: path.join(SHOTS, "4-audit.png"), fullPage: true });
    await logout();

    console.log("Séparation des tâches au niveau SSO");
    await page.goto(origin + "/");
    await page.click("#sso");
    await page.click("#user-fred");
    await page.waitForSelector(".notice.fail");
    check((await page.textContent(".notice.fail")).includes("séparation des tâches"), "rôles incompatibles refusés à la connexion");

    const csp = problems.filter((p) => /Content Security Policy|Refused/i.test(p));
    check(csp.length === 0, `aucune violation de la politique de sécurité (${csp.join(" | ")})`);
    console.log("\nTous les contrôles de l'interface sont passés.");
  } finally {
    await browser.close();
    server.kill();
    provider.close();
    fs.rmSync(ws, { recursive: true, force: true });
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
