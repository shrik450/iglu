//! Checks only Incus can run, inside a real guest: negative tests for what
//! makes [`IncusRuntime`] isolating, as far as hostd can see them from the
//! host, and the exec plumbing the generic suite can't reach. The VM test
//! checks the network side from the guests themselves.

use std::time::{Duration, Instant};

use iglu_domain::guest as guest_tools;
use iglu_domain::lifecycle::SecretsGeneration;
use iglu_domain::secret::bundle;

use iglu_proto::{Command, CommandOutcome, ErrorCode};
use serde_json::json;

use super::IncusRuntime;
use super::exec::{self, Program};
use super::guestfs::paths;
use super::observe::keys;
use crate::conformance::Outcome;
use crate::guest::{GuestCommand, parse_generation};
use crate::host::Host;
use crate::runtime::{Guest, GuestFile, Runtime};

/// A host file whose path a guest might try to point hostd at.
const HOST_ONLY: &str = "/run/iglu-hostd-conformance-marker";

/// Runs the checks inside `guest`, a running workspace the caller made.
pub async fn run(host: &Host<IncusRuntime>, guest: &Guest) -> Vec<Outcome> {
    let runtime = host.runtime();
    vec![
        Outcome {
            check: "guest programs run as the workspace user",
            result: unprivileged(runtime, guest).await,
        },
        Outcome {
            check: "a guest's symlinks can't redirect hostd's reads",
            result: no_redirected_reads(runtime, guest).await,
        },
        Outcome {
            check: "a program that ignores its input still finishes",
            result: unread_input(runtime, guest).await,
        },
        Outcome {
            check: "a workspace from another iglu gets no secrets",
            result: other_interface_refused(host, guest).await,
        },
    ]
}

async fn as_user(runtime: &IncusRuntime, guest: &Guest, argv: &[&str]) -> Result<String, String> {
    let program = Program::new(&guest.user, argv.iter().copied());
    let output = exec::capture(
        &runtime.incus,
        guest.name,
        &program,
        None,
        Duration::from_secs(60),
    )
    .await
    .map_err(|e| e.to_string())?;
    if output.success {
        Ok(output.stdout)
    } else {
        Err(format!("{argv:?} failed: {}", output.stderr_tail()))
    }
}

async fn unprivileged(runtime: &IncusRuntime, guest: &Guest) -> Result<(), String> {
    let uid = as_user(runtime, guest, &["id", "-u"]).await?;
    if uid.trim() == guest.user.uid.get().to_string() && uid.trim() != "0" {
        Ok(())
    } else {
        Err(format!("ran as uid {uid:?}"))
    }
}

/// Plants symlinks where hostd reads the secrets generation, pointing at a
/// host file by absolute and relative paths and through the guest's
/// `/proc`, and checks hostd never reads the host file's contents. First,
/// as a positive control, a real delivery must read back, so the test can't
/// pass by looking in the wrong place.
async fn no_redirected_reads(runtime: &IncusRuntime, guest: &Guest) -> Result<(), String> {
    let generation = SecretsGeneration::from_u64(7);
    let secrets = bundle(generation, []).map_err(|e| e.to_string())?;
    let installed = runtime
        .run(
            guest,
            &GuestCommand::InstallSecrets(&secrets),
            Duration::from_secs(60),
        )
        .await
        .map_err(|e| e.to_string())?;
    if !installed.success {
        return Err(format!(
            "installing secrets failed: {}",
            installed.stderr_tail()
        ));
    }
    let read = runtime
        .read(guest, GuestFile::SecretsGeneration)
        .await
        .map_err(|e| e.to_string())?;
    if read.as_deref().and_then(parse_generation) != Some(generation) {
        return Err(format!("the delivery itself didn't read back: {read:?}"));
    }

    tokio::fs::write(HOST_ONLY, b"4242\n")
        .await
        .map_err(|e| e.to_string())?;
    let path = format!(
        "{}/{}",
        paths::runtime_dir(&guest.user),
        guest_tools::SECRETS_GENERATION
    );
    let relative = format!("{}{HOST_ONLY}", "../".repeat(12));
    let through_proc = format!("/proc/1/root{HOST_ONLY}");
    let mut result = Ok(());
    for target in [HOST_ONLY, relative.as_str(), through_proc.as_str()] {
        if let Err(why) = as_user(runtime, guest, &["ln", "-sfn", target, &path]).await {
            result = Err(why);
            break;
        }
        // Either the link resolves inside the guest, where there's no such
        // file, or hostd refuses to follow it: ELOOP for a magic link,
        // EXDEV for an escape. Anything else is a failure of its own.
        match runtime.read(guest, GuestFile::SecretsGeneration).await {
            Ok(None) => {}
            Err(error)
                if ["os error 40", "os error 18"]
                    .iter()
                    .any(|e| error.to_string().contains(e)) => {}
            other => {
                result = Err(format!("following {target} read {other:?}"));
                break;
            }
        }
    }
    let _ = tokio::fs::remove_file(HOST_ONLY).await;
    result
}

/// A program that exits without reading its input must still come back,
/// with its own status, however much input was waiting for it.
async fn unread_input(runtime: &IncusRuntime, guest: &Guest) -> Result<(), String> {
    let program = Program::new(&guest.user, ["true"]);
    let input = vec![b'x'; 2 * 1024 * 1024];
    let started = Instant::now();
    let output = exec::capture(
        &runtime.incus,
        guest.name,
        &program,
        Some(&input),
        Duration::from_secs(60),
    )
    .await
    .map_err(|e| e.to_string())?;
    if output.success && started.elapsed() < Duration::from_secs(30) {
        Ok(())
    } else {
        Err(format!("after {:?}: {output:?}", started.elapsed()))
    }
}

/// An instance recording another guest-tools interface, as one created
/// before an upgrade would, is refused secrets with a typed error.
async fn other_interface_refused(host: &Host<IncusRuntime>, guest: &Guest) -> Result<(), String> {
    let record = |interface: String| async move {
        host.runtime()
            .incus
            .update_instance(
                guest.name,
                |instance| instance["config"][keys::GUEST_INTERFACE] = json!(interface),
                Duration::from_secs(60),
            )
            .await
            .map_err(|e| e.to_string())
    };
    let older = guest_tools::INTERFACE.get() - 1;
    record(older.to_string()).await?;
    let secrets = bundle(SecretsGeneration::from_u64(9), []).map_err(|e| e.to_string())?;
    let outcome = host
        .perform(guest.name.workspace(), Command::DeliverSecrets(secrets))
        .await;
    record(guest_tools::INTERFACE.to_string()).await?;
    match outcome {
        CommandOutcome::Failed(error) if error.code == ErrorCode::ImageIncompatible => Ok(()),
        other @ (CommandOutcome::Done { .. } | CommandOutcome::Failed(_)) => {
            Err(format!("expected ImageIncompatible, got {other:?}"))
        }
    }
}
