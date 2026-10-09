//! `iglu`: create, inspect and manage workspaces from a terminal or an agent.
//!
//! Every command accepts `--json` for machine-readable output and exits
//! nonzero on failure, so agents can drive it.

mod attach;
mod client;
mod login;

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use iglu_api::{
    Condition, CreateEnvironment, CreateProject, CreateWorkspace, ProjectView, PublishPort,
    PutSecret, RenameWorkspace, RevisionStatus, RouteView, SetDesiredState, WorkspaceView,
};
use iglu_domain::agent::Prompt;
use iglu_domain::env::{EnvName, EnvSource};
use iglu_domain::git::{Unpushed, Unsaved};
use iglu_domain::id::WorkspaceId;
use iglu_domain::label::{AgentName, ProjectName, WorkspaceName};
use iglu_domain::lifecycle::{DesiredState, Phase};
use iglu_domain::port::GuestPort;
use iglu_domain::project::Origin;
use iglu_domain::repo::{BranchName, GitHost, RepoUrl};
use iglu_domain::secret::{
    EnvVarName, GitUsername, HomePath, SecretName, SecretTarget, SecretValue,
};
use iglu_domain::terminal::SessionName;
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
    /// Manage projects: what workspaces clone and how they start.
    #[command(subcommand)]
    Project(ProjectCommand),
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
    /// Create a workspace in a project.
    New {
        /// The project's name; `general` has no repository.
        project: String,
        /// What to work on. Starts an agent with it and names the workspace.
        prompt: Option<Prompt>,
        /// The agent to start, if not the project's or the environment's first.
        #[arg(long)]
        agent: Option<AgentName>,
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
    /// Open one of a workspace's columns in this terminal; Ctrl-] detaches.
    Attach {
        workspace: String,
        /// Defaults to the first open column.
        column: Option<SessionName>,
    },
    /// Rename a workspace. Its branch, files and previews stay as they are.
    Rename {
        workspace: String,
        name: WorkspaceName,
    },
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
    /// Delete a workspace and everything in it. Refuses while it has
    /// uncommitted or unpushed work, or can't be checked, unless forced.
    Rm {
        workspace: String,
        #[arg(long)]
        wait: bool,
        /// Delete even if work would be lost.
        #[arg(long)]
        force: bool,
    },
    /// Publish a guest port at its own preview URL.
    Port { workspace: String, port: GuestPort },
    /// List a workspace's published ports.
    Ports { workspace: String },
    /// Show what happened to a workspace.
    Log { workspace: String },
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// List projects.
    Ls,
    /// Add a project. Its name defaults to the repository's.
    Add {
        name: Option<ProjectName>,
        /// The repository its workspaces clone. Without one they start empty.
        #[arg(long)]
        repo: Option<RepoUrl>,
        #[arg(long, default_value = "default")]
        env: EnvName,
    },
    /// Remove a project without workspaces.
    Rm { project: String },
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
    let branch = ws
        .checkout
        .as_ref()
        .map_or_else(String::new, |checkout| checkout.branch.to_string());
    let mut lines = vec![format!("{:<24} {:<10} {branch}", ws.name, ws.phase)];
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
                expected_revision: ws.revision,
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
        Command::Project(command) => project(&Client::from_stored()?, json_mode, command).await?,
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
            let projects = if json_mode {
                Vec::new()
            } else {
                client.projects().await?
            };
            print(json_mode, list.as_slice(), |list| {
                by_project(list, &projects)
            });
        }
        WorkspaceCommand::New {
            project,
            prompt,
            agent,
            branch,
            base,
            name,
            wait,
        } => {
            let project = client.resolve_project(&project).await?;
            let request = CreateWorkspace {
                project: project.id,
                prompt,
                agent,
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
        WorkspaceCommand::Attach { workspace, column } => {
            attach::run(client, &workspace, column).await?;
        }
        WorkspaceCommand::Rename { workspace, name } => {
            let ws = client.resolve(&workspace).await?;
            let ws = client.rename(ws.id, &RenameWorkspace { name }).await?;
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
        WorkspaceCommand::Rm {
            workspace,
            wait,
            force,
        } => {
            if !force {
                let ws = client.resolve(&workspace).await?;
                check_nothing_lost(client, &ws).await?;
            }
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

/// Refuses when deleting would lose work, or when that can't be told.
async fn check_nothing_lost(client: &Client, ws: &WorkspaceView) -> anyhow::Result<()> {
    if ws.checkout.is_none() {
        return Ok(());
    }
    if ws.phase != Phase::Running {
        bail!(
            "{} is {}, so its Git state can't be checked; start it to check, or pass --force",
            ws.name,
            ws.phase
        );
    }
    let live = client
        .live(ws.id)
        .await
        .context("couldn't check for unsaved work; pass --force to delete anyway")?;
    if let Some(unsaved) = live.unsaved {
        bail!(
            "{} has {}; push it first, or pass --force",
            ws.name,
            unsaved_text(unsaved)
        );
    }
    Ok(())
}

fn unsaved_text(unsaved: Unsaved) -> String {
    let files = match unsaved.uncommitted {
        0 => None,
        1 => Some("1 uncommitted file".to_owned()),
        n => Some(format!("{n} uncommitted files")),
    };
    let commits = match unsaved.unpushed {
        Unpushed::None => None,
        Unpushed::Commits { count: 1 } => Some("1 unpushed commit".to_owned()),
        Unpushed::Commits { count } => Some(format!("{count} unpushed commits")),
        Unpushed::NoUpstream => Some("a branch that was never pushed".to_owned()),
    };
    [files, commits]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" and ")
}

/// Workspaces under their projects' names.
fn by_project(list: &[WorkspaceView], projects: &[ProjectView]) -> String {
    if list.is_empty() {
        return "no workspaces".into();
    }
    projects
        .iter()
        .filter_map(|project| {
            let inside: Vec<&WorkspaceView> =
                list.iter().filter(|ws| ws.project == project.id).collect();
            (!inside.is_empty()).then(|| {
                let mut block = vec![project.name.to_string()];
                block.extend(inside.iter().map(|ws| indent(&describe(ws))));
                block.join("\n")
            })
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn describe_project(project: &ProjectView) -> String {
    let repo = project
        .repo
        .as_ref()
        .map_or_else(|| "no repository".to_owned(), ToString::to_string);
    let builtin = match project.origin {
        Origin::Builtin => "  (built in)",
        Origin::Added => "",
    };
    format!(
        "{:<20} {:<12} {repo}{builtin}",
        project.name, project.environment
    )
}

async fn project(client: &Client, json_mode: bool, command: ProjectCommand) -> anyhow::Result<()> {
    match command {
        ProjectCommand::Ls => {
            let projects = client.projects().await?;
            print(json_mode, projects.as_slice(), |projects| {
                if projects.is_empty() {
                    "no projects; add an environment first with `iglu env add`".into()
                } else {
                    lines(projects, describe_project)
                }
            });
        }
        ProjectCommand::Add { name, repo, env } => {
            let added = client
                .create_project(&CreateProject {
                    name,
                    repo,
                    environment: env,
                })
                .await?;
            print(json_mode, &added, describe_project);
        }
        ProjectCommand::Rm { project } => {
            let found = client.resolve_project(&project).await?;
            client.delete_project(found.id).await?;
            print(json_mode, &json!({ "removed": found.name }), |_| {
                format!("removed {}", found.name)
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
