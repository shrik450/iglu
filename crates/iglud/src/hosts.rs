//! The client side of the hostd protocol.

use std::sync::Arc;
use std::time::Duration;

use iglu_domain::env::EnvSource;
use iglu_domain::git::GitState;
use iglu_domain::id::WorkspaceId;
use iglu_domain::label::HostId;
use iglu_domain::listener::Listener;
use iglu_domain::port::GuestPort;
use iglu_domain::secret::FetchTokens;
use iglu_domain::terminal::{SessionName, TerminalSize};
use iglu_proto::{
    BuildOutcome, BuildRequest, Command, CommandError, CommandOutcome, ErrorCode, Inventory,
    PROTOCOL_VERSION, SessionSpec, TUNNEL_UPGRADE, TerminalInfo, path,
};
use serde::de::DeserializeOwned;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};
use url::Url;

use crate::oidc::WorkerTokens;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("host unreachable: {0}")]
    Unreachable(String),
    #[error("host speaks protocol {0}, iglud speaks {PROTOCOL_VERSION}")]
    Protocol(u32),
    #[error("{0}")]
    Command(CommandError),
    #[error("couldn't get a service token: {0}")]
    Token(String),
}

impl HostError {
    pub fn into_command_error(self) -> CommandError {
        match self {
            Self::Command(error) => error,
            Self::Unreachable(_) | Self::Protocol(_) | Self::Token(_) => {
                CommandError::new(ErrorCode::Runtime, self.to_string())
            }
        }
    }
}

pub struct HostClient {
    pub id: HostId,
    base: Url,
    http: reqwest::Client,
    tls: Arc<rustls::ClientConfig>,
    tokens: Arc<WorkerTokens>,
}

pub type Terminal = WebSocketStream<MaybeTlsStream<TcpStream>>;

impl HostClient {
    pub fn new(
        id: HostId,
        base: Url,
        http: reqwest::Client,
        tls: Arc<rustls::ClientConfig>,
        tokens: Arc<WorkerTokens>,
    ) -> Self {
        Self {
            id,
            base,
            http,
            tls,
            tokens,
        }
    }

    fn url(&self, path: &str) -> Result<Url, HostError> {
        self.base
            .join(path)
            .map_err(|e| HostError::Unreachable(e.to_string()))
    }

    async fn bearer(&self) -> Result<String, HostError> {
        self.tokens
            .token()
            .await
            .map(|t| format!("Bearer {t}"))
            .map_err(|e| HostError::Token(e.to_string()))
    }

