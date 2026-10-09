//! A typed client for iglud's API using a stored API token.

use anyhow::{Context, bail};
use iglu_api::{
    ActivityEntry, BuildStarted, ColumnStatus, CreateEnvironment, CreateProject, CreateWorkspace,
    EnvironmentView, ErrorBody, LiveView, ProjectView, PublishPort, PutSecret, RenameWorkspace,
    RouteView, SecretView, SetDesiredState, WorkspaceView,
};
use iglu_domain::env::EnvName;
use iglu_domain::id::{ProjectId, WorkspaceId};
use iglu_domain::secret::SecretName;
use iglu_domain::terminal::SessionName;
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;

use crate::login::Stored;

pub struct Client {
    http: reqwest::Client,
    server: url::Url,
    token: String,
}

impl Client {
    pub fn from_stored() -> anyhow::Result<Self> {
        let stored = Stored::load().context("not signed in; run `iglu login <server>`")?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self {
            http,
            server: stored.server,
            token: stored.token,
        })
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
        Ok(Some(parse(response).await?))
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
        check(self.start(self.http.delete(url)).await?).await?;
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
        check(self.start(self.http.put(url).json(request)).await?).await?;
        Ok(())
    }

    pub async fn delete_secret(&self, name: &SecretName) -> anyhow::Result<()> {
        let url = self.url(&format!("/v1/secrets/{name}"))?;
        check(self.start(self.http.delete(url)).await?).await?;
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
        Ok((
            self.url(&format!("/v1/workspaces/{id}/columns/{column}/attach"))?,
            &self.token,
        ))
    }

    fn url(&self, path: &str) -> anyhow::Result<url::Url> {
        Ok(self.server.join(path)?)
    }

    async fn start(&self, request: RequestBuilder) -> anyhow::Result<Response> {
        request
            .bearer_auth(&self.token)
            .send()
            .await
            .context("reaching iglu")
    }

    async fn json<T: DeserializeOwned>(&self, request: RequestBuilder) -> anyhow::Result<T> {
        parse(self.start(request).await?).await
    }
}

async fn parse<T: DeserializeOwned>(response: Response) -> anyhow::Result<T> {
    check(response)
        .await?
        .json()
        .await
        .context("reading iglu's reply")
}

/// Turns an error status into an error carrying iglud's message.
async fn check(response: Response) -> anyhow::Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let message = response
        .json::<ErrorBody>()
        .await
        .map_or_else(|_| "request failed".to_owned(), |body| body.message);
    if status == StatusCode::UNAUTHORIZED {
        bail!("{message}; run `iglu login` again");
    }
    bail!("{message} ({status})")
}
