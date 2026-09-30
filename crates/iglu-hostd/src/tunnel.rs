//! Raw byte tunnels to a guest's loopback ports, for the preview gateway.
//!
//! Each published port becomes an Incus proxy device that listens on a
//! host-side Unix socket owned by hostd and connects to `127.0.0.1:<port>`
//! inside the guest. The gateway never gets a route onto the guest network.

use std::path::PathBuf;

use iglu_domain::id::InstanceName;
use iglu_domain::port::GuestPort;
use iglu_proto::{CommandError, ErrorCode};
use serde_json::json;
use tokio::net::UnixStream;

use crate::observe::Owned;
use crate::server::App;

fn device_name(port: GuestPort) -> String {
    format!("iglu-port-{port}")
}

fn socket_path(app: &App, name: InstanceName, port: GuestPort) -> PathBuf {
    app.config
        .runtime_dir
        .join("proxy")
        .join(format!("{name}-{port}.sock"))
}

/// Connects to a guest port, adding the proxy device on first use.
pub async fn connect(
    app: &App,
    name: InstanceName,
    port: GuestPort,
) -> Result<UnixStream, CommandError> {
    let socket = socket_path(app, name, port);
    if let Ok(stream) = UnixStream::connect(&socket).await {
        return Ok(stream);
    }
    let instance = app
        .incus
        .instance(name)
        .await
        .map_err(|e| CommandError::new(ErrorCode::Runtime, e.to_string()))?
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, "no such workspace instance"))?;
    let owned = Owned::parse(instance)
        .map_err(|e| CommandError::new(ErrorCode::Conflict, e.to_string()))?;
    if owned.pid().is_none() {
        return Err(CommandError::new(
            ErrorCode::InvalidState,
            "the workspace isn't running",
        ));
    }
    if let Some(dir) = socket.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| CommandError::new(ErrorCode::Runtime, e.to_string()))?;
    }
    let listen = format!("unix:{}", socket.display());
    let device = json!({
        "type": "proxy",
        "bind": "host",
        "listen": listen,
        "connect": format!("tcp:127.0.0.1:{port}"),
        "mode": "0600",
    });
    let key = device_name(port);
    app.incus
        .update_instance(
            name,
            |instance| {
                instance["devices"][key.as_str()] = device;
            },
            app.config.timeouts.operation(),
        )
        .await
        .map_err(|e| CommandError::new(ErrorCode::Runtime, e.to_string()))?;
    UnixStream::connect(&socket).await.map_err(|e| {
        CommandError::new(ErrorCode::Runtime, format!("proxy socket unavailable: {e}"))
    })
}

/// Removes the host-side sockets of a deleted workspace.
pub async fn remove_sockets(app: &App, name: InstanceName) {
    let dir = app.config.runtime_dir.join("proxy");
    let prefix = format!("{name}-");
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}