    async fn json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, HostError> {
        let response = request
            .header(reqwest::header::AUTHORIZATION, self.bearer().await?)
            .send()
            .await
            .map_err(|e| HostError::Unreachable(e.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return response
                .json()
                .await
                .map_err(|e| HostError::Unreachable(e.to_string()));
        }
        match response.json::<CommandError>().await {
            Ok(error) => Err(HostError::Command(error)),
            Err(_) => Err(HostError::Unreachable(format!("host answered {status}"))),
        }
    }

    pub async fn inventory(&self) -> Result<Inventory, HostError> {
        let inventory: Inventory = self
            .json(
                self.http
                    .get(self.url(path::INVENTORY)?)
                    .timeout(Duration::from_secs(20)),
            )
            .await?;
        if inventory.host.protocol != PROTOCOL_VERSION {
            return Err(HostError::Protocol(inventory.host.protocol));
        }
        Ok(inventory)
    }

    pub async fn command(
        &self,
        workspace: WorkspaceId,
        command: &Command,
    ) -> Result<CommandOutcome, HostError> {
        let timeout = match command {
            Command::Provision(_) => Duration::from_secs(1900),
            Command::Create(_)
            | Command::Start
            | Command::DeliverSecrets(_)
            | Command::OpenColumns { .. }
            | Command::Freeze
            | Command::Thaw
            | Command::Stop
            | Command::Delete => Duration::from_secs(300),
        };
        self.json(
            self.http
                .post(self.url(&path::commands(workspace))?)
                .json(command)
                .timeout(timeout),
        )
        .await
    }

    pub async fn terminals(&self, workspace: WorkspaceId) -> Result<Vec<TerminalInfo>, HostError> {
        self.json(
            self.http
                .get(self.url(&path::terminals(workspace))?)
                .timeout(Duration::from_secs(60)),
        )
        .await
    }

    pub async fn git_state(&self, workspace: WorkspaceId) -> Result<Option<GitState>, HostError> {
        self.json(
            self.http
                .get(self.url(&path::git(workspace))?)
                .timeout(Duration::from_secs(60)),
        )
        .await
    }

    pub async fn listeners(&self, workspace: WorkspaceId) -> Result<Vec<Listener>, HostError> {
        self.json(
            self.http
                .get(self.url(&path::listeners(workspace))?)
                .timeout(Duration::from_secs(60)),
        )
        .await
    }

    /// Opens one more terminal session in a running workspace.
    pub async fn open_terminal(
        &self,
        workspace: WorkspaceId,
        session: &SessionSpec,
    ) -> Result<(), HostError> {
        let response = self
            .http
            .post(self.url(&path::terminals(workspace))?)
            .header(reqwest::header::AUTHORIZATION, self.bearer().await?)
            .json(session)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| HostError::Unreachable(e.to_string()))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(match response.json::<CommandError>().await {
                Ok(error) => HostError::Command(error),
                Err(e) => HostError::Unreachable(e.to_string()),
            })
        }
    }

    pub async fn close_terminal(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
    ) -> Result<(), HostError> {
        let response = self
            .http
            .delete(self.url(&path::terminal(workspace, session))?)
            .header(reqwest::header::AUTHORIZATION, self.bearer().await?)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| HostError::Unreachable(e.to_string()))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(match response.json::<CommandError>().await {
                Ok(error) => HostError::Command(error),
                Err(e) => HostError::Unreachable(e.to_string()),
            })
        }
    }

    pub async fn build(
        &self,
        source: &EnvSource,
        tokens: FetchTokens,
    ) -> Result<BuildOutcome, HostError> {
        let request = BuildRequest {
            source: source.clone(),
            tokens,
        };
        self.json(
            self.http
                .post(self.url(path::BUILDS)?)
                .json(&request)
                .timeout(Duration::from_secs(4000)),
        )
        .await
    }

    pub async fn attach(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
        size: TerminalSize,
    ) -> Result<Terminal, HostError> {
        let mut url = self.url(&path::attach(workspace, session))?;
        url.set_query(Some(&format!("cols={}&rows={}", size.cols(), size.rows())));
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|()| HostError::Unreachable("bad host URL".into()))?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| HostError::Unreachable(e.to_string()))?;
        let bearer = self.bearer().await?;
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            bearer
                .parse()
                .map_err(|_| HostError::Token("unusable token".into()))?,
        );
        let (socket, _) = tokio_tungstenite::connect_async_tls_with_config(
            request,
            None,
            true,
            Some(Connector::Rustls(self.tls.clone())),
        )
        .await
        .map_err(|e| HostError::Unreachable(e.to_string()))?;
        Ok(socket)
    }

    /// Opens a raw byte stream to a guest port.
    pub async fn tunnel(
        &self,
        workspace: WorkspaceId,
        port: GuestPort,
    ) -> Result<reqwest::Upgraded, HostError> {
        let response = self
            .http
            .get(self.url(&path::tunnel(workspace, port))?)
            .header(reqwest::header::AUTHORIZATION, self.bearer().await?)
            .header(reqwest::header::CONNECTION, "upgrade")
            .header(reqwest::header::UPGRADE, TUNNEL_UPGRADE)
            .send()
            .await
            .map_err(|e| HostError::Unreachable(e.to_string()))?;
        if response.status() != reqwest::StatusCode::SWITCHING_PROTOCOLS {
            return Err(match response.json::<CommandError>().await {
                Ok(error) => HostError::Command(error),
                Err(e) => HostError::Unreachable(e.to_string()),
            });
        }
        response
            .upgrade()
            .await
            .map_err(|e| HostError::Unreachable(e.to_string()))
    }
}
