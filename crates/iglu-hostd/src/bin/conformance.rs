//! `iglu-hostd-conformance --config <hostd config> --source <flake>#<attr>`:
//! runs the runtime conformance suite and the checks only Incus can run against
//! this host's Incus, with hostd's own configuration. The VM test runs it.
//! It creates and deletes workspaces of its own.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use iglu_domain::env::EnvSource;
use iglu_hostd::config::Config;
use iglu_hostd::conformance::{self, Fixture, Outcome};
use iglu_hostd::host::Host;
use iglu_hostd::incus::{IncusRuntime, checks};

fn flag(name: &str) -> Option<String> {
    std::env::args().skip_while(|arg| arg != name).nth(1)
}

fn report(outcomes: &[Outcome]) -> bool {
    for outcome in outcomes {
        match &outcome.result {
            Ok(()) => println!("ok    {}", outcome.check),
            Err(why) => println!("FAIL  {}: {why}", outcome.check),
        }
    }
    outcomes.iter().all(|o| o.result.is_ok())
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    tracing_subscriber::fmt().init();
    let usage = "usage: iglu-hostd-conformance --config <path> --source <flake>#<attr>";
    let config: Config =
        iglu_hostd::config::load(&PathBuf::from(flag("--config").context(usage)?))?;
    let source: EnvSource = flag("--source").context(usage)?.parse()?;
    let fixture = Fixture {
        source,
        boot_timeout: Duration::from_secs(300),
    };

    let runtime = IncusRuntime::new(
        config.incus,
        config.build,
        &config.runtime_dir,
        config.timeouts,
    )
    .await?;
    let host = Host::new(runtime, config.timeouts);
    let mut passed = report(&conformance::run(&host, &fixture).await);
    let host_ref = &host;
    let isolated = conformance::inside(&host, &fixture, |guest| async move {
        checks::run(host_ref, &guest).await
    })
    .await;
    passed &= report(&isolated);
    Ok(if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
