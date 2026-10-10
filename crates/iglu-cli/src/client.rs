//! A typed client for iglud's API: from a person's machine with their stored
//! API token, or from inside a workspace through its channel, as that
//! workspace.

use anyhow::{Context, anyhow, bail};
use iglu_api::{
    ActivityEntry, AddColumn, BuildStarted, ColumnOutput, ColumnStatus, CreateEnvironment,
    CreateProject, CreateWorkspace, EnvironmentView, ErrorBody, ErrorKind, Identity, LiveView,
    ProjectView, PublishPort, PutAccess, PutSecret, RenameWorkspace, RouteView, SecretView,
    SendInput, SetDesiredState, WorkspaceView,
};
use iglu_domain::column::ColumnSpec;
use iglu_domain::env::EnvName;
use iglu_domain::guest::CHANNEL_ENV;
use iglu_domain::id::{ProjectId, WorkspaceId};
use iglu_domain::secret::SecretName;
use iglu_domain::terminal::{OutputLines, SessionName};
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;

use crate::login::Stored;

pub struct Client {
    http: reqwest::Client,
    server: url::Url,
    credential: Credential,
}

/// How a request says who it's from.
enum Credential {
    /// A person's API token.
    Token(String),
    /// Nothing: whoever is on the other end of the channel knows which
    /// workspace it is.
    Channel,
}

impl Client {
    /// Inside a workspace, through its channel; elsewhere, as whoever
    /// signed in. A workspace never uses a person's token, even one that's
    /// there.
    pub fn connect() -> anyhow::Result<Self> {
        match std::env::var_os(CHANNEL_ENV) {
            Some(channel) => Self::through(std::path::Path::new(&channel)),
            None => Self::from_stored(),
        }
    }

