//! `iglu`: create, inspect and manage workspaces from a terminal or an agent.
//!
//! Every command accepts `--json` for machine-readable output and exits
//! nonzero on failure, so agents can drive it.

mod client;
mod login;

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use iglu_domain::env::{EnvName, EnvSource};
use iglu_domain::label::WorkspaceName;
use iglu_domain::port::GuestPort;
use iglu_domain::repo::{BranchName, GitHost, RepoUrl};
use iglu_domain::secret::{EnvVarName, GitUsername, HomePath, SecretName};
use serde_json::{Value, json};

use crate::client::Client;

#[derive(Parser)]
#[command(name = "iglu", about = "Workspaces for coding agents", version)]
struct Cli {
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in through the browser.
    Login {
        /// The console URL, such as https://iglu.example.org.
        server: url::Url,
    },
    /// Forget the stored credentials.
    Logout,
    /// List workspaces.
    Ls,
    /// Create a workspace from a repository.
    New {
        repo: RepoUrl,
        #[arg(long, default_value = "default")]
        env: EnvName,
        /// The branch to work on. Defaults to the workspace name.
        #[arg(long)]
        branch: Option<BranchName>,
        /// Where a new branch starts. Defaults to the remote's default branch.
        #[arg(long)]
        base: Option<BranchName>,
        #[arg(long)]
        name: Option<WorkspaceName>,
        /// Wait until the workspace is running.
        #[arg(long)]
        wait: bool,
    },
    /// Show one workspace.
    Show { workspace: String },
    /// Start or thaw a workspace.
    Start {
        workspace: String,
        #[arg(long)]
        wait: bool,
    },
    /// Freeze a running workspace. Its memory moves to swap; a host restart stops it.
    Freeze {
        workspace: String,
        #[arg(long)]
        wait: bool,
    },
    /// Stop a workspace. Files stay; processes end.
    Stop {
        workspace: String,
        #[arg(long)]
        wait: bool,
    },
    /// Delete a workspace and everything in it.
    Rm {
        workspace: String,
        #[arg(long)]
        wait: bool,
    },
    /// Publish a guest port at its own preview URL.
    Port { workspace: String, port: GuestPort },
    /// List a workspace's published ports.
    Ports { workspace: String },
    /// Show what happened to a workspace.
    Log { workspace: String },
    /// Manage environments.
    #[command(subcommand)]
    Env(EnvCommand),
    /// Manage secrets delivered to your workspaces.
    #[command(subcommand)]
    Secret(SecretCommand),
}

#[derive(Subcommand)]
enum EnvCommand {
    /// List environments.
    Ls,
    /// Add an environment from `<flake>#<nixosConfigurations attribute>` and build it.
    Add { name: EnvName, source: EnvSource },
    /// Rebuild an environment's image from its flake.
    Build { name: EnvName },
}

#[derive(Subcommand)]
enum SecretCommand {
    /// List secrets (names and destinations, never values).
    Ls,
    /// Store a secret, reading its value from stdin.
    Set {
        name: SecretName,
        /// Export it as this environment variable in every terminal.
        #[arg(long, group = "target")]
        env: Option<EnvVarName>,
        /// Write it to this path under the home directory.
        #[arg(long, group = "target")]
        file: Option<HomePath>,
        /// Answer Git's credential requests for this host.
        #[arg(long, group = "target", requires = "username")]
        git: Option<GitHost>,
        #[arg(long)]
        username: Option<GitUsername>,
    },
    /// Delete a secret.
    Rm { name: SecretName },
}

fn print(json_mode: bool, value: &Value, text: impl FnOnce(&Value) -> String) {
    if json_mode {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        );
    } else {
        println!("{}", text(value));
    }
}

fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn describe(ws: &Value) -> String {
    let mut line = format!(
        "{:<24} {:<10} {}  {}",
        s(ws, "name"),
        s(ws, "phase"),
        s(ws, "branch"),
        s(ws, "repo")
    );
    if let Some(attention) = ws.get("attention").filter(|a| !a.is_null()) {
        line.push_str(&format!(
            "\n  {}: {} {}",
            s(attention, "session"),
            s(attention, "state"),
            s(attention, "summary")
        ));
    }
    if let Some(condition) = ws.get("condition").filter(|c| !c.is_null()) {
        line.push_str(&format!("\n  ! {}", condition_text(condition)));
    }
    for route in ws
        .get("routes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        line.push_str(&format!(
            "\n  :{} → {}",
            route.get("port").unwrap_or(&Value::Null),
            s(route, "url")
        ));
    }
    line
}

fn condition_text(condition: &Value) -> String {
    match s(condition, "kind") {
        "error" => format!("{} ({})", s(condition, "message"), s(condition, "code")),
        "capacity" => "waiting for the host to have room".into(),
        "runtime_failed" => "the runtime reports the workspace as broken; stop or delete it".into(),
        "host_offline" => "the host isn't answering".into(),
        other => other.to_owned(),
    }
}

/// Polls until the workspace reaches `phase` (or disappears, for deletion).
async fn wait_for(client: &Client, id: &str, phase: &str) -> anyhow::Result<Value> {
    let started = Instant::now();
    loop {
        match client.get(&format!("/v1/workspaces/{id}")).await {
            Ok(ws) => {
                if s(&ws, "phase") == phase {
                    return Ok(ws);
                }
                if let Some(condition) = ws
                    .get("condition")
                    .filter(|c| s(c, "kind") == "error" || s(c, "kind") == "runtime_failed")
                {
                    bail!("{}", condition_text(condition));
                }
            }
            Err(error) if phase == "deleted" && error.to_string().contains("not found") => {
                return Ok(json!({ "phase": "deleted" }));
            }
            Err(error) => return Err(error),
        }
        if started.elapsed() > Duration::from_secs(1800) {
            bail!("timed out waiting for the workspace to be {phase}");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn set_state(
    client: &Client,
    reference: &str,
    state: &str,
    wait: bool,
    target: &str,
) -> anyhow::Result<Value> {
    let ws = client.resolve(reference).await?;
    let id = s(&ws, "id").to_owned();
    let updated = client
        .put(
            &format!("/v1/workspaces/{id}/desired-state"),
            &json!({ "state": state, "expected_revision": ws["revision"] }),
        )
        .await?;
    if wait {
        wait_for(client, &id, target).await
    } else {
        Ok(updated)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("a TLS provider was already installed"))?;
    let cli = Cli::parse();
    let json_mode = cli.json;

    if let Command::Login { server } = &cli.command {
        login::login(server).await?;
        println!("signed in to {server}");
        return Ok(());
    }
    if let Command::Logout = cli.command {
        login::logout()?;
        println!("signed out");
        return Ok(());
    }
    let client = Client::from_stored()?;

    match cli.command {
        Command::Login { .. } | Command::Logout => unreachable!("handled above"),
        Command::Ls => {
            let list = client.get("/v1/workspaces").await?;
            print(json_mode, &list, |list| {
                let items: Vec<String> = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(describe)
                    .collect();
                if items.is_empty() {
                    "no workspaces".into()
                } else {
                    items.join("\n")
                }
            });
        }
        Command::New {
            repo,
            env,
            branch,
            base,
            name,
            wait,
        } => {
            let body = json!({ "environment": env, "repo": repo, "branch": branch, "base": base, "name": name });
            let key = uuid::Uuid::new_v4().to_string();
            let ws = client
                .post_idempotent("/v1/workspaces", &body, &key)
                .await?;
            let ws = if wait {
                wait_for(&client, s(&ws, "id"), "running").await?
            } else {
                ws
            };
            print(json_mode, &ws, describe);
        }
        Command::Show { workspace } => {
            let ws = client.resolve(&workspace).await?;
            print(json_mode, &ws, describe);
        }
        Command::Start { workspace, wait } => {
            let ws = set_state(&client, &workspace, "running", wait, "running").await?;
            print(json_mode, &ws, describe);
        }
        Command::Freeze { workspace, wait } => {
            let ws = set_state(&client, &workspace, "frozen", wait, "frozen").await?;
            print(json_mode, &ws, describe);
        }
        Command::Stop { workspace, wait } => {
            let ws = set_state(&client, &workspace, "stopped", wait, "stopped").await?;
            print(json_mode, &ws, describe);
        }
        Command::Rm { workspace, wait } => {
            let ws = set_state(&client, &workspace, "deleted", wait, "deleted").await?;
            print(json_mode, &ws, |_| "deleting".into());
        }
        Command::Port { workspace, port } => {
            let ws = client.resolve(&workspace).await?;
            let route = client
                .post(
                    &format!("/v1/workspaces/{}/routes", s(&ws, "id")),
                    &json!({ "port": port }),
                )
                .await?;
            print(json_mode, &route, |r| s(r, "url").to_owned());
        }
        Command::Ports { workspace } => {
            let ws = client.resolve(&workspace).await?;
            let routes = client
                .get(&format!("/v1/workspaces/{}/routes", s(&ws, "id")))
                .await?;
            print(json_mode, &routes, |routes| {
                routes
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| {
                        format!(
                            ":{} → {}",
                            r.get("port").unwrap_or(&Value::Null),
                            s(r, "url")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        Command::Log { workspace } => {
            let ws = client.resolve(&workspace).await?;
            let log = client
                .get(&format!("/v1/workspaces/{}/activity", s(&ws, "id")))
                .await?;
            print(json_mode, &log, |log| {
                log.as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| format!("{:<18} {}", s(e, "kind"), s(e, "detail")))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        Command::Env(EnvCommand::Ls) => {
            let envs = client.get("/v1/environments").await?;
            print(json_mode, &envs, |envs| {
                envs.as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        let status = e.get("latest").map_or("never built", |l| s(l, "status"));
                        format!("{:<16} {:<12} {}", s(e, "name"), status, s(e, "source"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        Command::Env(EnvCommand::Add { name, source }) => {
            let reply = client
                .post(
                    "/v1/environments",
                    &json!({ "name": name, "source": source }),
                )
                .await?;
            print(json_mode, &reply, |_| {
                format!("building {name}; check with `iglu env ls`")
            });
        }
        Command::Env(EnvCommand::Build { name }) => {
            let reply = client
                .post(&format!("/v1/environments/{name}/builds"), &json!({}))
                .await?;
            print(json_mode, &reply, |_| format!("rebuilding {name}"));
        }
        Command::Secret(SecretCommand::Ls) => {
            let secrets = client.get("/v1/secrets").await?;
            print(json_mode, &secrets, |list| {
                list.as_array()
                    .into_iter()
                    .flatten()
                    .map(|secret| {
                        format!(
                            "{:<24} {}",
                            s(secret, "name"),
                            secret.get("target").unwrap_or(&Value::Null)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        Command::Secret(SecretCommand::Set {
            name,
            env,
            file,
            git,
            username,
        }) => {
            // Files are stored exactly as read. Single-line values lose the
            // newline that `echo` or a text file adds.
            let (target, single_line) = match (env, file, git, username) {
                (Some(name), None, None, _) => (json!({ "kind": "env", "name": name }), true),
                (None, Some(path), None, _) => (json!({ "kind": "file", "path": path }), false),
                (None, None, Some(host), Some(username)) => (
                    json!({ "kind": "git_credential", "host": host, "username": username }),
                    true,
                ),
                _ => bail!("choose exactly one of --env, --file or --git (with --username)"),
            };
            let mut value = String::new();
            std::io::stdin()
                .read_to_string(&mut value)
                .context("reading the secret from stdin")?;
            let value = if single_line {
                value.trim_end_matches(['\n', '\r'])
            } else {
                value.as_str()
            };
            client
                .put(
                    &format!("/v1/secrets/{name}"),
                    &json!({ "target": target, "value": value }),
                )
                .await?;
            print(json_mode, &json!({ "name": name }), |_| {
                format!("stored {name}; running workspaces receive it shortly")
            });
        }
        Command::Secret(SecretCommand::Rm { name }) => {
            client.delete(&format!("/v1/secrets/{name}")).await?;
            print(json_mode, &json!({ "name": name }), |_| {
                format!("deleted {name}")
            });
        }
    }
    Ok(())
}
