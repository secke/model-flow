//! OpenID Connect single sign-on (SSO): validation of JWT access tokens issued by the
//! bank's identity provider (Entra ID, ADFS, Okta, Keycloak…).
//!
//! - Signature verified with the public keys (JWKS) obtained by discovery
//!   (`<issuer>/.well-known/openid-configuration`), by URL or from a file (air-gapped environment).
//! - Key rotation: an unknown `kid` triggers a reload (at most every `jwks_refresh_secs`).
//! - Checks: allowed algorithm (never `none` or HMAC), issuer, audience, expiry, `nbf`.
//! - Roles: the token's groups or roles are mapped by `role_mapping`. A user who is both
//!   validator and model owner (or admin) is rejected: segregation of duties.

use crate::Role;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{Duration, Instant};

fn default_user_claim() -> String {
    "preferred_username".into()
}
fn default_roles_claim() -> String {
    "roles".into()
}
fn default_algorithms() -> Vec<Algorithm> {
    vec![Algorithm::RS256, Algorithm::PS256, Algorithm::ES256]
}
fn default_leeway() -> u64 {
    60
}
fn default_refresh() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize)]
pub struct OidcConfig {
    pub issuer: String,
    pub audience: String,
    /// JWKS URL; by default, obtained by discovery from `issuer`.
    pub jwks_url: Option<String>,
    /// Local JWKS (deployment without network access to the identity provider).
    pub jwks_file: Option<PathBuf>,
    #[serde(default = "default_user_claim")]
    pub username_claim: String,
    /// Claim carrying the groups/roles; dotted path accepted (`realm_access.roles`).
    #[serde(default = "default_roles_claim")]
    pub roles_claim: String,
    pub role_mapping: BTreeMap<String, Role>,
    #[serde(default = "default_algorithms")]
    pub algorithms: Vec<Algorithm>,
    #[serde(default = "default_leeway")]
    pub leeway_secs: u64,
    #[serde(default = "default_refresh")]
    pub jwks_refresh_secs: u64,
    /// Web application identifier (public client, "authorization code" flow + PKCE).
    pub client_id: Option<String>,
    /// Scopes requested by the web UI (e.g. `openid profile api://modelflow/.default`).
    pub scopes: Option<String>,
    /// Browser endpoints; by default, obtained by discovery.
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
}

/// Authentication failure: `forbidden` distinguishes a valid token without rights (403) from an invalid token (401).
#[derive(Debug)]
pub struct OidcError {
    pub forbidden: bool,
    pub message: String,
}

fn invalid(m: impl Into<String>) -> OidcError {
    OidcError {
        forbidden: false,
        message: m.into(),
    }
}

#[derive(Default)]
struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    fetched: Option<Instant>,
}

pub struct Oidc {
    cfg: OidcConfig,
    cache: RwLock<KeyCache>,
    browser: RwLock<Option<(String, String)>>,
}

/// Origin (`scheme://host[:port]`) of a URL.
pub fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split('/').next()?;
    (!host.is_empty()).then(|| format!("{scheme}://{host}"))
}

/// Value of a claim by dotted path.
pub fn claim<'a>(claims: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(claims, |v, k| v.get(k))
}

/// ModelFlow role selected from the matching roles, or denial.
pub fn resolve_role(mapped: &[Role]) -> Result<Role, OidcError> {
    let has = |r| mapped.contains(&r);
    if mapped.is_empty() {
        return Err(OidcError {
            forbidden: true,
            message: "no ModelFlow role in the token".into(),
        });
    }
    if has(Role::Validator) && (has(Role::ModelOwner) || has(Role::Admin)) {
        return Err(OidcError {
            forbidden: true,
            message: "incompatible roles: validator and model owner/admin (segregation of duties)".into(),
        });
    }
    Ok([Role::Admin, Role::ModelOwner, Role::Validator, Role::Auditor]
        .into_iter()
        .find(|r| has(*r))
        .unwrap())
}

impl Oidc {
    pub fn new(cfg: OidcConfig) -> Result<Self, String> {
        if cfg.role_mapping.is_empty() {
            return Err("[oidc]: role_mapping cannot be empty".into());
        }
        if cfg
            .algorithms
            .iter()
            .any(|a| matches!(a, Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512))
        {
            return Err("[oidc]: HMAC algorithms (HS*) are not allowed".into());
        }
        let o = Self {
            cfg,
            cache: RwLock::new(KeyCache::default()),
            browser: RwLock::new(None),
        };
        if o.cfg.jwks_file.is_some() {
            o.refresh()?; // an invalid file must prevent startup
        }
        Ok(o)
    }

    fn fetch_jwks(&self) -> Result<String, String> {
        if let Some(f) = &self.cfg.jwks_file {
            return std::fs::read_to_string(f).map_err(|e| format!("JWKS {}: {e}", f.display()));
        }
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
        let url = match &self.cfg.jwks_url {
            Some(u) => u.clone(),
            None => {
                let disco = format!(
                    "{}/.well-known/openid-configuration",
                    self.cfg.issuer.trim_end_matches('/')
                );
                let doc: Value = agent
                    .get(&disco)
                    .call()
                    .map_err(|e| format!("OIDC discovery {disco}: {e}"))?
                    .into_string()
                    .map_err(|e| format!("OIDC discovery: {e}"))
                    .and_then(|t| serde_json::from_str(&t).map_err(|e| format!("OIDC discovery: {e}")))?;
                doc.get("jwks_uri")
                    .and_then(Value::as_str)
                    .ok_or("OIDC discovery: `jwks_uri` missing")?
                    .to_string()
            }
        };
        agent
            .get(&url)
            .call()
            .map_err(|e| format!("JWKS {url}: {e}"))?
            .into_string()
            .map_err(|e| e.to_string())
    }

