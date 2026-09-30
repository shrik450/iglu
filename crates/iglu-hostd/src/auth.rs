//! Verifying the IdP-issued access tokens iglud presents.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iglu_domain::auth::{Issuer, Subject};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing bearer token")]
    Missing,
    #[error("invalid token: {0}")]
    Invalid(String),
    #[error("token subject isn't an allowed service")]
    Subject,
    #[error("couldn't fetch the IdP's keys: {0}")]
    Keys(String),
}

/// Refetch keys at most this often when a token names an unknown key.
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);

pub struct Verifier {
    issuer: Issuer,
    audience: String,
    subjects: Vec<Subject>,
    http: reqwest::Client,
    keys: RwLock<Keys>,
}

struct Keys {
    set: JwkSet,
    fetched: Option<Instant>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
}

impl Verifier {
    pub fn new(
        issuer: Issuer,
        audience: String,
        subjects: Vec<Subject>,
        http: reqwest::Client,
    ) -> Arc<Self> {
        Arc::new(Self {
            issuer,
            audience,
            subjects,
            http,
            keys: RwLock::new(Keys {
                set: JwkSet { keys: Vec::new() },
                fetched: None,
            }),
        })
    }

    async fn fetch_keys(&self) -> Result<JwkSet, AuthError> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.issuer.as_str().trim_end_matches('/')
        );
        let discovery: Discovery = self
            .http
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| AuthError::Keys(e.to_string()))?
            .json()
            .await
            .map_err(|e| AuthError::Keys(e.to_string()))?;
        if discovery.issuer != self.issuer.as_str() {
            return Err(AuthError::Keys("discovery names a different issuer".into()));
        }
        self.http
            .get(&discovery.jwks_uri)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| AuthError::Keys(e.to_string()))?
            .json()
            .await
            .map_err(|e| AuthError::Keys(e.to_string()))
    }

    async fn key(&self, kid: Option<&str>) -> Result<DecodingKey, AuthError> {
        let find = |set: &JwkSet| -> Option<DecodingKey> {
            let jwk = match kid {
                Some(kid) => set.find(kid),
                None => set.keys.first(),
            }?;
            DecodingKey::from_jwk(jwk).ok()
        };
        {
            let keys = self.keys.read().await;
            if let Some(key) = find(&keys.set) {
                return Ok(key);
            }
            if keys
                .fetched
                .is_some_and(|at| at.elapsed() < REFRESH_INTERVAL)
            {
                return Err(AuthError::Invalid("unknown signing key".into()));
            }
        }
        let set = self.fetch_keys().await?;
        let mut keys = self.keys.write().await;
        *keys = Keys {
            set,
            fetched: Some(Instant::now()),
        };
        find(&keys.set).ok_or_else(|| AuthError::Invalid("unknown signing key".into()))
    }

    pub async fn verify(&self, header: Option<&str>) -> Result<(), AuthError> {
        let token = header
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or(AuthError::Missing)?;
        let jwt_header =
            jsonwebtoken::decode_header(token).map_err(|e| AuthError::Invalid(e.to_string()))?;
        // Asymmetric algorithms only: a shared-secret algorithm would let
        // anyone holding the public key forge tokens.
        let asymmetric = matches!(
            jwt_header.alg,
            Algorithm::RS256
                | Algorithm::RS384
                | Algorithm::RS512
                | Algorithm::PS256
                | Algorithm::PS384
                | Algorithm::PS512
                | Algorithm::ES256
                | Algorithm::ES384
                | Algorithm::EdDSA
        );
        if !asymmetric {
            return Err(AuthError::Invalid("unsupported algorithm".into()));
        }
        let algorithm = jwt_header.alg;
        let key = self.key(jwt_header.kid.as_deref()).await?;
        let mut validation = Validation::new(algorithm);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[self.audience.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = 30;
        let data = jsonwebtoken::decode::<Claims>(token, &key, &validation)
            .map_err(|e| AuthError::Invalid(e.to_string()))?;
        if self
            .subjects
            .iter()
            .any(|allowed| allowed.as_str() == data.claims.sub)
        {
            Ok(())
        } else {
            Err(AuthError::Subject)
        }
    }
}
