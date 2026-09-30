//! `OpenID` Connect sign-in, through the `openidconnect` relying-party library.
//! iglu never sees passwords; the provider hosts login and MFA.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iglu_domain::auth::{Issuer, VerifiedIdentity};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use tokio::sync::RwLock;
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

/// How long any one request to the identity provider may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

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
        .timeout(REQUEST_TIMEOUT)
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

/// The identity provider's metadata and signing keys, discovered on first
/// use and cached. iglud starts and serves without the provider; sign-in
/// waits until it answers. The cache is dropped when a token fails to
/// verify, so rotated keys are picked up on the next attempt.
pub struct Provider {
    http: reqwest::Client,
    issuer: IssuerUrl,
    metadata: RwLock<Option<Arc<CoreProviderMetadata>>>,
}

impl Provider {
    /// # Errors
    ///
    /// When the issuer isn't a URL.
    pub fn new(http: reqwest::Client, issuer: &Issuer) -> Result<Self, OidcError> {
        let issuer =
            IssuerUrl::new(issuer.to_string()).map_err(|e| OidcError::Config(e.to_string()))?;
        Ok(Self {
            http,
            issuer,
            metadata: RwLock::new(None),
        })
    }

    /// # Errors
    ///
    /// When the provider can't be reached or its metadata doesn't parse.
    pub async fn metadata(&self) -> Result<Arc<CoreProviderMetadata>, OidcError> {
        if let Some(metadata) = self.metadata.read().await.as_ref() {
            return Ok(metadata.clone());
        }
        let mut slot = self.metadata.write().await;
        if let Some(metadata) = slot.as_ref() {
            return Ok(metadata.clone());
        }
        let fetch = self.http.clone();
        let discovered =
            CoreProviderMetadata::discover_async(self.issuer.clone(), &move |request| {
                send(fetch.clone(), request)
            })
            .await
            .map_err(|e| OidcError::Discovery(e.to_string()))?;
        let discovered = Arc::new(discovered);
        *slot = Some(discovered.clone());
        Ok(discovered)
    }

    async fn forget(&self) {
        *self.metadata.write().await = None;
    }
}

/// One relying party: the console or the preview gateway.
pub struct RelyingParty {
    provider: Arc<Provider>,
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
    #[must_use]
    pub fn new(
        provider: Arc<Provider>,
        client_id: String,
        client_secret: String,
        redirect: Url,
    ) -> Self {
        Self {
            provider,
            client_id: ClientId::new(client_id),
            client_secret: ClientSecret::new(client_secret),
            redirect: RedirectUrl::from_url(redirect),
        }
    }

    fn client(
        &self,
        metadata: &CoreProviderMetadata,
    ) -> CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    > {
        CoreClient::from_provider_metadata(
            metadata.clone(),
            self.client_id.clone(),
            Some(self.client_secret.clone()),
        )
        .set_redirect_uri(self.redirect.clone())
    }

    /// # Errors
    ///
    /// When the provider can't be discovered.
    pub async fn begin(&self) -> Result<Pending, OidcError> {
        let metadata = self.provider.metadata().await?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = self
            .client(&metadata)
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("email".into()))
            .add_scope(Scope::new("profile".into()))
            .set_pkce_challenge(challenge)
            .url();
        Ok(Pending {
            url,
            state: state.secret().clone(),
            nonce: nonce.secret().clone(),
            pkce_verifier: verifier.secret().clone(),
        })
    }

    /// Exchanges the code and verifies the ID token.
    ///
    /// # Errors
    ///
    /// When the provider can't be reached, refuses the code, or returns a
    /// token that doesn't verify or lacks usable claims.
    pub async fn finish(
        &self,
        code: String,
        nonce: String,
        pkce_verifier: String,
    ) -> Result<VerifiedIdentity, OidcError> {
        let result = self.exchange(code, nonce, pkce_verifier).await;
        if matches!(result, Err(OidcError::Token(_))) {
            self.provider.forget().await;
        }
        result
    }

    async fn exchange(
        &self,
        code: String,
        nonce: String,
        pkce_verifier: String,
    ) -> Result<VerifiedIdentity, OidcError> {
        let metadata = self.provider.metadata().await?;
        let client = self.client(&metadata);
        let http = self.provider.http.clone();
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

        let issuer: Issuer = parse_claim("issuer", claims.issuer().as_str())?;
        let subject = parse_claim("subject", claims.subject().as_str())?;
        let email = match (claims.email(), claims.email_verified()) {
            (Some(email), Some(true)) => Some(parse_claim("email", email.as_str())?),
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

/// Parses one claim of an ID token into its domain type.
fn parse_claim<T: std::str::FromStr<Err = iglu_domain::ParseError>>(
    what: &'static str,
    value: &str,
) -> Result<T, OidcError> {
    value
        .parse()
        .map_err(|e: iglu_domain::ParseError| OidcError::Claims(format!("{what}: {e}")))
}

/// Fetches and caches the service token iglud presents to hosts, through
/// the OAuth client-credentials grant.
pub struct WorkerTokens {
    provider: Arc<Provider>,
    client_id: String,
    client_secret: String,
    resource: Option<Url>,
    cached: tokio::sync::Mutex<Option<(String, Instant)>>,
}

#[derive(serde::Deserialize)]
struct TokenReply {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

impl WorkerTokens {
    #[must_use]
    pub fn new(
        provider: Arc<Provider>,
        client_id: String,
        client_secret: String,
        resource: Option<Url>,
    ) -> Self {
        Self {
            provider,
            client_id,
            client_secret,
            resource,
            cached: tokio::sync::Mutex::new(None),
        }
    }

    /// A current service token, from the cache or the provider.
    ///
    /// # Errors
    ///
    /// When the provider can't be reached or refuses the client.
    pub async fn token(&self) -> Result<String, OidcError> {
        let mut cached = self.cached.lock().await;
        if let Some((token, valid_until)) = cached.as_ref()
            && Instant::now() < *valid_until
        {
            return Ok(token.clone());
        }
        let metadata = self.provider.metadata().await?;
        let endpoint = metadata
            .token_endpoint()
            .ok_or_else(|| OidcError::Config("the provider has no token endpoint".into()))?
            .url()
            .clone();
        let mut form = vec![("grant_type", "client_credentials")];
        if let Some(resource) = &self.resource {
            form.push(("resource", resource.as_str()));
        }
        let reply: TokenReply = self
            .provider
            .http
            .post(endpoint)
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&form)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| OidcError::Exchange(e.to_string()))?
            .json()
            .await
            .map_err(|e| OidcError::Exchange(e.to_string()))?;
        let lifetime = reply.expires_in.unwrap_or(300).saturating_sub(60).max(30);
        let valid_until = Instant::now() + Duration::from_secs(lifetime);
        *cached = Some((reply.access_token.clone(), valid_until));
        Ok(reply.access_token)
    }
}
