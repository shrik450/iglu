//! Browser sign-in for the CLI: a loopback redirect protected with PKCE,
//! exchanged for an API token.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iglu_api::{CliToken, CliTokenRequest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
pub struct Stored {
    pub server: url::Url,
    pub token: String,
}

fn path() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .context("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(base.join("iglu").join("credentials.json"))
}

impl Stored {
    pub fn load() -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(&fs::read(path()?)?)?)
    }

    fn save(&self) -> anyhow::Result<()> {
        let path = path()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        Ok(())
    }
}

pub fn logout() -> anyhow::Result<()> {
    match fs::remove_file(path()?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn random() -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("no randomness available"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Reads the one callback request and returns its query parameters.
fn wait_for_callback(listener: &TcpListener) -> anyhow::Result<Vec<(String, String)>> {
    let (mut stream, _) = listener.accept()?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
    let body = "Signed in. You can close this tab and return to the terminal.";
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(url.query_pairs().into_owned().collect())
}

fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(url).status();
}

pub async fn login(server: &url::Url) -> anyhow::Result<()> {
    let verifier = random()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let mut url = server.join("/auth/cli")?;
    url.query_pairs_mut()
        .append_pair("port", &port.to_string())
        .append_pair("challenge", &challenge)
        .append_pair("state", &state);
    eprintln!("Opening {url}\nIf no browser opens, visit that URL.");
    open_browser(url.as_str());

    let params = tokio::task::spawn_blocking(move || wait_for_callback(&listener)).await??;
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    if get("state").as_deref() != Some(state.as_str()) {
        bail!("the sign-in response didn't match this request");
    }
    let code = get("code").context("the sign-in response had no code")?;

    let http = reqwest::Client::new();
    let response = http
        .post(server.join("/v1/cli/token")?)
        .json(&CliTokenRequest { code, verifier })
        .send()
        .await?;
    if !response.status().is_success() {
        bail!("iglu refused the sign-in ({})", response.status());
    }
    let CliToken { token } = response.json().await.context("reading iglu's reply")?;
    Stored {
        server: server.clone(),
        token,
    }
    .save()
}
