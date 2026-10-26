// ModelFlow — console de validation. Aucune dépendance ; tout le contenu est inséré en texte (pas d'innerHTML).
"use strict";

const app = document.getElementById("app");
const TOKEN_KEY = "mf.token";
const LABELS = { pass: "Conforme", warn: "Réserves", fail: "Non conforme", approved: "Approuvée", rejected: "Rejetée" };
const ROLES = { admin: "Administrateur", model_owner: "Propriétaire de modèle", validator: "Validateur", auditor: "Auditeur" };

let session = { token: null, me: null, config: null };

// ---------- Utilitaires ----------
function el(tag, attrs = {}, ...children) {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    if (k.startsWith("on")) n.addEventListener(k.slice(2), v);
    else if (k === "class") n.className = v;
    else n.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    n.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return n;
}

function state(s) {
  const key = s || "none";
  return el("span", { class: `state ${key}` }, LABELS[key] || "Sans contrôle");
}

function when(ts) {
  return new Date(ts * 1000).toLocaleString("fr-FR", { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit", timeZoneName: "short" });
}

function short(hash) {
  return (hash || "").slice(0, 12);
}

function enc(s) {
  return encodeURIComponent(s);
}

class ApiError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

async function api(path, opts = {}) {
  const headers = { ...(opts.headers || {}) };
  if (session.token) headers.Authorization = `Bearer ${session.token}`;
  if (opts.json !== undefined) {
    headers["Content-Type"] = "application/json";
    opts.body = JSON.stringify(opts.json);
  }
  const res = await fetch(path, { method: opts.method || "GET", headers, body: opts.body });
  if (res.status === 401) {
    logout("Votre session a expiré ou le jeton est refusé. Reconnectez-vous.");
    throw new ApiError(401, "non authentifié");
  }
  const text = await res.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch { data = text; }
  if (!res.ok) throw new ApiError(res.status, (data && data.error) || `erreur ${res.status}`);
  return data;
}

function can(...roles) {
  return session.me && roles.includes(session.me.role);
}

// ---------- SSO : authorization code + PKCE ----------
function b64url(bytes) {
  return btoa(String.fromCharCode(...new Uint8Array(bytes))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function randomString(n = 32) {
  return b64url(crypto.getRandomValues(new Uint8Array(n)));
}

async function startSso() {
  const sso = session.config.sso;
  const verifier = randomString(48);
  const challenge = b64url(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)));
  const st = randomString(16);
  sessionStorage.setItem("mf.pkce", JSON.stringify({ verifier, state: st, back: location.hash || "#/" }));
  const url = new URL(sso.authorization_endpoint);
  url.search = new URLSearchParams({
    response_type: "code",
    client_id: sso.client_id,
    redirect_uri: location.origin + "/",
    scope: sso.scopes,
    code_challenge: challenge,
    code_challenge_method: "S256",
    state: st,
  }).toString();
  location.assign(url.toString());
}

async function finishSso(params) {
  const saved = JSON.parse(sessionStorage.getItem("mf.pkce") || "null");
  sessionStorage.removeItem("mf.pkce");
  history.replaceState(null, "", "/" + ((saved && saved.back) || "#/"));
  if (params.get("error")) throw new Error(`Connexion refusée par le fournisseur d'identité : ${params.get("error_description") || params.get("error")}`);
  if (!saved || saved.state !== params.get("state")) throw new Error("Réponse de connexion inattendue (paramètre state différent). Relancez la connexion.");
  const sso = session.config.sso;
  const res = await fetch(sso.token_endpoint, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "authorization_code",
      code: params.get("code"),
      redirect_uri: location.origin + "/",
      client_id: sso.client_id,
      code_verifier: saved.verifier,
    }),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok || !data.access_token) throw new Error(`Échange du code refusé : ${data.error_description || data.error || res.status}`);
  return data.access_token;
}

// ---------- Session ----------
function logout(message) {
  sessionStorage.removeItem(TOKEN_KEY);
  session.token = null;
  session.me = null;
  renderLogin(message);
}

