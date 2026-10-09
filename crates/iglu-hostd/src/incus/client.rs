//! A small typed client for the parts of the Incus REST API iglu uses, over
//! the local Unix socket.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use iglu_domain::id::InstanceName;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::UnixStream;
use tokio_tungstenite::WebSocketStream;

/// Responses larger than this are refused, so a confused daemon can't make us
/// buffer without bound.
const MAX_RESPONSE: usize = 64 * 1024 * 1024;

const QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'/');

#[derive(Debug, thiserror::Error)]
pub enum IncusError {
    #[error("connecting to Incus: {0}")]
    Connect(#[source] std::io::Error),
    #[error("talking to Incus: {0}")]
    Http(String),
    #[error("Incus returned an unexpected response: {0}")]
    Protocol(String),
    #[error("Incus: {message}")]
    Api { status: u16, message: String },
    #[error("Incus operation failed: {0}")]
    Operation(String),
}

impl IncusError {
    #[must_use]
    pub const fn is_not_found(&self) -> bool {
        matches!(self, Self::Api { status: 404, .. })
    }
}

#[derive(Clone, Debug)]
pub struct Incus {
    socket: PathBuf,
    project: Option<String>,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    error_code: u16,
    #[serde(default)]
    error: String,
    #[serde(default)]
    operation: String,
    #[serde(default)]
    metadata: Value,
}

/// The finished state of an asynchronous operation.
#[derive(Debug, Deserialize)]
struct Operation {
    status_code: u16,
    #[serde(default)]
    err: String,
    #[serde(default)]
    metadata: Value,
}

impl Incus {
    pub const fn new(socket: PathBuf, project: Option<String>) -> Self {
        Self { socket, project }
    }

    fn uri(&self, path: &str) -> String {
        match &self.project {
            Some(project) => {
                let sep = if path.contains('?') { '&' } else { '?' };
                format!("{path}{sep}project={}", utf8_percent_encode(project, QUERY))
            }
            None => path.to_owned(),
        }
    }

