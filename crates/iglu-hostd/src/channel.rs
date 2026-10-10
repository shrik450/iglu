//! Each workspace's channel to iglu: the only way in from a guest.
//!
//! A runtime connects a socket inside the instance to a path hostd listens
//! on, one per instance, so whatever arrives on an instance's listener came
//! from that workspace and nowhere else. hostd doesn't read what it carries:
//! it holds each connection until iglud asks for the next one, and hands it
//! over as a tunnel labelled with the workspace. iglud then serves its API
//! on it as that workspace. So iglud authenticates the host the way it
//! always does, and needs nothing new to trust a guest.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use iglu_domain::id::{InstanceName, WorkspaceId};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Connections waiting for iglud. More wait in their guest's backlog.
const WAITING: usize = 32;

pub struct Channels {
    listening: Mutex<HashMap<InstanceName, JoinHandle<()>>>,
    sender: mpsc::Sender<(WorkspaceId, UnixStream)>,
    receiver: tokio::sync::Mutex<mpsc::Receiver<(WorkspaceId, UnixStream)>>,
}

impl Default for Channels {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel(WAITING);
        Self {
            listening: Mutex::new(HashMap::new()),
            sender,
            receiver: tokio::sync::Mutex::new(receiver),
        }
    }
}

impl Channels {
    /// Listens for each of `instances` at the path the runtime gives it, and
    /// stops listening for the instances that are gone.
    pub fn listen(&self, instances: impl IntoIterator<Item = (InstanceName, PathBuf)>) {
        let mut listening = self
            .listening
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let wanted: HashMap<InstanceName, PathBuf> = instances.into_iter().collect();
        listening.retain(|name, task| {
            let keep = wanted.contains_key(name) && !task.is_finished();
            if !keep {
                task.abort();
            }
            keep
        });
        for (name, path) in wanted {
            if listening.contains_key(&name) {
                continue;
            }
            match bind(&path) {
                Ok(listener) => {
                    let sender = self.sender.clone();
                    listening.insert(
                        name,
                        tokio::spawn(accept(listener, name.workspace(), sender)),
                    );
                }
                Err(error) => {
                    tracing::warn!(%name, path = %path.display(), %error, "couldn't listen for the workspace's channel");
                }
            }
        }
    }

    /// The next connection a workspace made, if one comes within `wait`.
    pub async fn next(&self, wait: Duration) -> Option<(WorkspaceId, UnixStream)> {
        let mut receiver = self.receiver.lock().await;
        tokio::time::timeout(wait, receiver.recv())
            .await
            .ok()
            .flatten()
    }
}

fn bind(path: &PathBuf) -> std::io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Left by an earlier hostd; nothing listens on it now.
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    UnixListener::bind(path)
}

async fn accept(
    listener: UnixListener,
    workspace: WorkspaceId,
    sender: mpsc::Sender<(WorkspaceId, UnixStream)>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                if sender.send((workspace, stream)).await.is_err() {
                    return;
                }
            }
            Err(error) => {
                tracing::debug!(%workspace, %error, "channel accept failed");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}
