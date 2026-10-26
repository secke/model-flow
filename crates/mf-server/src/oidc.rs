//! Authentification unique (SSO) OpenID Connect : validation des jetons d'accès JWT émis par le
//! fournisseur d'identité de la banque (Entra ID, ADFS, Okta, Keycloak…).
//!
//! - Signature vérifiée avec les clés publiques (JWKS) obtenues par découverte
//!   (`<issuer>/.well-known/openid-configuration`), par URL ou depuis un fichier (environnement isolé).
//! - Rotation des clés : un `kid` inconnu déclenche un rechargement (au plus tous les `jwks_refresh_secs`).
//! - Contrôles : algorithme autorisé (jamais `none` ni HMAC), émetteur, audience, expiration, `nbf`.
//! - Rôles : les groupes ou rôles du jeton sont convertis par `role_mapping`. Un utilisateur à la fois
//!   validateur et propriétaire de modèle (ou admin) est refusé : séparation des tâches.

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
    /// URL du JWKS ; par défaut, obtenue par découverte depuis `issuer`.
    pub jwks_url: Option<String>,
    /// JWKS local (déploiement sans accès réseau au fournisseur d'identité).
    pub jwks_file: Option<PathBuf>,
    #[serde(default = "default_user_claim")]
    pub username_claim: String,
    /// Revendication portant les groupes/rôles ; chemin pointé accepté (`realm_access.roles`).
    #[serde(default = "default_roles_claim")]
    pub roles_claim: String,
    pub role_mapping: BTreeMap<String, Role>,
    #[serde(default = "default_algorithms")]
    pub algorithms: Vec<Algorithm>,
    #[serde(default = "default_leeway")]
    pub leeway_secs: u64,
    #[serde(default = "default_refresh")]
    pub jwks_refresh_secs: u64,
}

/// Échec d'authentification : `forbidden` distingue un jeton valide sans droits (403) d'un jeton invalide (401).
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
}

/// Valeur d'une revendication par chemin pointé.
pub fn claim<'a>(claims: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(claims, |v, k| v.get(k))
}

/// Rôle ModelFlow retenu à partir des rôles correspondants, ou refus.
pub fn resolve_role(mapped: &[Role]) -> Result<Role, OidcError> {
    let has = |r| mapped.contains(&r);
    if mapped.is_empty() {
        return Err(OidcError {
            forbidden: true,
            message: "aucun rôle ModelFlow dans le jeton".into(),
        });
    }
    if has(Role::Validator) && (has(Role::ModelOwner) || has(Role::Admin)) {
        return Err(OidcError {
            forbidden: true,
            message: "rôles incompatibles : validateur et propriétaire de modèle/admin (séparation des tâches)".into(),
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
            return Err("[oidc] : role_mapping ne peut pas être vide".into());
        }
        if cfg
            .algorithms
            .iter()
            .any(|a| matches!(a, Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512))
        {
            return Err("[oidc] : les algorithmes HMAC (HS*) sont refusés".into());
        }
        let o = Self {
            cfg,
            cache: RwLock::new(KeyCache::default()),
        };
        if o.cfg.jwks_file.is_some() {
            o.refresh()?; // un fichier invalide doit empêcher le démarrage
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
                    .map_err(|e| format!("découverte OIDC {disco}: {e}"))?
                    .into_string()
                    .map_err(|e| format!("découverte OIDC : {e}"))
                    .and_then(|t| serde_json::from_str(&t).map_err(|e| format!("découverte OIDC : {e}")))?;
                doc.get("jwks_uri")
                    .and_then(Value::as_str)
                    .ok_or("découverte OIDC : `jwks_uri` absent")?
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

    fn refresh(&self) -> Result<(), String> {
        let set: JwkSet = serde_json::from_str(&self.fetch_jwks()?).map_err(|e| format!("JWKS invalide : {e}"))?;
        let mut keys = HashMap::new();
        for (i, k) in set.keys.iter().enumerate() {
            let kid = k.common.key_id.clone().unwrap_or_else(|| format!("#{i}"));
            if let Ok(dk) = DecodingKey::from_jwk(k) {
                keys.insert(kid, dk);
            }
        }
        if keys.is_empty() {
            return Err("JWKS sans clé exploitable".into());
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

    /// Vérifie un jeton (appel bloquant : peut recharger le JWKS) et renvoie (utilisateur, rôle).
    pub fn verify(&self, token: &str) -> Result<(String, Role), OidcError> {
        let header = decode_header(token).map_err(|e| invalid(format!("jeton illisible : {e}")))?;
        if !self.cfg.algorithms.contains(&header.alg) {
            return Err(invalid(format!("algorithme {:?} non autorisé", header.alg)));
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
                self.key(&header.kid)
                    .ok_or_else(|| invalid("clé de signature inconnue"))?
            }
        };
        let mut v = Validation::new(header.alg);
        v.set_issuer(&[&self.cfg.issuer]);
        v.set_audience(&[&self.cfg.audience]);
        v.leeway = self.cfg.leeway_secs;
        v.validate_nbf = true;
        v.set_required_spec_claims(&["exp", "iss", "aud"]);
        let data = decode::<Value>(token, &key, &v).map_err(|e| invalid(format!("jeton refusé : {e}")))?;
        let name = claim(&data.claims, &self.cfg.username_claim)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid(format!("revendication `{}` absente", self.cfg.username_claim)))?
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
            .contains("séparation"));
        assert!(resolve_role(&[]).unwrap_err().forbidden);
    }
}
