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
use iglu_api::{
    Condition, CreateEnvironment, CreateWorkspace, PublishPort, PutSecret, RevisionStatus,
    RouteView, SetDesiredState, WorkspaceView,
};
use iglu_domain::env::{EnvName, EnvSource};
use iglu_domain::id::WorkspaceId;
use iglu_domain::label::WorkspaceName;
use iglu_domain::lifecycle::DesiredState;
use iglu_domain::port::GuestPort;
use iglu_domain::repo::{BranchName, GitHost, RepoUrl};
use iglu_domain::secret::{
    EnvVarName, GitUsername, HomePath, SecretName, SecretTarget, SecretValue,
};
use serde::Serialize;
use serde_json::json;

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
        /// The console URL, such as <https://iglu.example.org>.
        server: url::Url,
    },
    /// Forget the stored credentials.
    Logout,
    #[command(flatten)]
    Workspace(WorkspaceCommand),
    /// Manage environments.
    #[command(subcommand)]
    Env(EnvCommand),
    /// Manage secrets delivered to your workspaces.
    #[command(subcommand)]
    Secret(SecretCommand),
}

#[derive(Subcommand)]
enum WorkspaceCommand {
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

fn print<T: Serialize + ?Sized>(json_mode: bool, value: &T, text: impl FnOnce(&T) -> String) {
    if json_mode {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        );
    } else {
        println!("{}", text(value));
    }
}

fn describe(ws: &WorkspaceView) -> String {
    let mut lines = vec![format!(
        "{:<24} {:<10} {}  {}",
        ws.name, ws.phase, ws.branch, ws.repo
    )];
    if let Some(attention) = &ws.attention {
        lines.push(format!(
            "  {}: {} {}",
            attention.session, attention.state, attention.summary
        ));
    }
    if let Some(condition) = &ws.condition {
        lines.push(format!("  ! {}", condition_text(condition)));
    }
    lines.extend(
        ws.routes
            .iter()
            .map(|route| format!("  {}", route_line(route))),
    );
    lines.join("\n")
}

fn route_line(route: &RouteView) -> String {
    format!(":{} → {}", route.port, route.url)
}

fn condition_text(condition: &Condition) -> String {
    match condition {
        Condition::Error { message, .. } => message.clone(),
        Condition::Capacity { available, needed } => {
            format!("waiting for the host to have room: {needed} needed, {available} free")
        }
        Condition::RuntimeFailed => {
            "the runtime reports the workspace as broken; stop or delete it".into()
        }
        Condition::HostOffline { .. } => "the host isn't answering".into(),
    }
}

