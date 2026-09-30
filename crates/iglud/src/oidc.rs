//! OpenID Connect sign-in, through the `openidconnect` relying-party library.
//! iglu never sees passwords; the provider hosts login and MFA.

use iglu_domain::auth::{Issuer, VerifiedIdentity};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("OIDC discovery failed: {0}")]
    Discovery(String),
    #[error("OIDC configuration: {0}")]
    Config(String),
    #[error("the sign-in couldn't be completed: {0}")]
    Exchange(String),
    #[error("the identity token was invalid: {0}")]
    Token(String),
    #[error("the provider returned a claim iglu can't use: {0}")]
    Claims(String),
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct HttpError(String);

/// Adapts the shared reqwest client to the library's HTTP interface. Redirects
/// are never followed, as the library requires to avoid SSRF.
async fn send(
    http: reqwest::Client,
    request: openidconnect::HttpRequest,
) -> Result<openidconnect::HttpResponse, HttpError> {
    let (parts, body) = request.into_parts();
    let response = http
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(|e| HttpError(e.to_string()))?;
    let mut builder = http::Response::builder().status(response.status());
    for (name, value) in response.headers() {
        builder = builder.header(name, value);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| HttpError(e.to_string()))?;
    builder
        .body(bytes.to_vec())
        .map_err(|e| HttpError(e.to_string()))
}

/// One relying party: the console or the preview gateway.
pub struct RelyingParty {
    http: reqwest::Client,
    metadata: CoreProviderMetadata,
    client_id: ClientId,
    client_secret: ClientSecret,
    redirect: RedirectUrl,
}

/// What must be remembered between redirecting to the provider and the callback.
pub struct Pending {
    pub url: Url,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

impl RelyingParty {
    pub async fn discover(
        http: reqwest::Client,
        issuer: &Issuer,
        client_id: String,
        client_secret: String,
        redirect: Url,
    ) -> Result<Self, OidcError> {
        let issuer =
            IssuerUrl::new(issuer.to_string()).map_err(|e| OidcError::Config(e.to_string()))?;
        let fetch = http.clone();
        let metadata = CoreProviderMetadata::discover_async(issuer, &move |request| {
            send(fetch.clone(), request)
        })
        .await
        .map_err(|e| OidcError::Discovery(e.to_string()))?;
        Ok(Self {
            http,
            metadata,
            client_id: ClientId::new(client_id),
            client_secret: ClientSecret::new(client_secret),
            redirect: RedirectUrl::from_url(redirect),
        })
    }

    pub fn token_endpoint(&self) -> Option<&Url> {
        self.metadata
            .token_endpoint()
            .map(|endpoint| endpoint.url())
    }

    pub fn begin(&self) -> Pending {
        let client = CoreClient::from_provider_metadata(
            self.metadata.clone(),
            self.client_id.clone(),
            Some(self.client_secret.clone()),
        )
        .set_redirect_uri(self.redirect.clone());
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("email".into()))
            .add_scope(Scope::new("profile".into()))
            .set_pkce_challenge(challenge)
            .url();
        Pending {
            url,
            state: state.secret().clone(),
            nonce: nonce.secret().clone(),
            pkce_verifier: verifier.secret().clone(),
        }
    }

    pub async fn finish(
        &self,
        code: String,
        nonce: String,
        pkce_verifier: String,
    ) -> Result<VerifiedIdentity, OidcError> {
        let client = CoreClient::from_provider_metadata(
            self.metadata.clone(),
            self.client_id.clone(),
            Some(self.client_secret.clone()),
        )
        .set_redirect_uri(self.redirect.clone());
        let http = self.http.clone();
        let response = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|e| OidcError::Config(e.to_string()))?
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
            .request_async(&move |request| send(http.clone(), request))
            .await
            .map_err(|e| OidcError::Exchange(e.to_string()))?;
        let id_token = response
            .id_token()
            .ok_or_else(|| OidcError::Token("no ID token".into()))?;
        let claims = id_token
            .claims(&client.id_token_verifier(), &Nonce::new(nonce))
            .map_err(|e| OidcError::Token(e.to_string()))?;

        fn parse<T: std::str::FromStr<Err = iglu_domain::ParseError>>(
            what: &'static str,
            value: &str,
        ) -> Result<T, OidcError> {
            value
                .parse()
                .map_err(|e: iglu_domain::ParseError| OidcError::Claims(format!("{what}: {e}")))
        }
        let issuer: Issuer = parse("issuer", claims.issuer().as_str())?;
        let subject = parse("subject", claims.subject().as_str())?;
        let email = match (claims.email(), claims.email_verified()) {
            (Some(email), Some(true)) => Some(parse("email", email.as_str())?),
            (Some(_) | None, _) => None,
        };
        let name = claims
            .preferred_username()
            .map(|n| n.as_str().to_owned())
            .or_else(|| {
                claims
                    .name()
                    .and_then(|n| n.get(None))
                    .map(|n| n.as_str().to_owned())
            })
            .and_then(|n| n.parse().ok());
        Ok(VerifiedIdentity {
            issuer,
            subject,
            email,
            name,
        })
    }
}

/// Fetches and caches the service token iglud presents to hosts, through
/// the OAuth client-credentials grant.
pub struct WorkerTokens {
    http: reqwest::Client,
    endpoint: Url,
    client_id: String,
    client_secret: String,
    audience: String,
    cached: tokio::sync::Mutex<Option<(String, std::time::Instant)>>,
}

#[derive(serde::Deserialize)]
struct TokenReply {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

impl WorkerTokens {
    pub fn new(
        http: reqwest::Client,
        endpoint: Url,
        client_id: String,
        client_secret: String,
        audience: String,
    ) -> Self {
        Self {
            http,
            endpoint,
            client_id,
            client_secret,
            audience,
            cached: tokio::sync::Mutex::new(None),
        }
    }

    pub async fn token(&self) -> Result<String, OidcError> {
        let mut cached = self.cached.lock().await;
        if let Some((token, valid_until)) = cached.as_ref()
            && std::time::Instant::now() < *valid_until
        {
            return Ok(token.clone());
        }
        let reply: TokenReply = self
            .http
            .post(self.endpoint.clone())
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[
                ("grant_type", "client_credentials"),
                ("audience", self.audience.as_str()),
            ])
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| OidcError::Exchange(e.to_string()))?
            .json()
            .await
            .map_err(|e| OidcError::Exchange(e.to_string()))?;
        let lifetime = reply.expires_in.unwrap_or(300).saturating_sub(60).max(30);
        let valid_until = std::time::Instant::now() + std::time::Duration::from_secs(lifetime);
        *cached = Some((reply.access_token.clone(), valid_until));
        Ok(reply.access_token)
    }
}