async function signIn(token) {
  session.token = token;
  session.me = await api("/api/me");
  sessionStorage.setItem(TOKEN_KEY, token);
}

function renderLogin(message, error) {
  const sso = session.config && session.config.sso;
  const tokenInput = el("input", { type: "password", id: "token", autocomplete: "off", required: true });
  const form = el("form", {
    onsubmit: async (e) => {
      e.preventDefault();
      try {
        await signIn(tokenInput.value.trim());
        route();
      } catch (err) {
        if (err.status !== 401) renderLogin(null, err.message);
      }
    },
  },
    el("div", { class: "field" }, el("label", { for: "token" }, "Jeton de service"), tokenInput),
    el("button", { class: "btn", type: "submit" }, "Se connecter avec le jeton"),
  );
  app.replaceChildren(el("div", { class: "login" }, el("div", { class: "login-box" },
    el("h1", {}, "ModelFlow"),
    el("p", { class: "lead" }, "Validation des modèles : exécutions, rejeu, décisions et piste d'audit."),
    message && el("div", { class: "notice" }, message),
    error && el("div", { class: "notice fail", role: "alert" }, error),
    sso && el("button", { class: "btn primary", id: "sso", onclick: () => startSso().catch((e) => renderLogin(null, e.message)) }, "Se connecter avec le compte de l'entreprise"),
    sso && session.config.service_tokens && el("p", { class: "sep" }, "Ou, pour un compte de service :"),
    (!sso || session.config.service_tokens) && form,
  )));
}

// ---------- Mise en page ----------
function layout(current, ...content) {
  const link = (href, label, key) => el("li", {}, el("a", { href, "aria-current": current === key ? "page" : null }, label));
  return el("div", { class: "shell" },
    el("nav", { class: "nav", "aria-label": "Navigation principale" },
      el("div", { class: "brand" }, "ModelFlow", el("small", {}, "Validation des modèles")),
      el("ul", {},
        link("#/", "Pipelines", "pipelines"),
        can("auditor", "admin") && link("#/audit", "Journal d'audit", "audit"),
      ),
      el("div", { class: "who" },
        el("div", {}, el("strong", {}, session.me.name), ROLES[session.me.role] || session.me.role),
        el("button", { onclick: () => logout("Vous êtes déconnecté.") }, "Se déconnecter"),
      ),
    ),
    el("main", { id: "main", tabindex: "-1" }, ...content),
  );
}

function show(current, ...content) {
  app.replaceChildren(layout(current, ...content));
}

function failure(err) {
  if (err.status === 401) return;
  show("", el("div", { class: "notice fail", role: "alert" }, err.message), el("p", {}, el("a", { href: "#/" }, "Revenir aux pipelines")));
}

// ---------- Pipelines ----------
async function viewPipelines() {
  const list = await api("/api/pipelines");
  const rows = list.map((p) => el("tr", { class: "link", tabindex: 0, onclick: () => (location.hash = `#/p/${enc(p.name)}`), onkeydown: (e) => e.key === "Enter" && (location.hash = `#/p/${enc(p.name)}`) },
    el("td", {}, el("a", { href: `#/p/${enc(p.name)}` }, p.name)),
    el("td", { class: "num" }, p.steps.length),
    el("td", { class: "num" }, p.runs),
    el("td", {}, p.last_run ? when(p.last_run.created_at) : el("span", { class: "muted" }, "Jamais exécuté")),
    el("td", {}, p.last_run ? state(p.last_run.status) : ""),
    el("td", {}, p.last_run && p.last_run.decision ? state(p.last_run.decision) : el("span", { class: "muted" }, p.last_run ? "En attente" : "")),
  ));
  show("pipelines",
    el("h1", {}, "Pipelines"),
    el("p", { class: "lead" }, "Chaque pipeline enchaîne données, modèles et tests de validation. Ouvrez-en un pour lancer une exécution ou statuer sur la dernière."),
    el("div", { class: "table-wrap" }, list.length
      ? el("table", {}, el("thead", {}, el("tr", {}, ["Pipeline", "Étapes", "Exécutions", "Dernière exécution", "Résultat", "Décision"].map((h) => el("th", { scope: "col" }, h)))), el("tbody", {}, rows))
      : el("p", { class: "empty" }, "Aucun pipeline dans l'espace de travail. Ajoutez un dossier contenant un fichier pipeline.toml.")),
  );
}