    pub fn config(&self) -> &OidcConfig {
        &self.cfg
    }

    /// Origins to allow in the browser security policy (issuer, endpoints).
    pub fn browser_origins(&self) -> Vec<String> {
        let mut v: Vec<String> = [
            Some(&self.cfg.issuer),
            self.cfg.authorization_endpoint.as_ref(),
            self.cfg.token_endpoint.as_ref(),
        ]
        .into_iter()
        .flatten()
        .filter_map(|u| origin(u))
        .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Authorization and token endpoints for the browser (may block).
    pub fn browser_endpoints(&self) -> Result<(String, String), String> {
        if let (Some(a), Some(t)) = (&self.cfg.authorization_endpoint, &self.cfg.token_endpoint) {
            return Ok((a.clone(), t.clone()));
        }
        if let Some(e) = self.browser.read().unwrap().clone() {
            return Ok(e);
        }
        let disco = format!(
            "{}/.well-known/openid-configuration",
            self.cfg.issuer.trim_end_matches('/')
        );
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
        let doc: Value = agent
            .get(&disco)
            .call()
            .map_err(|e| format!("OIDC discovery {disco}: {e}"))?
            .into_string()
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))?;
        let get = |k: &str| {
            doc.get(k)
                .and_then(Value::as_str)
                .map(String::from)
                .ok_or(format!("OIDC discovery: `{k}` missing"))
        };
        let e = (
            self.cfg
                .authorization_endpoint
                .clone()
                .map_or_else(|| get("authorization_endpoint"), Ok)?,
            self.cfg
                .token_endpoint
                .clone()
                .map_or_else(|| get("token_endpoint"), Ok)?,
        );
        *self.browser.write().unwrap() = Some(e.clone());
        Ok(e)
    }

    fn refresh(&self) -> Result<(), String> {
        let set: JwkSet = serde_json::from_str(&self.fetch_jwks()?).map_err(|e| format!("invalid JWKS: {e}"))?;
        let mut keys = HashMap::new();
        for (i, k) in set.keys.iter().enumerate() {
            let kid = k.common.key_id.clone().unwrap_or_else(|| format!("#{i}"));
            if let Ok(dk) = DecodingKey::from_jwk(k) {
                keys.insert(kid, dk);
            }
        }
        if keys.is_empty() {
            return Err("JWKS has no usable key".into());
        }
        let mut c = self.cache.write().unwrap();
        c.keys = keys;
        c.fetched = Some(Instant::now());
        Ok(())
    }

    fn key(&self, kid: &Option<String>) -> Option<DecodingKey> {
        let c = self.cache.read().unwrap();
        match kid {
            Some(k) => c.keys.get(k).cloned(),
            None if c.keys.len() == 1 => c.keys.values().next().cloned(),
            None => None,
        }
    }

    /// Verifies a token (blocking call: may reload the JWKS) and returns (user, role).
    pub fn verify(&self, token: &str) -> Result<(String, Role), OidcError> {
        let header = decode_header(token).map_err(|e| invalid(format!("unreadable token: {e}")))?;
        if !self.cfg.algorithms.contains(&header.alg) {
            return Err(invalid(format!("algorithm {:?} not allowed", header.alg)));
        }
        let key = match self.key(&header.kid) {
            Some(k) => k,
            None => {
                let stale = self
                    .cache
                    .read()
                    .unwrap()
                    .fetched
                    .is_none_or(|t| t.elapsed() >= Duration::from_secs(self.cfg.jwks_refresh_secs));
                if stale {
                    self.refresh().map_err(invalid)?;
                }
                self.key(&header.kid).ok_or_else(|| invalid("unknown signing key"))?
            }
        };
        let mut v = Validation::new(header.alg);
        v.set_issuer(&[&self.cfg.issuer]);
        v.set_audience(&[&self.cfg.audience]);
        v.leeway = self.cfg.leeway_secs;
        v.validate_nbf = true;
        v.set_required_spec_claims(&["exp", "iss", "aud"]);
        let data = decode::<Value>(token, &key, &v).map_err(|e| invalid(format!("token rejected: {e}")))?;
        let name = claim(&data.claims, &self.cfg.username_claim)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid(format!("claim `{}` missing", self.cfg.username_claim)))?
            .to_string();
        let groups: Vec<String> = match claim(&data.claims, &self.cfg.roles_claim) {
            Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
            Some(Value::String(s)) => s.split_whitespace().map(String::from).collect(),
            _ => vec![],
        };
        let mut mapped: Vec<Role> = groups
            .iter()
            .filter_map(|g| self.cfg.role_mapping.get(g).copied())
            .collect();
        mapped.dedup();
        Ok((name, resolve_role(&mapped)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_claims() {
        let c = json!({"realm_access": {"roles": ["a", "b"]}});
        assert_eq!(claim(&c, "realm_access.roles"), Some(&json!(["a", "b"])));
        assert_eq!(claim(&c, "absent.x"), None);
    }

    #[test]
    fn role_resolution() {
        use Role::*;
        assert_eq!(resolve_role(&[Auditor, ModelOwner]).unwrap(), ModelOwner);
        assert_eq!(resolve_role(&[Validator, Auditor]).unwrap(), Validator);
        assert!(resolve_role(&[Validator, ModelOwner]).unwrap_err().forbidden);
        assert!(resolve_role(&[Validator, Admin])
            .unwrap_err()
            .message
            .contains("segregation"));
        assert!(resolve_role(&[]).unwrap_err().forbidden);
    }
}
