//! The client side of the hostd protocol.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use iglu_api::ColumnOutput;
use iglu_domain::download::DownloadName;
use iglu_domain::env::{EnvSource, GuestPath};
use iglu_domain::git::GitState;
use iglu_domain::id::WorkspaceId;
use iglu_domain::label::HostId;
use iglu_domain::listener::Listener;
use iglu_domain::pasted::FileName;
use iglu_domain::port::GuestPort;
use iglu_domain::secret::FetchTokens;
use iglu_domain::terminal::{OutputLines, SessionName, TerminalInput, TerminalSize};
use iglu_proto::{
    BuildOutcome, BuildRequest, Command, CommandError, CommandOutcome, ErrorCode, GUEST_WORKSPACE,
    Inventory, KeptFile, PROTOCOL_VERSION, PruneImages, PrunedImages, SessionSpec, TUNNEL_UPGRADE,
    TerminalInfo, TerminalOutput, TerminalTyping, path,
};
use serde::de::DeserializeOwned;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream, tungstenite};
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

    async fn token(&self) -> Result<String, HostError> {
        self.tokens
            .token()
            .await
            .map_err(|e| HostError::Token(e.to_string()))
    }

    /// Sends a request with iglud's token. One the host refuses isn't used
    /// again, whatever made it stale.
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, HostError> {
        let token = self.token().await?;
        let response = request
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| HostError::Unreachable(e.to_string()))?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.tokens.forget(&token).await;
        }
        Ok(response)
    }

    async fn json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, HostError> {
        let response = self.send(request).await?;
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
            // Both can move a lot: a repository, or a new system.
            Command::Provision(_) | Command::Upgrade(_) => Duration::from_secs(1900),
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
        self.empty(
            self.http
                .post(self.url(&path::terminals(workspace))?)
                .json(session),
        )
        .await
    }

    pub async fn close_terminal(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
    ) -> Result<(), HostError> {
        self.empty(
            self.http
                .delete(self.url(&path::terminal(workspace, session))?),
        )
        .await
    }

    /// Sends a request that answers with nothing but whether it worked.
    async fn empty(&self, request: reqwest::RequestBuilder) -> Result<(), HostError> {
        let response = self.send(request.timeout(Duration::from_secs(60))).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(match response.json::<CommandError>().await {
                Ok(error) => HostError::Command(error),
                Err(e) => HostError::Unreachable(e.to_string()),
            })
        }
    }

    pub async fn prune_images(&self, request: &PruneImages) -> Result<PrunedImages, HostError> {
        self.json(
            self.http
                .post(self.url(path::IMAGE_PRUNE)?)
                .json(request)
                .timeout(Duration::from_secs(600)),
        )
        .await
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
        let token = self.token().await?;
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .map_err(|_| HostError::Token("unusable token".into()))?,
        );
        let (socket, _) = match tokio_tungstenite::connect_async_tls_with_config(
            request,
            None,
            true,
            Some(Connector::Rustls(self.tls.clone())),
        )
        .await
        {
            Ok(connected) => connected,
            Err(tungstenite::Error::Http(response))
                if response.status() == http::StatusCode::UNAUTHORIZED =>
            {
                self.tokens.forget(&token).await;
                return Err(HostError::Unreachable(
                    "host answered 401 Unauthorized".into(),
                ));
            }
            Err(error) => return Err(HostError::Unreachable(error.to_string())),
        };
        Ok(socket)
    }

    pub async fn output(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
        lines: OutputLines,
    ) -> Result<ColumnOutput, HostError> {
        let output: TerminalOutput = self
            .json(
                self.http
                    .get(self.url(&path::output(workspace, session, lines))?),
            )
            .await?;
        Ok(ColumnOutput {
            text: output.text,
            truncated: output.truncated,
        })
    }

    pub async fn send_input(
        &self,
        workspace: WorkspaceId,
        session: &SessionName,
        text: &TerminalInput,
    ) -> Result<(), HostError> {
        let typing = TerminalTyping { text: text.clone() };
        self.empty(
            self.http
                .post(self.url(&path::input(workspace, session))?)
                .json(&typing),
        )
        .await
    }

    pub async fn keep(
        &self,
        workspace: WorkspaceId,
        name: &FileName,
        bytes: Bytes,
    ) -> Result<GuestPath, HostError> {
        let kept: KeptFile = self
            .json(
                self.http
                    .post(self.url(&path::files(workspace, name))?)
                    .body(bytes)
                    .timeout(Duration::from_secs(120)),
            )
            .await?;
        Ok(kept.path)
    }

    /// A file the workspace's browser downloaded, as the host streams it;
    /// `None` when there's no such file.
    pub async fn download(
        &self,
        workspace: WorkspaceId,
        name: &DownloadName,
    ) -> Result<Option<reqwest::Response>, HostError> {
        let mut url = self.url(&format!("/v1/workspaces/{workspace}/downloads/"))?;
        url.path_segments_mut()
            .map_err(|()| HostError::Unreachable("the host's address can't have a path".into()))?
            .pop_if_empty()
            .push(name.as_str());
        let response = self.send(self.http.get(url)).await?;
        let status = response.status();
        if status.is_success() {
            return Ok(Some(response));
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        match response.json::<CommandError>().await {
            Ok(error) => Err(HostError::Command(error)),
            Err(_) => Err(HostError::Unreachable(format!("host answered {status}"))),
        }
    }

    /// Waits for the next connection a workspace on this host makes through
    /// its channel. `None` when none came while hostd held the request.
    pub async fn guest_connection(
        &self,
    ) -> Result<Option<(WorkspaceId, reqwest::Upgraded)>, HostError> {
        let response = self
            .send(
                self.http
                    .get(self.url(path::GUEST_CONNECTIONS)?)
                    .header(reqwest::header::CONNECTION, "upgrade")
                    .header(reqwest::header::UPGRADE, TUNNEL_UPGRADE),
            )
            .await?;
        match response.status() {
            reqwest::StatusCode::NO_CONTENT => return Ok(None),
            reqwest::StatusCode::SWITCHING_PROTOCOLS => {}
            status => return Err(HostError::Unreachable(format!("host answered {status}"))),
        }
        let workspace = response
            .headers()
            .get(GUEST_WORKSPACE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<WorkspaceId>().ok())
            .ok_or_else(|| {
                HostError::Unreachable("a guest connection without its workspace".into())
            })?;
        let upgraded = response
            .upgrade()
            .await
            .map_err(|e| HostError::Unreachable(e.to_string()))?;
        Ok(Some((workspace, upgraded)))
    }

    /// Opens a raw byte stream to a guest port.
    pub async fn tunnel(
        &self,
        workspace: WorkspaceId,
        port: GuestPort,
    ) -> Result<reqwest::Upgraded, HostError> {
        let response = self
            .send(
                self.http
                    .get(self.url(&path::tunnel(workspace, port))?)
                    .header(reqwest::header::CONNECTION, "upgrade")
                    .header(reqwest::header::UPGRADE, TUNNEL_UPGRADE),
            )
            .await?;
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