async function viewPipeline(name) {
  const [runs, pipelines] = await Promise.all([api(`/api/pipelines/${enc(name)}/runs`), api("/api/pipelines")]);
  const p = pipelines.find((x) => x.name === name);
  const status = el("div", { id: "launch-status" });
  const launch = can("model_owner", "admin") && el("button", {
    class: "btn primary", id: "launch",
    onclick: async (e) => {
      e.target.disabled = true;
      status.replaceChildren(el("div", { class: "progress" }, "Exécution en file d'attente…"));
      try {
        const job = await api(`/api/pipelines/${enc(name)}/runs`, { method: "POST" });
        for (;;) {
          await new Promise((r) => setTimeout(r, 800));
          const j = await api(job.href);
          if (j.status === "running") status.replaceChildren(el("div", { class: "progress" }, "Exécution en cours…"));
          if (j.status === "succeeded") { location.hash = `#/p/${enc(name)}/r/${j.run_id}`; return; }
          if (j.status === "failed") throw new Error(`L'exécution a échoué : ${j.error}`);
        }
      } catch (err) {
        e.target.disabled = false;
        status.replaceChildren(el("div", { class: "notice fail", role: "alert" }, err.message));
      }
    },
  }, "Lancer une exécution");
  const rows = runs.slice().reverse().map((r) => el("tr", { class: "link", tabindex: 0, onclick: () => (location.hash = `#/p/${enc(name)}/r/${r.id}`), onkeydown: (e) => e.key === "Enter" && (location.hash = `#/p/${enc(name)}/r/${r.id}`) },
    el("td", { class: "num" }, el("a", { href: `#/p/${enc(name)}/r/${r.id}` }, `#${r.id}`)),
    el("td", {}, when(r.created_at)),
    el("td", {}, r.triggered_by),
    el("td", {}, state(r.status)),
    el("td", {}, r.decision ? state(r.decision) : el("span", { class: "muted" }, "En attente")),
    el("td", { class: "hash" }, short(r.run_hash)),
  ));
  show("pipelines",
    el("div", { class: "crumbs" }, el("a", { href: "#/" }, "Pipelines")),
    el("div", { class: "toolbar" }, el("div", {}, el("h1", {}, name), el("p", { class: "lead" }, p ? `${p.steps.length} étapes : ${p.steps.map((s) => s.name).join(", ")}.` : "")), launch),
    status,
    el("h2", {}, "Exécutions"),
    el("div", { class: "table-wrap" }, runs.length
      ? el("table", {}, el("thead", {}, el("tr", {}, ["N°", "Date", "Lancée par", "Résultat", "Décision", "Empreinte"].map((h) => el("th", { scope: "col" }, h)))), el("tbody", {}, rows))
      : el("p", { class: "empty" }, can("model_owner", "admin") ? "Aucune exécution. Lancez la première avec le bouton ci-dessus." : "Aucune exécution pour l'instant.")),
  );
}