    async fn send(
        &self,
        request: Request<Full<Bytes>>,
    ) -> Result<hyper::Response<hyper::body::Incoming>, IncusError> {
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(IncusError::Connect)?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| IncusError::Http(e.to_string()))?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "Incus connection closed with an error");
            }
        });
        sender
            .send_request(request)
            .await
            .map_err(|e| IncusError::Http(e.to_string()))
    }

    fn request(&self, method: Method, path: &str) -> hyper::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(self.uri(path))
            .header("host", "incus")
    }

    async fn read_body(
        response: hyper::Response<hyper::body::Incoming>,
    ) -> Result<(StatusCode, Bytes), IncusError> {
        let status = response.status();
        let body = Limited::new(response.into_body(), MAX_RESPONSE)
            .collect()
            .await
            .map_err(|e| IncusError::Http(e.to_string()))?
            .to_bytes();
        Ok((status, body))
    }

    /// Sends a JSON request and returns the envelope, turning API errors into `Err`.
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Envelope, IncusError> {
        let payload = match body {
            Some(value) => Bytes::from(
                serde_json::to_vec(value).map_err(|e| IncusError::Protocol(e.to_string()))?,
            ),
            None => Bytes::new(),
        };
        let request = self
            .request(method, path)
            .header("content-type", "application/json")
            .body(Full::new(payload))
            .map_err(|e| IncusError::Http(e.to_string()))?;
        let (status, bytes) = Self::read_body(self.send(request).await?).await?;
        let envelope: Envelope = serde_json::from_slice(&bytes)
            .map_err(|e| IncusError::Protocol(format!("{status}: {e}")))?;
        match envelope.kind.as_str() {
            "sync" | "async" => Ok(envelope),
            "error" => Err(IncusError::Api {
                status: envelope.error_code,
                message: envelope.error,
            }),
            other => Err(IncusError::Protocol(format!(
                "unknown response type {other}"
            ))),
        }
    }

    /// A synchronous call whose metadata parses as `T`.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, IncusError> {
        let envelope = self.call(Method::GET, path, None).await?;
        serde_json::from_value(envelope.metadata)
            .map_err(|e| IncusError::Protocol(format!("{path}: {e}")))
    }

    /// A call that may start an operation; waits for it and returns its metadata.
    pub async fn run(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Value, IncusError> {
        let envelope = self.call(method, path, body).await?;
        match envelope.kind.as_str() {
            "async" => self.wait(&envelope.operation, timeout).await,
            _ => Ok(envelope.metadata),
        }
    }

    /// Starts an operation without waiting and returns its path and metadata.
    pub async fn start(
        &self,
        method: Method,
        path: &str,
        body: &Value,
    ) -> Result<(String, Value), IncusError> {
        let envelope = self.call(method, path, Some(body)).await?;
        if envelope.kind != "async" {
            return Err(IncusError::Protocol(format!(
                "{path} did not start an operation"
            )));
        }
        let metadata = envelope
            .metadata
            .get("metadata")
            .cloned()
            .unwrap_or(Value::Null);
        Ok((envelope.operation, metadata))
    }

    pub async fn wait(&self, operation: &str, timeout: Duration) -> Result<Value, IncusError> {
        let path = format!("{operation}/wait?timeout={}", timeout.as_secs().max(1));
        let envelope = self.call(Method::GET, &path, None).await?;
        let op: Operation = serde_json::from_value(envelope.metadata)
            .map_err(|e| IncusError::Protocol(e.to_string()))?;
        match op.status_code {
            200 => Ok(op.metadata),
            103 | 105 => Err(IncusError::Operation(format!(
                "timed out after {}s",
                timeout.as_secs()
            ))),
            _ => Err(IncusError::Operation(op.err)),
        }
    }

    /// Opens one of an operation's `WebSockets`.
    pub async fn websocket(
        &self,
        operation: &str,
        secret: &str,
    ) -> Result<WebSocketStream<UnixStream>, IncusError> {
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(IncusError::Connect)?;
        let uri = format!(
            "ws://incus{}",
            self.uri(&format!(
                "{operation}/websocket?secret={}",
                utf8_percent_encode(secret, QUERY)
            ))
        );
        let (socket, _) = tokio_tungstenite::client_async(uri, stream)
            .await
            .map_err(|e| IncusError::Http(e.to_string()))?;
        Ok(socket)
    }

    pub async fn instance(&self, name: InstanceName) -> Result<Option<InstanceFull>, IncusError> {
        match self
            .get::<InstanceFull>(&format!("/1.0/instances/{name}?recursion=1"))
            .await
        {
            Ok(instance) => Ok(Some(instance)),
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn instances(&self) -> Result<Vec<InstanceFull>, IncusError> {
        self.get("/1.0/instances?recursion=2").await
    }

    /// Replaces an instance's writable configuration, keeping everything else.
    pub async fn update_instance(
        &self,
        name: InstanceName,
        update: impl FnOnce(&mut Value),
        timeout: Duration,
    ) -> Result<(), IncusError> {
        let mut current: Value = self.get(&format!("/1.0/instances/{name}")).await?;
        update(&mut current);
        self.run(
            Method::PUT,
            &format!("/1.0/instances/{name}"),
            Some(&current),
            timeout,
        )
        .await
        .map(|_| ())
    }

    pub async fn change_state(
        &self,
        name: InstanceName,
        action: StateAction,
        timeout: Duration,
    ) -> Result<(), IncusError> {
        let body = match action {
            StateAction::Start => serde_json::json!({ "action": "start" }),
            StateAction::Freeze => serde_json::json!({ "action": "freeze" }),
            StateAction::Unfreeze => serde_json::json!({ "action": "unfreeze" }),
            StateAction::Stop { timeout_secs } => {
                serde_json::json!({ "action": "stop", "timeout": timeout_secs })
            }
            StateAction::ForceStop => serde_json::json!({ "action": "stop", "force": true }),
        };
        self.run(
            Method::PUT,
            &format!("/1.0/instances/{name}/state"),
            Some(&body),
            timeout,
        )
        .await
        .map(|_| ())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum StateAction {
    Start,
    Freeze,
    Unfreeze,
    Stop { timeout_secs: u32 },
    ForceStop,
}

/// The fields of an Incus instance iglu reads.
#[derive(Clone, Debug, Deserialize)]
pub struct InstanceFull {
    pub name: String,
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    #[serde(default)]
    pub state: Option<InstanceState>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InstanceState {
    pub status_code: u16,
    #[serde(default)]
    pub pid: i64,
    #[serde(default)]
    pub memory: MemoryState,
    #[serde(default)]
    pub network: Option<BTreeMap<String, NetworkState>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MemoryState {
    #[serde(default)]
    pub usage: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkState {
    #[serde(default)]
    pub addresses: Vec<NetworkAddress>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkAddress {
    pub family: String,
    pub scope: String,
}
