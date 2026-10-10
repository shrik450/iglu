//! `iglu`: create, inspect and manage workspaces from a terminal or an agent.
//!
//! Every command accepts `--json` for machine-readable output and exits
//! nonzero on failure, so agents can drive it.
//!
//! Inside a workspace it acts as that workspace, through its channel: on
//! its own columns, and on whatever else its owner granted it.

mod attach;
mod client;
mod login;

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use iglu_api::{
    AccessGrant, AddColumn, Condition, CreateEnvironment, CreateProject, CreateWorkspace, Identity,
    ProjectView, PublishPort, PutAccess, PutSecret, RenameWorkspace, RevisionStatus, RouteView,
    SendInput, SetDesiredState, WorkspaceView,
};
use iglu_domain::agent::Prompt;
use iglu_domain::auth::Permission;
use iglu_domain::column::{Arg, Argv, ColumnKind};
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
use iglu_domain::terminal::{OutputLines, SessionName};
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
    /// Say who iglu takes you for: you, or the workspace you're in.
    Whoami,
    #[command(flatten)]
    Workspace(WorkspaceCommand),
    /// Work with a workspace's columns. Inside a workspace, `--workspace`
    /// defaults to it.
    #[command(subcommand)]
    Column(ColumnCommand),
    /// What a workspace may do from inside, with `iglu`.
    #[command(subcommand)]
    Access(AccessCommand),
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
enum ColumnCommand {
    /// List columns, and whether each is open.
    Ls {
        #[arg(long, short)]
        workspace: Option<String>,
    },
    /// Open a column: a shell, an agent, or `-- <command>` to run as a server.
    New {
        #[arg(long, short)]
        workspace: Option<String>,
        /// Run this agent from the workspace's environment.
        #[arg(long)]
        agent: Option<AgentName>,
        /// What the agent starts on.
        #[arg(long, requires = "agent", conflicts_with = "prompt_file")]
        prompt: Option<Prompt>,
        /// Read what the agent starts on from this file.
        #[arg(long, requires = "agent")]
        prompt_file: Option<std::path::PathBuf>,
        /// Defaults to one from its kind, such as `shell-2`.
        #[arg(long)]
        name: Option<SessionName>,
        /// The column to put it after; the end by default.
        #[arg(long)]
        after: Option<SessionName>,
        /// A command to run as a server column.
        #[arg(last = true, conflicts_with = "agent")]
        command: Vec<Arg>,
    },
    /// End a column's session and everything in it, and remove the column.
    Close {
        column: SessionName,
        #[arg(long, short)]
        workspace: Option<String>,
    },
    /// Start a column's program again.
    Restart {
        column: SessionName,
        #[arg(long, short)]
        workspace: Option<String>,
    },
    /// Print what a column's terminal shows: its last lines, history included.
    Output {
        column: SessionName,
        #[arg(long, short)]
        workspace: Option<String>,
        #[arg(long, default_value_t = OutputLines::DEFAULT)]
        lines: OutputLines,
    },
    /// Type into a column, exactly as given: add --enter to run it, or
    /// --paste --enter to give an agent a message. The text comes from
    /// stdin when it isn't an argument.
    Send {
        column: SessionName,
        text: Option<String>,
        #[arg(long, short)]
        workspace: Option<String>,
        /// Paste it, as a terminal does: programs that take pastes, like
        /// agents, take it whole, newlines and all.
        #[arg(long)]
        paste: bool,
        /// Press Enter after it.
        #[arg(long)]
        enter: bool,
    },
}