    fn from_stored() -> anyhow::Result<Self> {
        let stored = Stored::load().context("not signed in; run `iglu login <server>`")?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self {
            http,
            server: stored.server,
            credential: Credential::Token(stored.token),
        })
    }

    fn through(channel: &std::path::Path) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .unix_socket(channel)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self {
            http,
            // Only the path matters on the channel.
            server: url::Url::parse("http://iglu/").context("the channel's URL")?,
            credential: Credential::Channel,
        })
    }

    /// Whether this is a workspace asking from inside.
    pub const fn inside(&self) -> bool {
        matches!(self.credential, Credential::Channel)
    }

    pub async fn identity(&self) -> anyhow::Result<Identity> {
        self.json(self.http.get(self.url("/v1/identity")?)).await
    }

    pub async fn add_column(
        &self,
        id: WorkspaceId,
        request: &AddColumn,
    ) -> anyhow::Result<ColumnSpec> {
        let url = self.url(&format!("/v1/workspaces/{id}/columns"))?;
        self.json(self.http.post(url).json(request)).await
    }

    pub async fn close_column(&self, id: WorkspaceId, column: &SessionName) -> anyhow::Result<()> {
        let url = self.url(&format!("/v1/workspaces/{id}/columns/{column}"))?;
        self.check(self.start(self.http.delete(url)).await?).await?;
        Ok(())
    }

    pub async fn restart_column(
        &self,
        id: WorkspaceId,
        column: &SessionName,
    ) -> anyhow::Result<()> {
        let url = self.url(&format!("/v1/workspaces/{id}/columns/{column}/restart"))?;
        self.check(self.start(self.http.post(url)).await?).await?;
        Ok(())
    }

    pub async fn output(
        &self,
        id: WorkspaceId,
        column: &SessionName,
        lines: OutputLines,
    ) -> anyhow::Result<ColumnOutput> {
        let url = self.url(&format!(
            "/v1/workspaces/{id}/columns/{column}/output?lines={lines}"
        ))?;
        self.json(self.http.get(url)).await
    }

    pub async fn send_input(
        &self,
        id: WorkspaceId,
        column: &SessionName,
        request: &SendInput,
    ) -> anyhow::Result<()> {
        let url = self.url(&format!("/v1/workspaces/{id}/columns/{column}/input"))?;
        self.check(self.start(self.http.post(url).json(request)).await?)
            .await?;
        Ok(())
    }

    pub async fn put_access(
        &self,
        id: WorkspaceId,
        request: &PutAccess,
    ) -> anyhow::Result<WorkspaceView> {
        let url = self.url(&format!("/v1/workspaces/{id}/access"))?;
        self.json(self.http.put(url).json(request)).await
    }

    pub async fn workspaces(&self) -> anyhow::Result<Vec<WorkspaceView>> {
        self.json(self.http.get(self.url("/v1/workspaces")?)).await
    }

    /// `None` once the workspace is gone.
    pub async fn workspace(&self, id: WorkspaceId) -> anyhow::Result<Option<WorkspaceView>> {
        let response = self
            .start(self.http.get(self.url(&format!("/v1/workspaces/{id}"))?))
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(self.parse(response).await?))
    }

    /// Finds a workspace by name or ID.
    pub async fn resolve(&self, reference: &str) -> anyhow::Result<WorkspaceView> {
        self.workspaces()
            .await?
            .into_iter()
            .find(|ws| ws.name.as_str() == reference || ws.id.to_string() == reference)
            .with_context(|| format!("no workspace named {reference}"))
    }

    /// Safe to retry: the same key returns the same workspace.
    pub async fn create_workspace(
        &self,
        request: &CreateWorkspace,
        key: &str,
    ) -> anyhow::Result<WorkspaceView> {
        self.json(
            self.http
                .post(self.url("/v1/workspaces")?)
                .header("idempotency-key", key)
                .json(request),
        )
        .await
    }

    pub async fn set_desired_state(
        &self,
        id: WorkspaceId,
        request: &SetDesiredState,
    ) -> anyhow::Result<WorkspaceView> {
        let url = self.url(&format!("/v1/workspaces/{id}/desired-state"))?;
        self.json(self.http.put(url).json(request)).await
    }

    pub async fn rename(
        &self,
        id: WorkspaceId,
        request: &RenameWorkspace,
    ) -> anyhow::Result<WorkspaceView> {
        let url = self.url(&format!("/v1/workspaces/{id}/name"))?;
        self.json(self.http.put(url).json(request)).await
    }

    pub async fn publish(
        &self,
        id: WorkspaceId,
        request: &PublishPort,
    ) -> anyhow::Result<RouteView> {
        let url = self.url(&format!("/v1/workspaces/{id}/routes"))?;
        self.json(self.http.post(url).json(request)).await
    }

    pub async fn routes(&self, id: WorkspaceId) -> anyhow::Result<Vec<RouteView>> {
        let url = self.url(&format!("/v1/workspaces/{id}/routes"))?;
        self.json(self.http.get(url)).await
    }

    pub async fn activity(&self, id: WorkspaceId) -> anyhow::Result<Vec<ActivityEntry>> {
        let url = self.url(&format!("/v1/workspaces/{id}/activity"))?;
        self.json(self.http.get(url)).await
    }

    pub async fn projects(&self) -> anyhow::Result<Vec<ProjectView>> {
        self.json(self.http.get(self.url("/v1/projects")?)).await
    }

    /// Finds a project by name or ID.
    pub async fn resolve_project(&self, reference: &str) -> anyhow::Result<ProjectView> {
        self.projects()
            .await?
            .into_iter()
            .find(|p| p.name.as_str() == reference || p.id.to_string() == reference)
            .with_context(|| format!("no project named {reference}; see `iglu project ls`"))
    }

    pub async fn create_project(&self, request: &CreateProject) -> anyhow::Result<ProjectView> {
        let url = self.url("/v1/projects")?;
        self.json(self.http.post(url).json(request)).await
    }

    pub async fn delete_project(&self, id: ProjectId) -> anyhow::Result<()> {
        let url = self.url(&format!("/v1/projects/{id}"))?;
        self.check(self.start(self.http.delete(url)).await?).await?;
        Ok(())
    }

    pub async fn environments(&self) -> anyhow::Result<Vec<EnvironmentView>> {
        self.json(self.http.get(self.url("/v1/environments")?))
            .await
    }

    pub async fn create_environment(
        &self,
        request: &CreateEnvironment,
    ) -> anyhow::Result<BuildStarted> {
        let url = self.url("/v1/environments")?;
        self.json(self.http.post(url).json(request)).await
    }

    pub async fn build_environment(&self, name: &EnvName) -> anyhow::Result<BuildStarted> {
        let url = self.url(&format!("/v1/environments/{name}/builds"))?;
        self.json(self.http.post(url)).await
    }

    pub async fn secrets(&self) -> anyhow::Result<Vec<SecretView>> {
        self.json(self.http.get(self.url("/v1/secrets")?)).await
    }

    pub async fn put_secret(&self, name: &SecretName, request: &PutSecret) -> anyhow::Result<()> {
        let url = self.url(&format!("/v1/secrets/{name}"))?;
        self.check(self.start(self.http.put(url).json(request)).await?)
            .await?;
        Ok(())
    }

    pub async fn delete_secret(&self, name: &SecretName) -> anyhow::Result<()> {
        let url = self.url(&format!("/v1/secrets/{name}"))?;
        self.check(self.start(self.http.delete(url)).await?).await?;
        Ok(())
    }

    pub async fn live(&self, id: WorkspaceId) -> anyhow::Result<LiveView> {
        let url = self.url(&format!("/v1/workspaces/{id}/live"))?;
        self.json(self.http.get(url)).await
    }

    pub async fn columns(&self, id: WorkspaceId) -> anyhow::Result<Vec<ColumnStatus>> {
        let url = self.url(&format!("/v1/workspaces/{id}/columns"))?;
        self.json(self.http.get(url)).await
    }

    /// Where a column is attached, and the token that may.
    pub fn attach_target(
        &self,
        id: WorkspaceId,
        column: &SessionName,
    ) -> anyhow::Result<(url::Url, &str)> {
        let Credential::Token(token) = &self.credential else {
            bail!(
                "a column can't be attached from inside a workspace; use `iglu column output` and `iglu column send`"
            );
        };
        Ok((
            self.url(&format!("/v1/workspaces/{id}/columns/{column}/attach"))?,
            token,
        ))
    }

    fn url(&self, path: &str) -> anyhow::Result<url::Url> {
        Ok(self.server.join(path)?)
    }

    async fn start(&self, request: RequestBuilder) -> anyhow::Result<Response> {
        let request = match &self.credential {
            Credential::Token(token) => request.bearer_auth(token),
            Credential::Channel => request,
        };
        request.send().await.context("reaching iglu")
    }

    async fn json<T: DeserializeOwned>(&self, request: RequestBuilder) -> anyhow::Result<T> {
        let response = self.start(request).await?;
        self.parse(response).await
    }

    async fn parse<T: DeserializeOwned>(&self, response: Response) -> anyhow::Result<T> {
        let body = self
            .check(response)
            .await?
            .bytes()
            .await
            .context("reading iglu's reply")?;
        reply(&body)
    }

    /// Passes a success through; turns a refusal into an error saying why.
    async fn check(&self, response: Response) -> anyhow::Result<Response> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.bytes().await.unwrap_or_default();
        if self.inside() && status == StatusCode::UNAUTHORIZED {
            bail!("this workspace can't reach iglu any more; it may be being deleted");
        }
        Err(anyhow!(refusal(status, &body, &self.server)))
    }
}

