// Simulated OIDC identity provider for tests: discovery, user picker page,
// code exchange with PKCE verification (S256), JWKS. RSA key generated at startup.
"use strict";
const http = require("http");
const crypto = require("crypto");

const USERS = {
  dana: { groups: ["MF-Owners"], label: "Dana, model owner" },
  eli: { groups: ["MF-Validators"], label: "Eli, validator" },
  carol: { groups: ["MF-Auditors"], label: "Carol, auditor" },
  fred: { groups: ["MF-Owners", "MF-Validators"], label: "Fred, incompatible roles" },
};

function b64url(buf) {
  return Buffer.from(buf).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function start({ port = 0, audience, clientId }) {
  const { privateKey, publicKey } = crypto.generateKeyPairSync("rsa", { modulusLength: 2048 });
  const jwk = { ...publicKey.export({ format: "jwk" }), kid: "mock-1", alg: "RS256", use: "sig" };
  const codes = new Map();
  let base = "";

  const sign = (claims) => {
    const head = b64url(JSON.stringify({ alg: "RS256", typ: "JWT", kid: "mock-1" }));
    const body = b64url(JSON.stringify(claims));
    const sig = crypto.sign("sha256", Buffer.from(`${head}.${body}`), privateKey);
    return `${head}.${body}.${b64url(sig)}`;
  };

  const server = http.createServer((req, res) => {
    const url = new URL(req.url, base);
    const cors = { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Headers": "Content-Type" };
    const json = (code, obj) => {
      res.writeHead(code, { "Content-Type": "application/json", ...cors });
      res.end(JSON.stringify(obj));
    };
    if (req.method === "OPTIONS") {
      res.writeHead(204, { ...cors, "Access-Control-Allow-Methods": "POST, GET" });
      return res.end();
    }
    if (url.pathname === "/.well-known/openid-configuration") {
      return json(200, {
        issuer: base,
        authorization_endpoint: `${base}/authorize`,
        token_endpoint: `${base}/token`,
        jwks_uri: `${base}/jwks`,
      });
    }
    if (url.pathname === "/jwks") return json(200, { keys: [jwk] });
    if (url.pathname === "/authorize") {
      const p = url.searchParams;
      if (p.get("client_id") !== clientId || p.get("code_challenge_method") !== "S256") {
        res.writeHead(400);
        return res.end("invalid authorization request");
      }
      const links = Object.entries(USERS)
        .map(([u, v]) => {
          const q = new URLSearchParams(p);
          q.set("user", u);
          return `<li><a id="user-${u}" href="/pick?${q}">${v.label}</a></li>`;
        })
        .join("");
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      return res.end(`<!doctype html><meta charset="utf-8"><title>Test IdP</title><h1>Pick a user</h1><ul>${links}</ul>`);
    }
    if (url.pathname === "/pick") {
      const p = url.searchParams;
      const code = crypto.randomBytes(16).toString("hex");
      codes.set(code, { user: p.get("user"), challenge: p.get("code_challenge"), redirect: p.get("redirect_uri") });
      const back = new URL(p.get("redirect_uri"));
      back.searchParams.set("code", code);
      back.searchParams.set("state", p.get("state"));
      res.writeHead(302, { Location: back.toString() });
      return res.end();
    }
    if (url.pathname === "/token" && req.method === "POST") {
      let raw = "";
      req.on("data", (c) => (raw += c));
      req.on("end", () => {
        const f = new URLSearchParams(raw);
        const c = codes.get(f.get("code"));
        codes.delete(f.get("code"));
        if (!c || f.get("client_id") !== clientId || f.get("redirect_uri") !== c.redirect) {
          return json(400, { error: "invalid_grant" });
        }
        const expected = b64url(crypto.createHash("sha256").update(f.get("code_verifier") || "").digest());
        if (expected !== c.challenge) return json(400, { error: "invalid_grant", error_description: "PKCE" });
        const now = Math.floor(Date.now() / 1000);
        const u = USERS[c.user];
        const token = sign({ iss: base, aud: audience, sub: `id-${c.user}`, preferred_username: c.user, groups: u.groups, iat: now, exp: now + 900 });
        json(200, { access_token: token, token_type: "Bearer", expires_in: 900 });
      });
      return;
    }
    res.writeHead(404);
    res.end();
  });

  return new Promise((resolve) =>
    server.listen(port, "127.0.0.1", () => {
      base = `http://127.0.0.1:${server.address().port}`;
      resolve({ base, close: () => server.close() });
    }),
  );
}

module.exports = { start, USERS };