#[derive(Subcommand)]
enum AccessCommand {
    /// Show what a workspace may do, and to which workspaces.
    Show { workspace: String },
    /// Set what a workspace may do to one workspace, itself unless `--on`
    /// says another, replacing what it could do there before. Nothing
    /// after `--allow` takes it all away.
    Set {
        workspace: String,
        #[arg(long)]
        on: Option<String>,
        /// Any of `view`, `read_output`, `send_input`, `manage_columns`,
        /// `publish_routes` and `operate`, separated by commas.
        #[arg(long, value_delimiter = ',', num_args = 0..)]
        allow: Vec<Permission>,
    },
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
        Command::Whoami => whoami(&Client::connect()?, json_mode).await?,
        Command::Workspace(command) => {
            workspace(&Client::connect()?, json_mode, command).await?;
        }
        Command::Column(command) => column(&Client::connect()?, json_mode, command).await?,
        Command::Access(command) => access(&Client::connect()?, json_mode, command).await?,
        Command::Project(command) => project(&Client::connect()?, json_mode, command).await?,
        Command::Env(command) => env(&Client::connect()?, json_mode, command).await?,
        Command::Secret(command) => secret(&Client::connect()?, json_mode, command).await?,
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
            // Projects are a person's; a workspace sees only workspaces.
            let projects = if json_mode || client.inside() {
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

async fn whoami(client: &Client, json_mode: bool) -> anyhow::Result<()> {
    let identity = client.identity().await?;
    print(json_mode, &identity, |identity| match identity {
        Identity::Person { name, email, .. } => match (name, email) {
            (Some(name), Some(email)) => format!("{name} <{email}>"),
            (Some(name), None) => name.to_string(),
            (None, Some(email)) => email.to_string(),
            (None, None) => "signed in".into(),
        },
        Identity::Workspace { name, .. } => format!("the workspace {name}"),
    });
    Ok(())
}

/// The workspace a command names, or inside a workspace, that one.
async fn target(client: &Client, reference: Option<&str>) -> anyhow::Result<WorkspaceView> {
    if let Some(reference) = reference {
        return client.resolve(reference).await;
    }
    if !client.inside() {
        bail!("say which workspace with --workspace");
    }
    let Identity::Workspace { id, .. } = client.identity().await? else {
        bail!("iglu doesn't take this for a workspace");
    };
    client
        .workspace(id)
        .await?
        .context("this workspace can't see itself; its owner can grant it `view`")
}

async fn column(client: &Client, json_mode: bool, command: ColumnCommand) -> anyhow::Result<()> {
    match command {
        ColumnCommand::Ls { workspace } => {
            let ws = target(client, workspace.as_deref()).await?;
            let columns = client.columns(ws.id).await?;
            print(json_mode, columns.as_slice(), |columns| {
                lines(columns, |c| format!("{:<20} {}", c.name, c.state))
            });
        }
        ColumnCommand::New {
            workspace,
            agent,
            prompt,
            prompt_file,
            name,
            after,
            command,
        } => {
            let ws = target(client, workspace.as_deref()).await?;
            let prompt = match prompt_file {
                Some(path) => Some(
                    std::fs::read_to_string(&path)
                        .with_context(|| format!("reading {}", path.display()))?
                        .parse::<Prompt>()?,
                ),
                None => prompt,
            };
            let kind = match (agent, command.is_empty()) {
                (Some(agent), _) => ColumnKind::Agent { agent },
                (None, true) => ColumnKind::Shell,
                (None, false) => ColumnKind::Server {
                    command: Argv::try_from(command)?,
                },
            };
            let spec = client
                .add_column(
                    ws.id,
                    &AddColumn {
                        kind,
                        name,
                        width: None,
                        after,
                        prompt,
                    },
                )
                .await?;
            print(json_mode, &spec, |spec| spec.name.to_string());
        }
        ColumnCommand::Close { column, workspace } => {
            let ws = target(client, workspace.as_deref()).await?;
            client.close_column(ws.id, &column).await?;
            print(json_mode, &json!({ "closed": column }), |_| {
                format!("closed {column}")
            });
        }
        ColumnCommand::Restart { column, workspace } => {
            let ws = target(client, workspace.as_deref()).await?;
            client.restart_column(ws.id, &column).await?;
            print(json_mode, &json!({ "restarted": column }), |_| {
                format!("restarted {column}")
            });
        }
        ColumnCommand::Output {
            column,
            workspace,
            lines,
        } => {
            let ws = target(client, workspace.as_deref()).await?;
            let output = client.output(ws.id, &column, lines).await?;
            print(json_mode, &output, |output| output.text.clone());
        }
        ColumnCommand::Send {
            column,
            text,
            workspace,
            paste,
            enter,
        } => {
            let ws = target(client, workspace.as_deref()).await?;
            send(client, ws.id, &column, text, paste, enter).await?;
            print(json_mode, &json!({ "sent": column }), |_| {
                format!("sent to {column}")
            });
        }
    }
    Ok(())
}

/// Types into a column: the text given, or stdin, then Enter if asked.
async fn send(
    client: &Client,
    id: WorkspaceId,
    column: &SessionName,
    text: Option<String>,
    paste: bool,
    enter: bool,
) -> anyhow::Result<()> {
    let text = if let Some(text) = text {
        text
    } else {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading what to send from stdin")?;
        text
    };
    let text = (!text.is_empty())
        .then(|| {
            if paste {
                format!("\x1b[200~{text}\x1b[201~")
            } else {
                text
            }
            .parse()
        })
        .transpose()?;
    client
        .send_input(id, column, &SendInput { text, enter })
        .await
}

async fn access(client: &Client, json_mode: bool, command: AccessCommand) -> anyhow::Result<()> {
    let all = client.workspaces().await?;
    let name_of = |id: WorkspaceId| {
        all.iter()
            .find(|ws| ws.id == id)
            .map_or_else(|| id.to_string(), |ws| ws.name.to_string())
    };
    let describe_access = |ws: &WorkspaceView| {
        if ws.access.is_empty() {
            return format!("{} may do nothing from inside", ws.name);
        }
        lines(&ws.access, |grant| {
            let permissions: Vec<&str> = grant.permissions.iter().map(|p| p.as_str()).collect();
            format!(
                "{:<24} {}",
                name_of(grant.workspace),
                permissions.join(", ")
            )
        })
    };
    match command {
        AccessCommand::Show { workspace } => {
            let ws = client.resolve(&workspace).await?;
            print(json_mode, &ws.access, |_| describe_access(&ws));
        }
        AccessCommand::Set {
            workspace,
            on,
            allow,
        } => {
            let ws = client.resolve(&workspace).await?;
            let on = match on {
                Some(on) => client.resolve(&on).await?.id,
                None => ws.id,
            };
            let mut grants: Vec<AccessGrant> = ws
                .access
                .into_iter()
                .filter(|g| g.workspace != on)
                .collect();
            if !allow.is_empty() {
                grants.push(AccessGrant {
                    workspace: on,
                    permissions: allow,
                });
            }
            let ws = client.put_access(ws.id, &PutAccess { grants }).await?;
            print(json_mode, &ws.access, |_| describe_access(&ws));
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
    if projects.is_empty() {
        return lines(list, describe);
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