// ---------- Exécution ----------
async function viewRun(name, id) {
  const base = `/api/pipelines/${enc(name)}/runs/${id}`;
  const [runs, decisions, report] = await Promise.all([
    api(`/api/pipelines/${enc(name)}/runs`),
    api(`${base}/decisions`),
    fetch(`${base}/report?no_verify=true`, { headers: { Authorization: `Bearer ${session.token}` } }).then((r) => (r.ok ? r.text() : null)),
  ]);
  const run = runs.find((r) => r.id === Number(id));
  if (!run) throw new Error(`L'exécution #${id} n'existe pas pour ${name}.`);
  const sealClass = run.status || "none";

  const replay = el("div", { id: "replay" });
  const replayBtn = can("validator", "model_owner", "admin") && el("button", {
    class: "btn", id: "verify",
    onclick: async (e) => {
      e.target.disabled = true;
      replay.replaceChildren(el("div", { class: "progress" }, "Rejeu en cours : le pipeline est réexécuté et comparé à l'enregistrement…"));
      try {
        const v = await api(`${base}/verify`, { method: "POST" });
        replay.replaceChildren(v.reproducible
          ? el("div", { class: "notice pass" }, "Rejeu identique : données, code et résultats correspondent à l'enregistrement.")
          : el("div", { class: "notice fail", role: "alert" }, "Le rejeu diffère de l'enregistrement :", el("ul", {}, v.mismatches.map((m) => el("li", {}, `${m.step} : ${m.reason}`)))));
      } catch (err) {
        replay.replaceChildren(el("div", { class: "notice fail", role: "alert" }, err.message));
      } finally {
        e.target.disabled = false;
      }
    },
  }, "Rejouer l'exécution");

  let decide = null;
  if (can("validator")) {
    if (run.triggered_by === session.me.name) {
      decide = el("p", { class: "muted" }, "Vous avez lancé cette exécution : un autre validateur doit statuer (séparation des tâches).");
    } else {
      const comment = el("textarea", { id: "comment", placeholder: "Constats, réserves, conditions d'utilisation…" });
      const msg = el("div");
      const send = async (decision) => {
        try {
          await api(`${base}/decisions`, { method: "POST", json: { decision, comment: comment.value } });
          route();
        } catch (err) {
          msg.replaceChildren(el("div", { class: "notice fail", role: "alert" }, err.message));
        }
      };
      decide = el("div", {},
        el("div", { class: "field" }, el("label", { for: "comment" }, "Commentaire (obligatoire pour un rejet)"), comment),
        el("div", { class: "actions" },
          el("button", { class: "btn approve", id: "approve", onclick: () => send("approved") }, "Approuver"),
          el("button", { class: "btn reject", id: "reject", onclick: () => send("rejected") }, "Rejeter"),
        ),
        el("p", { class: "muted" }, "La décision n'est enregistrée que si le rejeu est identique."),
        msg,
      );
    }
  }

  const frame = el("iframe", { class: "report", title: `Rapport de validation ${name} #${id}`, sandbox: "" });
  frame.srcdoc = report || "<p>Rapport indisponible.</p>";

  show("pipelines",
    el("div", { class: "crumbs" }, el("a", { href: "#/" }, "Pipelines"), " / ", el("a", { href: `#/p/${enc(name)}` }, name)),
    el("div", { class: "run-head" },
      el("div", { class: `seal ${sealClass}`, role: "img", "aria-label": `Résultat : ${LABELS[sealClass] || "sans contrôle"}, empreinte ${short(run.run_hash)}` },
        el("div", {}, el("div", { class: "verdict" }, LABELS[sealClass] || "Sans contrôle"), el("div", { class: "print" }, run.run_hash.slice(0, 8))),
      ),
      el("div", {},
        el("h1", {}, `${name}, exécution #${id}`),
        el("dl", { class: "facts" },
          el("div", {}, el("dt", {}, "Lancée le"), el("dd", {}, when(run.created_at))),
          el("div", {}, el("dt", {}, "Par"), el("dd", {}, run.triggered_by)),
          el("div", {}, el("dt", {}, "Décision"), el("dd", {}, run.decision ? state(run.decision) : "En attente")),
          el("div", {}, el("dt", {}, "Empreinte de l'exécution"), el("dd", { class: "hash", title: run.run_hash }, short(run.run_hash))),
        ),
      ),
    ),
    el("div", { class: "columns" },
      el("section", { "aria-label": "Rapport de validation" }, el("h2", {}, "Rapport de validation"), frame),
      el("aside", {},
        el("div", { class: "panel" }, el("h2", {}, "Reproductibilité"),
          el("p", { class: "muted" }, "Le rejeu réexécute le pipeline et compare chaque empreinte (données, code, résultats)."),
          replayBtn, replay),
        el("div", { class: "panel" }, el("h2", {}, "Décisions"),
          decisions.length
            ? el("ul", { class: "timeline" }, decisions.map((d) => el("li", { class: d.decision },
                el("div", {}, state(d.decision), " ", el("strong", {}, d.user)),
                el("div", { class: "meta" }, when(d.created_at)),
                d.comment && el("p", {}, d.comment))))
            : el("p", { class: "muted" }, "Aucune décision pour cette exécution."),
          decide && el("h2", {}, "Statuer"),
          decide),
      ),
    ),
  );
}

