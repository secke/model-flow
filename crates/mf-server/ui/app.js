// ModelFlow — validation console. No dependencies; all content is inserted as text (no innerHTML).
"use strict";

const app = document.getElementById("app");
const TOKEN_KEY = "mf.token";
const LABELS = { pass: "Pass", warn: "Warning", fail: "Fail", approved: "Approved", rejected: "Rejected" };
const MODEL_STATES = {
  development: ["In development", "none"],
  in_validation: ["In validation", "warn"],
  approved: ["Approved", "pass"],
  in_use: ["In use", "pass"],
  retired: ["Retired", "none"],
};
const TIERS = { 1: "1 — high risk", 2: "2 — medium risk", 3: "3 — low risk" };
const ROLES = { admin: "Administrator", model_owner: "Model owner", validator: "Validator", auditor: "Auditor" };
const SEVERITY = { critical: ["Critical", "fail"], high: ["High", "fail"], medium: ["Medium", "warn"], low: ["Low", "none"] };
const FINDING_STATES = { open: ["Open", "warn"], remediated: ["Remediated, awaiting review", "warn"], closed: ["Closed", "pass"], risk_accepted: ["Risk accepted", "none"] };

let session = { token: null, me: null, config: null };

// ---------- Utilities ----------
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
  return el("span", { class: `state ${key}` }, LABELS[key] || "No checks");
}

function when(ts) {
  return new Date(ts * 1000).toLocaleString("en-GB", { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit", timeZoneName: "short" });
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
    logout("Your session has expired or the token was rejected. Please sign in again.");
    throw new ApiError(401, "not authenticated");
  }
  const text = await res.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch { data = text; }
  if (!res.ok) throw new ApiError(res.status, (data && data.error) || `error ${res.status}`);
  return data;
}

function can(...roles) {
  return session.me && roles.includes(session.me.role);
}

// ---------- SSO: authorization code + PKCE ----------
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
  if (params.get("error")) throw new Error(`Sign-in rejected by the identity provider: ${params.get("error_description") || params.get("error")}`);
  if (!saved || saved.state !== params.get("state")) throw new Error("Unexpected sign-in response (state parameter mismatch). Please sign in again.");
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
  if (!res.ok || !data.access_token) throw new Error(`Code exchange rejected: ${data.error_description || data.error || res.status}`);
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
    el("div", { class: "field" }, el("label", { for: "token" }, "Service token"), tokenInput),
    el("button", { class: "btn", type: "submit" }, "Sign in with the token"),
  );
  app.replaceChildren(el("div", { class: "login" }, el("div", { class: "login-box" },
    el("h1", {}, "ModelFlow"),
    el("p", { class: "lead" }, "Model validation: runs, replay, decisions and audit trail."),
    message && el("div", { class: "notice" }, message),
    error && el("div", { class: "notice fail", role: "alert" }, error),
    sso && el("button", { class: "btn primary", id: "sso", onclick: () => startSso().catch((e) => renderLogin(null, e.message)) }, "Sign in with your company account"),
    sso && session.config.service_tokens && el("p", { class: "sep" }, "Or, for a service account:"),
    (!sso || session.config.service_tokens) && form,
  )));
}

