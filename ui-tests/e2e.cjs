// End-to-end UI test: SSO (code + PKCE) with a simulated provider, then
// owner → triggers, validator → replays and decides, auditor → reviews the log.
// Usage: node ui-tests/e2e.cjs [path/to/mf-server]   (screenshots in $SHOTS, default ui-tests/screenshots)
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
  if (!cond) throw new Error(`Failed: ${msg}`);
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
    await page.click("text=Sign out");
    await page.waitForSelector("#sso");
  };

  try {
    console.log("Security headers");
    const h = (await fetch(origin + "/")).headers;
    check((h.get("content-security-policy") || "").includes("script-src 'self'"), "CSP restricts scripts to the origin");
    check(h.get("x-frame-options") === "DENY", "the UI cannot be framed");

    console.log("Model owner");
    await login("dana");
    check((await page.textContent(".who")).includes("Model owner"), "role derived from the SSO group");
    await page.screenshot({ path: path.join(SHOTS, "1-pipelines.png"), fullPage: true });

    await page.click("text=Model inventory");
    await page.waitForSelector("text=Pipelines linked to a model missing");
    check(true, "a pipeline linked to an uninventoried model is flagged");
    await page.click("#new-model");
    await page.fill("#m-id", "PD-RETAIL-01");
    await page.fill("#m-name", "Retail PD scorecard");
    await page.fill("#m-purpose", "One-year probability of default, retail portfolio (IRB).");
    await page.selectOption("#m-tier", "1");
    await page.fill("#m-review", "2027-03-31");
    await page.click("#m-save");
    await page.waitForSelector("text=Retail PD scorecard");
    check((await page.textContent(".facts")).includes("In development"), "model registered in development");
    check((await page.$("#status-approved")) === null, "an owner cannot approve their own model");
    await page.click("#status-in_validation");
    await page.waitForSelector("text=In validation");
    check(true, "model submitted for validation");

    await page.goto(origin + "/#/p/pd-retail-scorecard");
    await page.waitForSelector("text=Next run");
    check((await page.textContent(".facts")).includes("0 6 * * 1-5"), "schedule shown on the pipeline");
    await page.goto(origin + "/#/");
    await page.waitForSelector("text=pd-retail-scorecard");
    await page.click("text=pd-retail-scorecard");
    await page.waitForSelector("#launch");
    await page.click("#launch");
    await page.waitForURL(/#\/p\/.+\/r\/1$/, { timeout: 60000 });
    await page.waitForSelector(".seal");
    check((await page.textContent(".seal .verdict")) === "Fail", "the seal shows the run verdict");
    check((await page.$("#approve")) === null, "an owner cannot decide");
    const frame = page.frameLocator("iframe.report");
    await frame.locator("text=Non-compliant — action required").waitFor();
    check(true, "the validation report is displayed");
    await logout();

    console.log("Validator");
    await login("eli");
    await page.goto(origin + "/#/p/pd-retail-scorecard/r/1");
    await page.waitForSelector("#verify");
    await page.click("#verify");
    await page.waitForSelector("text=Identical replay", { timeout: 60000 });
    check(true, "replay confirms reproducibility");
    await page.click("#reject");
    await page.waitForSelector(".notice.fail");
    check((await page.textContent(".notice.fail")).includes("justified"), "a rejection without a comment is refused");
    await page.fill("#comment", "Grade 6 underestimated (observed rate 14.9% for a 10% PD): recalibration required before use.");
    await page.click("#reject");
    await page.waitForSelector(".timeline li.rejected");
    check((await page.textContent(".timeline")).includes("eli"), "the decision is recorded and attributed");
    await page.screenshot({ path: path.join(SHOTS, "2-run-validator.png"), fullPage: true });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: path.join(SHOTS, "3-run-mobile.png"), fullPage: true });
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    check(overflow <= 0, `no horizontal scrolling on mobile (${overflow}px)`);
    await page.setViewportSize({ width: 1280, height: 900 });

    await page.goto(origin + "/#/models/PD-RETAIL-01");
    await page.waitForSelector("#status-approved");
    check((await page.$("#edit-model")) === null, "a validator cannot edit the record");
    await page.click("#status-approved");
    await page.waitForSelector(".notice.fail");
    check((await page.textContent(".notice.fail")).includes("latest run rejected"), "model cannot be approved while its latest run is rejected");
    await page.goto(origin + "/#/models");
    await page.waitForSelector("text=Retail PD scorecard");
    await page.screenshot({ path: path.join(SHOTS, "5-inventory.png"), fullPage: true });
    const stray = await page.evaluate(() => [...document.querySelector("main").childNodes].some((n) => n.nodeType === 3 && n.textContent.trim() !== ""));
    check(!stray, "no stray text in the page");
    await logout();

    console.log("Auditor");
    await login("carol");
    check((await page.$("text=Audit log")) !== null, "the audit log is accessible");
    await page.goto(origin + "/#/p/pd-retail-scorecard");
    await page.waitForSelector("h1");
    check((await page.$("#launch")) === null, "an auditor cannot trigger a run");
    await page.goto(origin + "/#/audit");
    await page.waitForSelector("#filter");
    await page.fill("#filter", "run.decide");
    const text = await page.textContent("tbody");
    check(text.includes("eli") && text.includes("Rejected"), "the log records the validator's decision");
    await page.fill("#filter", "model.status");
    check((await page.textContent("tbody")).includes("refused: in_validation -> approved"), "the log records the refused model approval");
    await page.fill("#filter", "value-not-found");
    check((await page.textContent("tbody")).includes("No entry"), "the log filter reports no results");
    await page.fill("#filter", "");
    await page.screenshot({ path: path.join(SHOTS, "4-audit.png"), fullPage: true });
    await logout();

    console.log("Segregation of duties at SSO level");
    await page.goto(origin + "/");
    await page.click("#sso");
    await page.click("#user-fred");
    await page.waitForSelector(".notice.fail");
    check((await page.textContent(".notice.fail")).includes("segregation of duties"), "incompatible roles rejected at sign-in");

    const csp = problems.filter((p) => /Content Security Policy|Refused/i.test(p));
    check(csp.length === 0, `no security policy violation (${csp.join(" | ")})`);
    console.log("\nAll UI checks passed.");
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