/// Polls until the workspace settles in `desired`. `None` means it's gone,
/// which is how a deletion finishes.
async fn wait_for(
    client: &Client,
    id: WorkspaceId,
    desired: DesiredState,
) -> anyhow::Result<Option<WorkspaceView>> {
    let started = Instant::now();
    loop {
        match client.workspace(id).await? {
            None if desired == DesiredState::Deleted => return Ok(None),
            None => bail!("the workspace was deleted"),
            Some(ws) if ws.phase == desired.settled() => return Ok(Some(ws)),
            Some(ws) => match &ws.condition {
                Some(condition @ (Condition::Error { .. } | Condition::RuntimeFailed)) => {
                    bail!("{}", condition_text(condition));
                }
                Some(Condition::Capacity { .. } | Condition::HostOffline { .. }) | None => {}
            },
        }
        if started.elapsed() > Duration::from_secs(1800) {
            bail!("timed out waiting for the workspace to be {desired}");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Requests `state` and prints the workspace, with `wait` once it gets there.
async fn transition(
    client: &Client,
    json_mode: bool,
    reference: &str,
    state: DesiredState,
    wait: bool,
) -> anyhow::Result<()> {
    let ws = client.resolve(reference).await?;
    let updated = client
        .set_desired_state(
            ws.id,
            &SetDesiredState {
                state,
                expected_revision: Some(ws.revision),
            },
        )
        .await?;
    let ws = if wait {
        wait_for(client, ws.id, state).await?
    } else {
        Some(updated)
    };
    print(json_mode, &ws, |ws| {
        ws.as_ref().map_or_else(|| "deleted".into(), describe)
    });
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("a TLS provider was already installed"))?;
    let cli = Cli::parse();
    let json_mode = cli.json;

    match cli.command {
        Command::Login { server } => {
            login::login(&server).await?;
            println!("signed in to {server}");
        }
        Command::Logout => {
            login::logout()?;
            println!("signed out");
        }
        Command::Workspace(command) => {
            workspace(&Client::from_stored()?, json_mode, command).await?;
        }
        Command::Env(command) => env(&Client::from_stored()?, json_mode, command).await?,
        Command::Secret(command) => secret(&Client::from_stored()?, json_mode, command).await?,
    }
    Ok(())
}

fn lines<T>(list: &[T], line: impl Fn(&T) -> String) -> String {
    list.iter().map(line).collect::<Vec<_>>().join("\n")
}

async fn workspace(
    client: &Client,
    json_mode: bool,
    command: WorkspaceCommand,
) -> anyhow::Result<()> {
    match command {
        WorkspaceCommand::Ls => {
            let list = client.workspaces().await?;
            print(json_mode, list.as_slice(), |list| {
                if list.is_empty() {
                    "no workspaces".into()
                } else {
                    lines(list, describe)
                }
            });
        }
        WorkspaceCommand::New {
            repo,
            env,
            branch,
            base,
            name,
            wait,
        } => {
            let request = CreateWorkspace {
                environment: env,
                repo,
                branch,
                base,
                name,
            };
            let key = uuid::Uuid::new_v4().to_string();
            let ws = client.create_workspace(&request, &key).await?;
            let ws = if wait {
                wait_for(client, ws.id, DesiredState::Running)
                    .await?
                    .context("the workspace was deleted")?
            } else {
                ws
            };
            print(json_mode, &ws, describe);
        }
        WorkspaceCommand::Show { workspace } => {
            let ws = client.resolve(&workspace).await?;
            print(json_mode, &ws, describe);
        }
        WorkspaceCommand::Start { workspace, wait } => {
            transition(client, json_mode, &workspace, DesiredState::Running, wait).await?;
        }
        WorkspaceCommand::Freeze { workspace, wait } => {
            transition(client, json_mode, &workspace, DesiredState::Frozen, wait).await?;
        }
        WorkspaceCommand::Stop { workspace, wait } => {
            transition(client, json_mode, &workspace, DesiredState::Stopped, wait).await?;
        }
        WorkspaceCommand::Rm { workspace, wait } => {
            transition(client, json_mode, &workspace, DesiredState::Deleted, wait).await?;
        }
        WorkspaceCommand::Port { workspace, port } => {
            let ws = client.resolve(&workspace).await?;
            let route = client.publish(ws.id, &PublishPort { port }).await?;
            print(json_mode, &route, |route| route.url.clone());
        }
        WorkspaceCommand::Ports { workspace } => {
            let ws = client.resolve(&workspace).await?;
            let routes = client.routes(ws.id).await?;
            print(json_mode, routes.as_slice(), |routes| {
                lines(routes, route_line)
            });
        }
        WorkspaceCommand::Log { workspace } => {
            let ws = client.resolve(&workspace).await?;
            let log = client.activity(ws.id).await?;
            print(json_mode, log.as_slice(), |log| {
                lines(log, |entry| format!("{:<18} {}", entry.kind, entry.detail))
            });
        }
    }
    Ok(())
}

async fn env(client: &Client, json_mode: bool, command: EnvCommand) -> anyhow::Result<()> {
    match command {
        EnvCommand::Ls => {
            let envs = client.environments().await?;
            print(json_mode, envs.as_slice(), |envs| {
                lines(envs, |env| {
                    let status =
                        env.latest
                            .as_ref()
                            .map_or("never built", |latest| match latest.status {
                                RevisionStatus::Building => "building",
                                RevisionStatus::Ready { .. } => "ready",
                                RevisionStatus::Failed { .. } => "failed",
                            });
                    format!("{:<16} {:<12} {}", env.name, status, env.source)
                })
            });
        }
        EnvCommand::Add { name, source } => {
            let started = client
                .create_environment(&CreateEnvironment {
                    name: name.clone(),
                    source,
                })
                .await?;
            print(json_mode, &started, |_| {
                format!("building {name}; check with `iglu env ls`")
            });
        }
        EnvCommand::Build { name } => {
            let started = client.build_environment(&name).await?;
            print(json_mode, &started, |_| format!("rebuilding {name}"));
        }
    }
    Ok(())
}

fn target_text(target: &SecretTarget) -> String {
    match target {
        SecretTarget::Env { name } => format!("env {name}"),
        SecretTarget::File { path } => format!("file ~/{path}"),
        SecretTarget::GitCredential { host, username } => format!("git {username}@{host}"),
    }
}

async fn secret(client: &Client, json_mode: bool, command: SecretCommand) -> anyhow::Result<()> {
    match command {
        SecretCommand::Ls => {
            let secrets = client.secrets().await?;
            print(json_mode, secrets.as_slice(), |list| {
                lines(list, |secret| {
                    format!("{:<24} {}", secret.name, target_text(&secret.target))
                })
            });
        }
        SecretCommand::Set {
            name,
            env,
            file,
            git,
            username,
        } => {
            let target = match (env, file, git, username) {
                (Some(name), None, None, _) => SecretTarget::Env { name },
                (None, Some(path), None, _) => SecretTarget::File { path },
                (None, None, Some(host), Some(username)) => {
                    SecretTarget::GitCredential { host, username }
                }
                _ => bail!("choose exactly one of --env, --file or --git (with --username)"),
            };
            let mut value = String::new();
            std::io::stdin()
                .read_to_string(&mut value)
                .context("reading the secret from stdin")?;
            // Files are stored exactly as read. Single-line values lose the
            // newline that `echo` or a text file adds.
            let value = match target {
                SecretTarget::File { .. } => value,
                SecretTarget::Env { .. } | SecretTarget::GitCredential { .. } => {
                    value.trim_end_matches(['\n', '\r']).to_owned()
                }
            };
            let value = SecretValue::try_from(value)?;
            client
                .put_secret(&name, &PutSecret { target, value })
                .await?;
            print(json_mode, &json!({ "name": name }), |_| {
                format!("stored {name}; running workspaces receive it shortly")
            });
        }
        SecretCommand::Rm { name } => {
            client.delete_secret(&name).await?;
            print(json_mode, &json!({ "name": name }), |_| {
                format!("deleted {name}")
            });
        }
    }
    Ok(())
}