// ---------- Audit ----------
const OUTCOMES = { ok: "Réussi", denied: "Refusé (droits)", approved: "Approuvée", rejected: "Rejetée", reproducible: "Reproductible", mismatch: "Écart au rejeu" };

function outcomeClass(o) {
  if (/^(denied|refused|error|mismatch|rejected)/.test(o)) return "fail";
  if (o === "approved" || o === "reproducible") return "pass";
  return "none";
}

async function viewAudit() {
  const log = await api("/api/audit?limit=500");
  const tbody = el("tbody");
  const fill = (q) => {
    const needle = q.toLowerCase();
    const rows = log.filter((e) => !needle || [e.user, e.role, e.action, e.target, e.outcome].join(" ").toLowerCase().includes(needle));
    tbody.replaceChildren(...rows.map((e) => el("tr", {},
      el("td", {}, when(e.ts)),
      el("td", {}, e.user),
      el("td", {}, ROLES[e.role] || e.role),
      el("td", {}, e.action),
      el("td", {}, e.target),
      el("td", {}, el("span", { class: `state ${outcomeClass(e.outcome)}` }, OUTCOMES[e.outcome] || e.outcome)),
    )));
    if (!rows.length) tbody.replaceChildren(el("tr", {}, el("td", { colspan: 6, class: "empty" }, "Aucune entrée ne correspond à ce filtre.")));
  };
  const search = el("input", { type: "search", id: "filter", placeholder: "Filtrer : utilisateur, action, cible…", "aria-label": "Filtrer le journal", oninput: (e) => fill(e.target.value) });
  fill("");
  show("audit",
    el("div", { class: "toolbar" }, el("div", {}, el("h1", {}, "Journal d'audit"), el("p", { class: "lead" }, "Toutes les actions, y compris les refus. Le journal est en ajout seul : la base refuse toute modification.")), search),
    el("div", { class: "table-wrap" }, el("table", {}, el("thead", {}, el("tr", {}, ["Date", "Utilisateur", "Rôle", "Action", "Cible", "Résultat"].map((h) => el("th", { scope: "col" }, h)))), tbody)),
  );
}

// ---------- Routage ----------
async function route() {
  if (!session.token) return renderLogin();
  const parts = location.hash.replace(/^#\/?/, "").split("/").map(decodeURIComponent);
  try {
    if (parts[0] === "p" && parts[2] === "r") await viewRun(parts[1], parts[3]);
    else if (parts[0] === "p" && parts[1]) await viewPipeline(parts[1]);
    else if (parts[0] === "audit") await viewAudit();
    else await viewPipelines();
    document.getElementById("main")?.focus?.();
  } catch (err) {
    failure(err);
  }
}

async function boot() {
  session.config = await fetch("/api/ui-config").then((r) => r.json()).catch(() => ({ sso: null, service_tokens: true }));
  const params = new URLSearchParams(location.search);
  try {
    if (params.get("code") || params.get("error")) await signIn(await finishSso(params));
    else if (sessionStorage.getItem(TOKEN_KEY)) await signIn(sessionStorage.getItem(TOKEN_KEY));
  } catch (err) {
    if (err.status !== 401) return renderLogin(null, err.message);
  }
  window.addEventListener("hashchange", route);
  route();
}

boot();
