//! The local runtime against the runtime conformance suite, with real
//! processes, the real guest tools, zmx and Git.
//!
//! It needs `IGLU_GUEST_TOOLS`, the directory with a built `iglu-guest`, and
//! zmx on the `PATH`; `just test` and the flake check provide both.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use iglu_devhost::config::Settings;
use iglu_devhost::local::{LocalRuntime, SetupError};
use iglu_domain::capacity::Bytes;
use iglu_domain::guest::INTERFACE;
use iglu_domain::id::{PrincipalId, WorkspaceId};
use iglu_domain::lifecycle::SecretsGeneration;
use iglu_domain::secret::{FetchTokens, SecretTarget, SecretValue, bundle};
use iglu_hostd::config::Timeouts;
use iglu_hostd::conformance::{self, Fixture};
use iglu_hostd::guest::GuestCommand;
use iglu_hostd::host::Host;
use iglu_hostd::runtime::Runtime;
use iglu_proto::{BuildOutcome, Command, CommandOutcome, CreateSpec, ErrorCode, Limits};

const TIMEOUTS: Timeouts = Timeouts {
    operation_secs: 60,
    stop_secs: 5,
    provision_secs: 60,
};

fn settings(name: &str) -> Settings {
    let tools = std::env::var("IGLU_GUEST_TOOLS")
        .expect("IGLU_GUEST_TOOLS names the directory with iglu-guest; just test sets it");
    let state = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&state);
    // Session sockets need a path short enough for a Unix socket, which
    // the target directory isn't.
    let runtime = format!("/tmp/iglu-{name}-{}", std::process::id());
    let _ = std::fs::remove_dir_all(&runtime);
    serde_json::from_value(serde_json::json!({
        "state_dir": state,
        "runtime_dir": runtime,
        "guest_tools": tools,
        "memory_available": 1u64 << 34,
        // The build account in a Nix sandbox has no usable login shell.
        "shell": "/bin/sh",
    }))
    .expect("valid settings")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_local_runtime_conforms() {
    let settings = settings("conformance");
    let runtime = LocalRuntime::new(&settings, TIMEOUTS)
        .await
        .expect("the runtime sets up");
    let host = Host::new(runtime, TIMEOUTS);
    let fixture = Fixture {
        source: "path:/nonexistent#local".parse().expect("valid source"),
        boot_timeout: Duration::from_secs(10),
    };
    let outcomes = conformance::run(&host, &fixture).await;
    for outcome in &outcomes {
        eprintln!("{:?} {}", outcome.result, outcome.check);
    }
    let _ = std::fs::remove_dir_all(settings.runtime_dir.as_path());
    let failed: Vec<_> = outcomes.iter().filter(|o| o.result.is_err()).collect();
    assert!(failed.is_empty(), "failed checks: {failed:#?}");
}

#[tokio::test]
async fn a_runtime_directory_others_can_use_is_refused() {
    use std::os::unix::fs::PermissionsExt;

    let settings = settings("shared");
    let runtime = settings.runtime_dir.as_path();
    std::fs::create_dir_all(runtime).expect("made");
    std::fs::set_permissions(runtime, std::fs::Permissions::from_mode(0o755)).expect("set");
    let refused = LocalRuntime::new(&settings, TIMEOUTS).await;
    let _ = std::fs::remove_dir_all(runtime);
    assert!(matches!(refused, Err(SetupError::Unsafe(_))));
}