/// A successful reply as `T`, or where it differs from what this CLI expects.
pub fn reply<T: DeserializeOwned>(body: &[u8]) -> anyhow::Result<T> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let at = if error.path().iter().next().is_some() {
            format!(" at {}", error.path())
        } else {
            String::new()
        };
        anyhow!(
            "iglu's reply didn't match this CLI{at}: {}; is the CLI the same version as the server?",
            error.inner()
        )
    })
}

/// What iglu said when it refused a request, for a person to read. Anything
/// that isn't iglu's error body came from something else at that address.
pub fn refusal(status: StatusCode, body: &[u8], server: &url::Url) -> String {
    match serde_json::from_slice::<ErrorBody>(body) {
        Ok(ErrorBody {
            error: ErrorKind::Unauthorized,
            message,
            ..
        }) => format!("{message}; run `iglu login` again"),
        Ok(ErrorBody {
            field: Some(field),
            message,
            ..
        }) => format!("{field}: {message}"),
        Ok(ErrorBody { message, .. }) => message,
        Err(_) => format!("{server} answered {status}, which isn't an answer from iglu"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> url::Url {
        url::Url::parse("https://iglu.example.org").expect("a URL")
    }

    #[test]
    fn refusals_name_their_field() {
        let body = br#"{"error":"bad_request","message":"invalid DNS label: may not start or end with '-'","field":"name"}"#;
        assert_eq!(
            refusal(StatusCode::BAD_REQUEST, body, &server()),
            "name: invalid DNS label: may not start or end with '-'"
        );
    }

    #[test]
    fn an_ended_session_says_how_to_start_another() {
        let body = br#"{"error":"unauthorized","message":"sign in first","field":null}"#;
        assert_eq!(
            refusal(StatusCode::UNAUTHORIZED, body, &server()),
            "sign in first; run `iglu login` again"
        );
    }

    #[test]
    fn a_refusal_that_isnt_iglus_says_where_it_came_from() {
        let message = refusal(
            StatusCode::BAD_GATEWAY,
            b"<html>bad gateway</html>",
            &server(),
        );
        assert!(
            message.starts_with("https://iglu.example.org/ answered 502"),
            "{message}"
        );
    }

    #[test]
    fn a_reply_that_doesnt_match_says_where() {
        let error = reply::<PublishPort>(br#"{"port":"x"}"#).expect_err("a string isn't a port");
        assert!(error.to_string().contains(" at port: "), "{error}");
        assert!(reply::<PublishPort>(br#"{"port":3000}"#).is_ok());
    }
}
