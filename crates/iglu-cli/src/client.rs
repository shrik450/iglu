//! A thin client for iglud's API using a stored API token.

use anyhow::{Context, bail};
use serde_json::Value;

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

    async fn send(&self, request: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let response = request
            .bearer_auth(&self.token)
            .send()
            .await
            .context("reaching iglu")?;
        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body);
        }
        let message = body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        match status {
            reqwest::StatusCode::UNAUTHORIZED => bail!("{message}; run `iglu login` again"),
            reqwest::StatusCode::NOT_FOUND => bail!("not found"),
            _ => bail!("{message} ({status})"),
        }
    }

    fn url(&self, path: &str) -> anyhow::Result<url::Url> {
        Ok(self.server.join(path)?)
    }

    pub async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.send(self.http.get(self.url(path)?)).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        self.send(self.http.post(self.url(path)?).json(body)).await
    }

    /// A create that is safe to retry: the same key returns the same workspace.
    pub async fn post_idempotent(
        &self,
        path: &str,
        body: &Value,
        key: &str,
    ) -> anyhow::Result<Value> {
        self.send(
            self.http
                .post(self.url(path)?)
                .header("idempotency-key", key)
                .json(body),
        )
        .await
    }

    pub async fn put(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        self.send(self.http.put(self.url(path)?).json(body)).await
    }

    pub async fn delete(&self, path: &str) -> anyhow::Result<Value> {
        self.send(self.http.delete(self.url(path)?)).await
    }

    /// Finds a workspace by name or ID.
    pub async fn resolve(&self, reference: &str) -> anyhow::Result<Value> {
        let list = self.get("/v1/workspaces").await?;
        list.as_array()
            .into_iter()
            .flatten()
            .find(|ws| {
                ws.get("name").and_then(Value::as_str) == Some(reference)
                    || ws.get("id").and_then(Value::as_str) == Some(reference)
            })
            .cloned()
            .with_context(|| format!("no workspace named {reference}"))
    }
}