/// A guest tool that exits without reading its input must still come back,
/// with its own status, however much input was waiting for it. The suite
/// can't make the real tool do that, so this one stands in for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_program_that_ignores_its_input_still_finishes() {
    use std::os::unix::fs::PermissionsExt;

    let mut settings = settings("unread");
    let tools = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("unread-tools");
    std::fs::create_dir_all(&tools).expect("made");
    let tool = tools.join("iglu-guest");
    std::fs::write(&tool, "#!/bin/sh\nexit 3\n").expect("written");
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("set");
    settings.guest_tools = serde_json::from_value(serde_json::json!(tools)).expect("absolute");

    let runtime = LocalRuntime::new(&settings, TIMEOUTS)
        .await
        .expect("the runtime sets up");
    let host = Host::new(runtime, TIMEOUTS);
    let source = "path:/nonexistent#local".parse().expect("valid source");
    let BuildOutcome::Built(image) = host.build(&source, &FetchTokens::default()).await else {
        panic!("recording the environment failed");
    };
    let workspace = WorkspaceId::from_uuid(uuid::Uuid::new_v4());
    let spec = CreateSpec {
        owner: PrincipalId::from_uuid(uuid::Uuid::new_v4()),
        image: image.fingerprint,
        user: image.user,
        limits: Limits {
            cpus: 1,
            memory: Bytes::new(1 << 30),
            processes: 1024,
            swap: Bytes::new(1 << 30),
        },
    };
    for command in [Command::Create(spec), Command::Start] {
        let outcome = host.perform(workspace, command).await;
        assert!(
            matches!(outcome, CommandOutcome::Done { .. }),
            "{outcome:?}"
        );
    }
    let guest = host
        .guest(workspace.instance_name())
        .await
        .expect("running");
    let big = (0..17).map(|i| {
        (
            SecretTarget::File {
                path: format!(".big-{i}").parse().expect("a valid path"),
            },
            SecretValue::try_from("x".repeat(SecretValue::MAX_BYTES)).expect("a valid secret"),
        )
    });
    let secrets = bundle(SecretsGeneration::from_u64(1), big).expect("a valid bundle");

    let started = Instant::now();
    let output = host
        .runtime()
        .run(
            &guest,
            &GuestCommand::InstallSecrets(&secrets),
            Duration::from_secs(30),
        )
        .await;
    let elapsed = started.elapsed();
    let _ = host.perform(workspace, Command::Delete).await;
    let _ = std::fs::remove_dir_all(settings.runtime_dir.as_path());
    let output = output.expect("the runtime reports the program's own result");
    assert!(!output.success, "{output:?}");
    assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
}

/// A workspace recording another guest-tools interface, as one created
/// before an upgrade would, can't start, but can still be deleted.
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_from_another_iglu_is_refused() {
    let settings = settings("older");
    let runtime = LocalRuntime::new(&settings, TIMEOUTS)
        .await
        .expect("the runtime sets up");
    let host = Host::new(runtime, TIMEOUTS);
    let source = "path:/nonexistent#local".parse().expect("valid source");
    let BuildOutcome::Built(image) = host.build(&source, &FetchTokens::default()).await else {
        panic!("recording the environment failed");
    };
    let workspace = WorkspaceId::from_uuid(uuid::Uuid::new_v4());
    let spec = CreateSpec {
        owner: PrincipalId::from_uuid(uuid::Uuid::new_v4()),
        image: image.fingerprint,
        user: image.user,
        limits: Limits {
            cpus: 1,
            memory: Bytes::new(1 << 30),
            processes: 1024,
            swap: Bytes::new(1 << 30),
        },
    };
    let created = host.perform(workspace, Command::Create(spec)).await;
    assert!(
        matches!(created, CommandOutcome::Done { .. }),
        "{created:?}"
    );

    let record = settings
        .state_dir
        .as_path()
        .join("instances")
        .join(workspace.instance_name().to_string())
        .join("record.json");
    let mut written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record).expect("read")).expect("json");
    written["guest_interface"] = serde_json::json!(INTERFACE.get() - 1);
    std::fs::write(&record, serde_json::to_vec(&written).expect("json")).expect("written");

    let started = host.perform(workspace, Command::Start).await;
    let deleted = host.perform(workspace, Command::Delete).await;
    let _ = std::fs::remove_dir_all(settings.runtime_dir.as_path());
    assert!(
        matches!(&started, CommandOutcome::Failed(e) if e.code == ErrorCode::ImageIncompatible),
        "{started:?}"
    );
    assert!(
        matches!(deleted, CommandOutcome::Done { .. }),
        "{deleted:?}"
    );
}
