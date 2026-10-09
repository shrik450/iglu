//! Raw byte tunnels to a guest's loopback ports, for the preview gateway.
//!
//! Each published port becomes an Incus proxy device that listens on a
//! host-side Unix socket owned by hostd and connects to `127.0.0.1:<port>`
//! inside the guest. The gateway never gets a route onto the guest network.

use std::path::{Path, PathBuf};
use std::time::Duration;

use iglu_domain::id::InstanceName;
use iglu_domain::port::GuestPort;
use serde_json::json;
use tokio::net::UnixStream;

use super::client::Incus;
use crate::runtime::RuntimeError;

fn device_name(port: GuestPort) -> String {
    format!("iglu-port-{port}")
}

fn socket_path(sockets: &Path, name: InstanceName, port: GuestPort) -> PathBuf {
    sockets.join(format!("{name}-{port}.sock"))
}

/// Connects to a guest port, adding the proxy device on first use.
pub async fn connect(
    incus: &Incus,
    sockets: &Path,
    name: InstanceName,
    port: GuestPort,
    timeout: Duration,
) -> Result<UnixStream, RuntimeError> {
    let socket = socket_path(sockets, name, port);
    if let Ok(stream) = UnixStream::connect(&socket).await {
        return Ok(stream);
    }
    tokio::fs::create_dir_all(sockets)
        .await
        .map_err(RuntimeError::failed)?;
    let listen = format!("unix:{}", socket.display());
    let device = json!({
        "type": "proxy",
        "bind": "host",
        "listen": listen,
        "connect": format!("tcp:127.0.0.1:{port}"),
        "mode": "0600",
    });
    let key = device_name(port);
    incus
        .update_instance(
            name,
            |instance| {
                instance["devices"][key.as_str()] = device;
            },
            timeout,
        )
        .await
        .map_err(RuntimeError::failed)?;
    UnixStream::connect(&socket)
        .await
        .map_err(|e| RuntimeError::Failed(format!("proxy socket unavailable: {e}")))
}

/// Removes the host-side sockets of a deleted workspace.
pub async fn remove_sockets(sockets: &Path, name: InstanceName) {
    let prefix = format!("{name}-");
    let Ok(mut entries) = tokio::fs::read_dir(sockets).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}