// ---------- Layout ----------
function layout(current, ...content) {
  const link = (href, label, key) => el("li", {}, el("a", { href, "aria-current": current === key ? "page" : null }, label));
  return el("div", { class: "shell" },
    el("nav", { class: "nav", "aria-label": "Main navigation" },
      el("div", { class: "brand" }, "ModelFlow", el("small", {}, "Model validation")),
      el("ul", {},
        link("#/", "Pipelines", "pipelines"),
        link("#/models", "Model inventory", "models"),
        link("#/findings", "Findings", "findings"),
        can("auditor", "admin") && link("#/audit", "Audit log", "audit"),
      ),
      el("div", { class: "who" },
        el("div", {}, el("strong", {}, session.me.name), ROLES[session.me.role] || session.me.role),
        el("button", { onclick: () => logout("You are signed out.") }, "Sign out"),
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
  show("", el("div", { class: "notice fail", role: "alert" }, err.message), el("p", {}, el("a", { href: "#/" }, "Back to pipelines")));
}

// ---------- Pipelines ----------
async function viewPipelines() {
  const at = location.hash;
  const list = await api("/api/pipelines");
  const rows = list.map((p) => el("tr", { class: "link", tabindex: 0, onclick: () => (location.hash = `#/p/${enc(p.name)}`), onkeydown: (e) => e.key === "Enter" && (location.hash = `#/p/${enc(p.name)}`) },
    el("td", {}, el("a", { href: `#/p/${enc(p.name)}` }, p.name)),
    el("td", { class: "num" }, p.steps.length),
    el("td", { class: "num" }, p.runs),
    el("td", {}, p.last_run ? when(p.last_run.created_at) : el("span", { class: "muted" }, "Never run")),
    el("td", {}, p.last_run ? state(p.last_run.status) : ""),
    el("td", {}, p.last_run && p.last_run.decision ? state(p.last_run.decision) : el("span", { class: "muted" }, p.last_run ? "Pending" : "")),
  ));
  if (location.hash !== at) return; // the user has moved on: do not render over the new page
  show("pipelines",
    el("h1", {}, "Pipelines"),
    el("p", { class: "lead" }, "Each pipeline chains data, models and validation tests. Open one to trigger a run or decide on the latest one."),
    el("div", { class: "table-wrap" }, list.length
      ? el("table", {}, el("thead", {}, el("tr", {}, ["Pipeline", "Steps", "Runs", "Latest run", "Result", "Decision"].map((h) => el("th", { scope: "col" }, h)))), el("tbody", {}, rows))
      : el("p", { class: "empty" }, "No pipeline in the workspace. Add a directory containing a pipeline.toml file.")),
  );
}

async function viewPipeline(name) {
  const at = location.hash;
  const [runs, pipelines, alerts] = await Promise.all([
    api(`/api/pipelines/${enc(name)}/runs`),
    api("/api/pipelines"),
    api(`/api/pipelines/${enc(name)}/alerts`).catch(() => null),
  ]);
  const p = pipelines.find((x) => x.name === name);
  const status = el("div", { id: "launch-status" });
  const launch = can("model_owner", "admin") && el("button", {
    class: "btn primary", id: "launch",
    onclick: async (e) => {
      e.target.disabled = true;
      status.replaceChildren(el("div", { class: "progress" }, "Run queued…"));
      try {
        const job = await api(`/api/pipelines/${enc(name)}/runs`, { method: "POST" });
        for (;;) {
          await new Promise((r) => setTimeout(r, 800));
          const j = await api(job.href);
          if (j.status === "running") status.replaceChildren(el("div", { class: "progress" }, "Run in progress…"));
          if (j.status === "succeeded") { location.hash = `#/p/${enc(name)}/r/${j.run_id}`; return; }
          if (j.status === "failed") throw new Error(`The run failed: ${j.error}`);
        }
      } catch (err) {
        e.target.disabled = false;
        status.replaceChildren(el("div", { class: "notice fail", role: "alert" }, err.message));
      }
    },
  }, "Trigger a run");
  const dated = runs.some((r) => r.as_of);
  const rows = runs.slice().reverse().map((r) => el("tr", { class: "link", tabindex: 0, onclick: () => (location.hash = `#/p/${enc(name)}/r/${r.id}`), onkeydown: (e) => e.key === "Enter" && (location.hash = `#/p/${enc(name)}/r/${r.id}`) },
    el("td", { class: "num" }, el("a", { href: `#/p/${enc(name)}/r/${r.id}` }, `#${r.id}`)),
    el("td", {}, when(r.created_at)),
    dated && el("td", {}, r.as_of || ""),
    el("td", {}, r.triggered_by),
    el("td", {}, state(r.status)),
    el("td", {}, r.decision ? state(r.decision) : el("span", { class: "muted" }, "Pending")),
    el("td", { class: "hash" }, short(r.run_hash)),
  ));
  // Alert rules of the pipeline ([[alert]]): state at the latest date and latest episode.
  const rules = (alerts && alerts.report.alerts) || [];
  const period = (e) => e ? `${e.start} → ${e.ongoing ? "ongoing" : e.last} (${e.dates} ${e.dates === 1 ? "date" : "dates"})` : "Never raised";
  const alertRows = rules.map((a) => el("tr", {},
    el("td", {}, a.rule.name),
    el("td", { class: "hash" }, a.condition),
    el("td", {}, a.active ? el("span", { class: "state fail" }, "Active") : el("span", { class: "state pass" }, "Clear")),
    el("td", {}, String(a.episodes.length)),
    el("td", {}, period(a.episodes[a.episodes.length - 1])),
  ));
  if (location.hash !== at) return; // the user has moved on: do not render over the new page
  show("pipelines",
    el("div", { class: "crumbs" }, el("a", { href: "#/" }, "Pipelines")),
    el("div", { class: "toolbar" }, el("div", {}, el("h1", {}, name), el("p", { class: "lead" }, p ? `${p.steps.length} steps: ${p.steps.map((s) => s.name).join(", ")}.` : "")), launch),
    p && (p.model || p.schedule) && el("dl", { class: "facts" },
      p.model && el("div", {}, el("dt", {}, "Model"), el("dd", {}, el("a", { href: `#/models/${enc(p.model)}` }, p.model))),
      p.schedule && el("div", {}, el("dt", {}, "Schedule (UTC)"), el("dd", { class: "hash" }, p.schedule)),
      p.next_run && el("div", {}, el("dt", {}, "Next run"), el("dd", {}, when(p.next_run))),
    ),
    status,
    rules.length > 0 && el("h2", {}, "Alerts"),
    rules.length > 0 && el("div", { class: "table-wrap" }, el("table", { id: "alerts" },
      el("thead", {}, el("tr", {}, ["Alert", "Condition", "State", "Episodes", "Latest episode"].map((h) => el("th", { scope: "col" }, h)))),
      el("tbody", {}, alertRows))),
    el("h2", {}, "Runs"),
    el("div", { class: "table-wrap" }, runs.length
      ? el("table", {}, el("thead", {}, el("tr", {}, ["No.", "Date", dated && "As of", "Triggered by", "Result", "Decision", "Fingerprint"].filter(Boolean).map((h) => el("th", { scope: "col" }, h)))), el("tbody", {}, rows))
      : el("p", { class: "empty" }, can("model_owner", "admin") ? "No runs yet. Trigger the first one with the button above." : "No runs yet.")),
  );
}

// ---------- Run ----------
async function viewRun(name, id) {
  const at = location.hash;
  const base = `/api/pipelines/${enc(name)}/runs/${id}`;
  const [runs, decisions, report] = await Promise.all([
    api(`/api/pipelines/${enc(name)}/runs`),
    api(`${base}/decisions`),
    fetch(`${base}/report?no_verify=true`, { headers: { Authorization: `Bearer ${session.token}` } }).then((r) => (r.ok ? r.text() : null)),
  ]);
  const run = runs.find((r) => r.id === Number(id));
  if (!run) throw new Error(`Run #${id} does not exist for ${name}.`);
  const sealClass = run.status || "none";

  const replay = el("div", { id: "replay" });
  const replayBtn = can("validator", "model_owner", "admin") && el("button", {
    class: "btn", id: "verify",
    onclick: async (e) => {
      e.target.disabled = true;
      replay.replaceChildren(el("div", { class: "progress" }, "Replay in progress: the pipeline is re-run and compared with the record…"));
      try {
        const v = await api(`${base}/verify`, { method: "POST" });
        replay.replaceChildren(v.reproducible
          ? el("div", { class: "notice pass" }, "Identical replay: data, code and results match the record.")
          : el("div", { class: "notice fail", role: "alert" }, "The replay differs from the record:", el("ul", {}, v.mismatches.map((m) => el("li", {}, `${m.step}: ${m.reason}`)))));
      } catch (err) {
        replay.replaceChildren(el("div", { class: "notice fail", role: "alert" }, err.message));
      } finally {
        e.target.disabled = false;
      }
    },
  }, "Replay the run");

  let decide = null;
  if (can("validator")) {
    if (run.triggered_by === session.me.name) {
      decide = el("p", { class: "muted" }, "You triggered this run: another validator must decide (segregation of duties).");
    } else {
      const comment = el("textarea", { id: "comment", placeholder: "Findings, reservations, conditions of use…" });
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
        el("div", { class: "field" }, el("label", { for: "comment" }, "Comment (required for a rejection)"), comment),
        el("div", { class: "actions" },
          el("button", { class: "btn approve", id: "approve", onclick: () => send("approved") }, "Approve"),
          el("button", { class: "btn reject", id: "reject", onclick: () => send("rejected") }, "Reject"),
        ),
        el("p", { class: "muted" }, "The decision is only recorded if the replay is identical."),
        msg,
      );
    }
  }

  const pipeline = (await api("/api/pipelines")).find((x) => x.name === name);
  const runFindings = (await api("/api/findings")).findings.filter((f) => f.pipeline === name && f.run_id === Number(id));
  const raise = pipeline && pipeline.model && can("validator") &&
    el("details", {}, el("summary", { id: "raise-finding" }, "Raise a finding on this run"),
      findingForm({ model: pipeline.model, pipeline: name, run_id: Number(id) }));

  const frame = el("iframe", { class: "report", title: `Validation report ${name} #${id}`, sandbox: "" });
  frame.srcdoc = report || "<p>Report unavailable.</p>";

  if (location.hash !== at) return; // the user has moved on: do not render over the new page
  show("pipelines",
    el("div", { class: "crumbs" }, el("a", { href: "#/" }, "Pipelines"), " / ", el("a", { href: `#/p/${enc(name)}` }, name)),
    el("div", { class: "run-head" },
      el("div", { class: `seal ${sealClass}`, role: "img", "aria-label": `Result: ${LABELS[sealClass] || "no checks"}, fingerprint ${short(run.run_hash)}` },
        el("div", {}, el("div", { class: "verdict" }, LABELS[sealClass] || "No checks"), el("div", { class: "print" }, run.run_hash.slice(0, 8))),
      ),
      el("div", {},
        el("h1", {}, `${name}, run #${id}`),
        el("dl", { class: "facts" },
          el("div", {}, el("dt", {}, "Triggered on"), el("dd", {}, when(run.created_at))),
          el("div", {}, el("dt", {}, "By"), el("dd", {}, run.triggered_by)),
          run.as_of && el("div", {}, el("dt", {}, "As of"), el("dd", {}, run.as_of)),
          el("div", {}, el("dt", {}, "Decision"), el("dd", {}, run.decision ? state(run.decision) : "Pending")),
          el("div", {}, el("dt", {}, "Run fingerprint"), el("dd", { class: "hash", title: run.run_hash }, short(run.run_hash))),
        ),
      ),
    ),
    el("div", { class: "columns" },
      el("section", { "aria-label": "Validation report" }, el("h2", {}, "Validation report"), frame),
      el("aside", {},
        el("div", { class: "panel" }, el("h2", {}, "Reproducibility"),
          el("p", { class: "muted" }, "Replay re-runs the pipeline and compares every fingerprint (data, code, results)."),
          replayBtn, replay),
        el("div", { class: "panel" }, el("h2", {}, "Decisions"),
          decisions.length
            ? el("ul", { class: "timeline" }, decisions.map((d) => el("li", { class: d.decision },
                el("div", {}, state(d.decision), " ", el("strong", {}, d.user)),
                el("div", { class: "meta" }, when(d.created_at)),
                d.comment && el("p", {}, d.comment))))
            : el("p", { class: "muted" }, "No decision for this run."),
          decide && el("h2", {}, "Decide"),
          decide),
        el("div", { class: "panel" }, el("h2", {}, "Findings on this run"),
          runFindings.length ? findingList(runFindings) : el("p", { class: "muted" }, "No finding raised on this run."),
          raise),
      ),
    ),
  );
}

// ---------- Model inventory ----------
function modelState(s) {
  const [label, cls] = MODEL_STATES[s] || [s, "none"];
  return el("span", { class: `state ${cls}` }, label);
}

function modelForm(initial, onSave, submitLabel) {
  const m = initial || {};
  const f = {
    id: el("input", { type: "text", id: "m-id", value: m.id || "", required: true, readonly: initial ? true : null, pattern: "[A-Za-z0-9._-]{1,64}" }),
    name: el("input", { type: "text", id: "m-name", value: m.name || "", required: true }),
    owner: el("input", { type: "text", id: "m-owner", value: m.owner || session.me.name, required: true, readonly: session.me.role === "model_owner" ? true : null }),
    purpose: el("textarea", { id: "m-purpose" }, m.purpose || ""),
    tier: el("select", { id: "m-tier" }, Object.entries(TIERS).map(([v, l]) => el("option", { value: v, selected: String(m.tier || 2) === v ? true : null }, l))),
    review: el("input", { type: "date", id: "m-review", value: m.next_review || "" }),
  };
  const msg = el("div");
  const field = (key, label, hint) => el("div", { class: "field" }, el("label", { for: f[key].id }, label), f[key], hint && el("span", { class: "muted" }, hint));
  return el("form", {
    class: "panel model-form",
    onsubmit: async (e) => {
      e.preventDefault();
      try {
        await onSave({ id: f.id.value.trim(), name: f.name.value.trim(), owner: f.owner.value.trim(), purpose: f.purpose.value.trim(), tier: Number(f.tier.value), next_review: f.review.value || null });
      } catch (err) {
        msg.replaceChildren(el("div", { class: "notice fail", role: "alert" }, err.message));
      }
    },
  },
    field("id", "Identifier", initial ? null : "Letters, digits, dots, dashes. The same identifier is declared in pipeline.toml (model = …)."),
    field("name", "Name"),
    field("owner", "Owner"),
    field("purpose", "Purpose"),
    field("tier", "Risk tier"),
    field("review", "Next review"),
    el("div", { class: "actions" }, el("button", { class: "btn primary", type: "submit", id: "m-save" }, submitLabel)),
    msg,
  );
}

async function viewModels() {
  const at = location.hash;
  const data = await api("/api/models");
  const createBox = el("div");
  const add = can("model_owner", "admin") && el("button", {
    class: "btn primary", id: "new-model",
    onclick: () => createBox.replaceChildren(modelForm(null, async (m) => {
      await api("/api/models", { method: "POST", json: m });
      location.hash = `#/models/${enc(m.id)}`;
    }, "Register the model")),
  }, "Register a model");
  const rows = data.models.map((m) => {
    const last = m.pipelines.map((p) => p.last_run).filter(Boolean).sort((a, b) => b.created_at - a.created_at)[0];
    return el("tr", { class: "link", tabindex: 0, onclick: () => (location.hash = `#/models/${enc(m.id)}`), onkeydown: (e) => e.key === "Enter" && (location.hash = `#/models/${enc(m.id)}`) },
      el("td", {}, el("a", { href: `#/models/${enc(m.id)}` }, m.id)),
      el("td", {}, m.name),
      el("td", {}, m.owner),
      el("td", { class: "num" }, m.tier),
      el("td", {}, modelState(m.status)),
      el("td", {}, m.next_review ? (m.review_overdue ? el("span", { class: "state fail" }, `Overdue (${m.next_review})`) : m.next_review) : el("span", { class: "muted" }, "Not set")),
      el("td", {}, m.pipelines.length ? m.pipelines.map((p) => p.pipeline).join(", ") : el("span", { class: "muted" }, "None")),
      el("td", {}, last ? state(last.status) : el("span", { class: "muted" }, "Never run")),
    );
  });
  if (location.hash !== at) return; // the user has moved on: do not render over the new page
  show("models",
    el("div", { class: "toolbar" }, el("div", {}, el("h1", {}, "Model inventory"), el("p", { class: "lead" }, "Every model, its owner, risk tier, status and next review. Only a validator can approve a model.")), add),
    createBox,
    el("div", { class: "table-wrap" }, data.models.length
      ? el("table", {}, el("thead", {}, el("tr", {}, ["Identifier", "Name", "Owner", "Tier", "Status", "Next review", "Pipelines", "Latest result"].map((h) => el("th", { scope: "col" }, h)))), el("tbody", {}, rows))
      : el("p", { class: "empty" }, "No registered model.")),
    data.unregistered.length > 0 && el("div", { class: "notice" }, `Pipelines linked to a model missing from the inventory: ${data.unregistered.join(", ")}.`),
  );
}

async function viewModel(id) {
  const at = location.hash;
  const data = await api("/api/models");
  const m = data.models.find((x) => x.id === id);
  if (!m) throw new Error(`Model ${id} is not in the inventory.`);
  const msg = el("div");
  const setStatus = async (status) => {
    try {
      await api(`/api/models/${enc(id)}/status`, { method: "POST", json: { status } });
      route();
    } catch (err) {
      msg.replaceChildren(el("div", { class: "notice fail", role: "alert" }, err.message));
    }
  };
  const own = session.me.role === "admin" || (session.me.role === "model_owner" && m.owner === session.me.name);
  const btn = (to, label, cls = "btn") => el("button", { class: cls, id: `status-${to}`, onclick: () => setStatus(to) }, label);
  const actions = [];
  if (can("validator") && m.status !== "retired") {
    if (m.status !== "approved") actions.push(btn("approved", "Approve the model", "btn approve"));
    if (m.status !== "in_validation") actions.push(btn("in_validation", "Send back to validation"));
  }
  if (own) {
    if (m.status === "development") actions.push(btn("in_validation", "Submit for validation", "btn primary"));
    if (m.status === "approved") actions.push(btn("in_use", "Put into use", "btn primary"));
    if (m.status !== "development" && m.status !== "retired") actions.push(btn("development", "Back to development"));
    if (m.status !== "retired") actions.push(btn("retired", "Retire the model", "btn reject"));
  }
  const editBox = el("div");
  const modelFindings = (await api(`/api/findings?model=${enc(id)}`)).findings;
  const raise = can("validator") && el("details", {}, el("summary", { id: "raise-finding" }, "Raise a finding"), findingForm({ model: id }));
  if (location.hash !== at) return; // the user has moved on: do not render over the new page
  show("models",
    el("div", { class: "crumbs" }, el("a", { href: "#/models" }, "Model inventory")),
    el("div", { class: "toolbar" }, el("div", {}, el("h1", {}, `${m.id} · ${m.name}`), el("p", { class: "lead" }, m.purpose || "")),
      own && el("button", { class: "btn", id: "edit-model", onclick: () => editBox.replaceChildren(modelForm(m, async (u) => {
        await api(`/api/models/${enc(id)}`, { method: "PUT", json: u });
        route();
      }, "Save changes")) }, "Edit record")),
    el("dl", { class: "facts" },
      el("div", {}, el("dt", {}, "Status"), el("dd", {}, modelState(m.status))),
      el("div", {}, el("dt", {}, "Owner"), el("dd", {}, m.owner)),
      el("div", {}, el("dt", {}, "Risk tier"), el("dd", {}, TIERS[m.tier] || m.tier)),
      el("div", {}, el("dt", {}, "Next review"), el("dd", {}, m.next_review ? (m.review_overdue ? el("span", { class: "state fail" }, `Overdue (${m.next_review})`) : m.next_review) : "Not set")),
      el("div", {}, el("dt", {}, "Last modified"), el("dd", {}, `${when(m.updated_at)} by ${m.updated_by}`)),
      el("div", {}, el("dt", {}, "Active findings"), el("dd", {}, m.findings.blocking
        ? el("span", { class: "state fail" }, `${m.findings.active} (${m.findings.blocking} blocking approval)`)
        : String(m.findings.active))),
    ),
    actions.length > 0 && el("div", { class: "actions", id: "model-actions" }, actions),
    msg,
    editBox,
    el("h2", {}, "Linked pipelines"),
    el("div", { class: "table-wrap" }, m.pipelines.length
      ? el("table", {}, el("thead", {}, el("tr", {}, ["Pipeline", "Schedule (UTC)", "Latest run", "Result", "Decision"].map((h) => el("th", { scope: "col" }, h)))),
          el("tbody", {}, m.pipelines.map((p) => el("tr", {},
            el("td", {}, el("a", { href: `#/p/${enc(p.pipeline)}` }, p.pipeline)),
            el("td", { class: "hash" }, p.schedule || "—"),
            el("td", {}, p.last_run ? el("a", { href: `#/p/${enc(p.pipeline)}/r/${p.last_run.id}` }, `#${p.last_run.id} · ${when(p.last_run.created_at)}`) : el("span", { class: "muted" }, "Never run")),
            el("td", {}, p.last_run ? state(p.last_run.status) : ""),
            el("td", {}, p.last_run && p.last_run.decision ? state(p.last_run.decision) : el("span", { class: "muted" }, p.last_run ? "Pending" : "")),
          ))))
      : el("p", { class: "empty" }, `No pipeline declares this model. Add model = "${m.id}" to the relevant pipeline.toml.`)),
    el("h2", {}, "Findings"),
    modelFindings.length ? findingList(modelFindings) : el("p", { class: "empty" }, "No finding on this model."),
    raise,
  );
}

// ---------- Findings ----------
function severity(s) {
  const [label, cls] = SEVERITY[s] || [s, "none"];
  return el("span", { class: `state ${cls}` }, label);
}

function findingState(f) {
  if (f.overdue_days) return el("span", { class: "state fail" }, `Overdue by ${f.overdue_days} days`);
  const [label, cls] = FINDING_STATES[f.status] || [f.status, "none"];
  return el("span", { class: `state ${cls}` }, label);
}

function findingList(list) {
  return el("div", { class: "table-wrap" }, el("table", { class: "findings" },
    el("thead", {}, el("tr", {}, ["No.", "Model", "Severity", "Title", "Owner", "Due", "Status"].map((h) => el("th", { scope: "col" }, h)))),
    el("tbody", {}, list.map((f) => el("tr", { class: "link", tabindex: 0, onclick: () => (location.hash = `#/findings/${f.id}`), onkeydown: (e) => e.key === "Enter" && (location.hash = `#/findings/${f.id}`) },
      el("td", { class: "num" }, el("a", { href: `#/findings/${f.id}` }, `#${f.id}`)),
      el("td", {}, f.model),
      el("td", {}, severity(f.severity)),
      el("td", {}, f.title),
      el("td", {}, f.owner),
      el("td", {}, f.due_date),
      el("td", {}, findingState(f)),
    )))));
}

function findingForm(base) {
  const f = {
    title: el("input", { type: "text", id: "f-title", required: true }),
    severity: el("select", { id: "f-severity" }, Object.entries(SEVERITY).map(([v, [l]]) => el("option", { value: v, selected: v === "medium" ? true : null }, l))),
    description: el("textarea", { id: "f-description", required: true, placeholder: "What is wrong, the evidence, the impact on the model's use…" }),
    step: el("input", { type: "text", id: "f-step", placeholder: "Optional: the step concerned" }),
    due: el("input", { type: "date", id: "f-due" }),
  };
  const msg = el("div");
  const field = (key, label, hint) => el("div", { class: "field" }, el("label", { for: f[key].id }, label), f[key], hint && el("span", { class: "muted" }, hint));
  return el("form", {
    class: "panel finding-form",
    onsubmit: async (e) => {
      e.preventDefault();
      try {
        const body = { ...base, title: f.title.value.trim(), severity: f.severity.value, description: f.description.value.trim(), step: f.step.value.trim() || null, due_date: f.due.value || null };
        const created = await api("/api/findings", { method: "POST", json: body });
        location.hash = `#/findings/${created.id}`;
      } catch (err) {
        msg.replaceChildren(el("div", { class: "notice fail", role: "alert" }, err.message));
      }
    },
  },
    field("title", "Title"),
    field("severity", "Severity", "Critical and high findings block the model's approval until they are closed."),
    field("description", "Description"),
    base.pipeline && field("step", "Step"),
    field("due", "Remediation deadline", "Default: 30, 90, 180 or 365 days by severity."),
    el("div", { class: "actions" }, el("button", { class: "btn primary", type: "submit", id: "f-save" }, "Raise the finding")),
    msg,
  );
}

async function viewFindings() {
  const at = location.hash;
  const [data, report] = await Promise.all([api("/api/findings"), api("/api/findings/report")]);
  const tbody = el("div");
  const filters = {
    status: el("select", { id: "filter-status", "aria-label": "Status" },
      [["active", "Active"], ["", "All"], ...Object.entries(FINDING_STATES).map(([k, [l]]) => [k, l])].map(([v, l]) => el("option", { value: v }, l))),
    severity: el("select", { id: "filter-severity", "aria-label": "Severity" },
      [["", "All severities"], ...Object.entries(SEVERITY).map(([k, [l]]) => [k, l])].map(([v, l]) => el("option", { value: v }, l))),
  };
  const fill = () => {
    const st = filters.status.value, sev = filters.severity.value;
    const rows = data.findings
      .filter((f) => !st || f.status === st || (st === "active" && (f.status === "open" || f.status === "remediated")))
      .filter((f) => !sev || f.severity === sev)
      .sort((a, b) => (b.overdue_days || 0) - (a.overdue_days || 0) || a.due_date.localeCompare(b.due_date));
    tbody.replaceChildren(rows.length ? findingList(rows) : el("p", { class: "empty" }, "No finding matches this filter."));
  };
  filters.status.addEventListener("change", fill);
  filters.severity.addEventListener("change", fill);
  fill();
  const download = el("button", {
    class: "btn", id: "report-md",
    onclick: async () => {
      const res = await fetch("/api/findings/report?format=md", { headers: { Authorization: `Bearer ${session.token}` } });
      const url = URL.createObjectURL(new Blob([await res.text()], { type: "text/markdown" }));
      const a = el("a", { href: url, download: `findings-${report.as_of}.md` });
      a.click();
      URL.revokeObjectURL(url);
    },
  }, "Governance report (Markdown)");
  const tile = (label, value, cls) => el("div", { class: `tile ${cls || ""}` }, el("div", { class: "value" }, String(value)), el("div", { class: "label" }, label));
  if (location.hash !== at) return; // the user has moved on: do not render over the new page
  show("findings",
    el("div", { class: "toolbar" }, el("div", {}, el("h1", {}, "Findings"), el("p", { class: "lead" }, "Raised by validators, remediated by model owners with a reproducible run as evidence, closed by validators.")), download),
    el("div", { class: "tiles", id: "finding-tiles" },
      tile("Active", report.active.length),
      tile("Overdue", report.overdue, report.overdue ? "fail" : ""),
      tile("Awaiting review", report.awaiting_review, report.awaiting_review ? "warn" : ""),
      tile("Closed, last 90 days", report.closed_last_90_days)),
    el("div", { class: "actions" }, filters.status, filters.severity),
    tbody,
  );
}

async function viewFinding(id) {
  const at = location.hash;
  const f = await api(`/api/findings/${enc(id)}`);
  const msg = el("div");
  const comment = el("textarea", { id: "fa-comment", placeholder: "Comment (required)" });
  const due = el("input", { type: "date", id: "fa-due" });
  const evidence = el("input", { type: "text", id: "fa-evidence", placeholder: "pipeline#run, e.g. desk-var#12" });
  const send = async (action) => {
    try {
      await api(`/api/findings/${enc(id)}/actions`, { method: "POST", json: { action, comment: comment.value, due_date: due.value || null, evidence: evidence.value.trim() || null } });
      route();
    } catch (err) {
      msg.replaceChildren(el("div", { class: "notice fail", role: "alert" }, err.message));
    }
  };
  const btn = (action, label, cls = "btn") => el("button", { class: cls, id: `fa-${action}`, onclick: () => send(action) }, label);
  const owner = session.me.role === "admin" || (session.me.role === "model_owner" && f.owner === session.me.name);
  const buttons = [];
  const extra = [];
  if (owner && f.status === "open") {
    buttons.push(btn("plan", "Save as remediation plan"), btn("remediate", "Submit the remediation", "btn primary"));
    extra.push(el("div", { class: "field" }, el("label", { for: "fa-evidence" }, "Evidence run (optional)"), evidence,
      el("span", { class: "muted" }, "A run of a pipeline of this model. It must replay identically.")));
  }
  if (can("validator") && f.status === "remediated") buttons.push(btn("close", "Close the finding", "btn approve"), btn("reopen", "Reopen", "btn reject"));
  if (can("validator") && (f.status === "open" || f.status === "remediated")) {
    buttons.push(btn("extend", "Extend the deadline"), btn("accept", "Accept the risk"));
    extra.push(el("div", { class: "field" }, el("label", { for: "fa-due" }, "New deadline (to extend)"), due));
  }
  if (!can("auditor")) buttons.push(btn("comment", "Comment"));
  const runLink = f.pipeline && (f.run_id ? el("a", { href: `#/p/${enc(f.pipeline)}/r/${f.run_id}` }, `${f.pipeline} #${f.run_id}`) : el("a", { href: `#/p/${enc(f.pipeline)}` }, f.pipeline));
  const evLink = f.evidence && (() => { const [p, r] = f.evidence.split("#"); return el("a", { href: `#/p/${enc(p)}/r/${r}` }, f.evidence); })();
  if (location.hash !== at) return; // the user has moved on: do not render over the new page
  show("findings",
    el("div", { class: "crumbs" }, el("a", { href: "#/findings" }, "Findings"), " / ", el("a", { href: `#/models/${enc(f.model)}` }, f.model)),
    el("div", { class: "toolbar" }, el("div", {}, el("h1", {}, `Finding #${f.id}: ${f.title}`), el("p", { class: "lead" }, f.description))),
    el("dl", { class: "facts" },
      el("div", {}, el("dt", {}, "Status"), el("dd", { id: "finding-status" }, findingState(f))),
      el("div", {}, el("dt", {}, "Severity"), el("dd", {}, severity(f.severity))),
      el("div", {}, el("dt", {}, "Remediation owner"), el("dd", {}, f.owner)),
      el("div", {}, el("dt", {}, "Deadline"), el("dd", {}, f.due_date)),
      el("div", {}, el("dt", {}, "Raised by"), el("dd", {}, `${f.raised_by}, ${when(f.created_at)}`)),
      runLink && el("div", {}, el("dt", {}, "Raised on"), el("dd", {}, runLink, f.step ? ` · step ${f.step}` : "")),
      evLink && el("div", {}, el("dt", {}, "Evidence"), el("dd", {}, evLink)),
      f.blocks_approval && el("div", {}, el("dt", {}, "Approval"), el("dd", {}, el("span", { class: "state fail" }, "Blocks the model's approval"))),
    ),
    f.remediation && el("div", { class: "panel" }, el("h2", {}, "Remediation plan"), el("p", {}, f.remediation)),
    el("div", { class: "columns" },
      el("section", {}, el("h2", {}, "History"),
        el("ul", { class: "timeline", id: "finding-events" }, f.events.map((e) => el("li", { class: e.action === "close" ? "approved" : e.action === "reopen" || e.action === "escalate" ? "rejected" : "" },
          el("div", {}, el("strong", {}, e.action), " · ", e.user),
          el("div", { class: "meta" }, `${when(e.ts)} · ${(FINDING_STATES[e.status] || [e.status])[0]}`),
          e.comment && el("p", {}, e.comment))))),
      buttons.length > 0 && el("aside", {}, el("div", { class: "panel" }, el("h2", {}, "Act"),
        el("div", { class: "field" }, el("label", { for: "fa-comment" }, "Comment"), comment),
        extra,
        el("div", { class: "actions", id: "finding-actions" }, buttons),
        msg)),
    ),
  );
}

// ---------- Audit ----------
const OUTCOMES = { ok: "Succeeded", denied: "Denied (permissions)", approved: "Approved", rejected: "Rejected", reproducible: "Reproducible", mismatch: "Replay mismatch" };

function outcomeClass(o) {
  if (/^(denied|refused|error|mismatch|rejected)/.test(o)) return "fail";
  if (o === "approved" || o === "reproducible") return "pass";
  return "none";
}

async function viewAudit() {
  const at = location.hash;
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
    if (!rows.length) tbody.replaceChildren(el("tr", {}, el("td", { colspan: 6, class: "empty" }, "No entry matches this filter.")));
  };
  const search = el("input", { type: "search", id: "filter", placeholder: "Filter: user, action, target…", "aria-label": "Filter the log", oninput: (e) => fill(e.target.value) });
  fill("");
  if (location.hash !== at) return; // the user has moved on: do not render over the new page
  show("audit",
    el("div", { class: "toolbar" }, el("div", {}, el("h1", {}, "Audit log"), el("p", { class: "lead" }, "Every action, including denials. The log is append-only: the database rejects any modification.")), search),
    el("div", { class: "table-wrap" }, el("table", {}, el("thead", {}, el("tr", {}, ["Date", "User", "Role", "Action", "Target", "Outcome"].map((h) => el("th", { scope: "col" }, h)))), tbody)),
  );
}

// ---------- Routing ----------
async function route() {
  if (!session.token) return renderLogin();
  const parts = location.hash.replace(/^#\/?/, "").split("/").map(decodeURIComponent);
  try {
    if (parts[0] === "p" && parts[2] === "r") await viewRun(parts[1], parts[3]);
    else if (parts[0] === "p" && parts[1]) await viewPipeline(parts[1]);
    else if (parts[0] === "audit") await viewAudit();
    else if (parts[0] === "models" && parts[1]) await viewModel(parts[1]);
    else if (parts[0] === "models") await viewModels();
    else if (parts[0] === "findings" && parts[1]) await viewFinding(parts[1]);
    else if (parts[0] === "findings") await viewFindings();
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
