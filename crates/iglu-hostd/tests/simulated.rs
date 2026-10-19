//! The simulated runtime against the runtime conformance suite, with each
//! step taking effect at once. Its guests run no programs, so it's held to
//! the checks that need none.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use iglu_hostd::config::Timeouts;
use iglu_hostd::conformance::{self, Fixture, Guests};
use iglu_hostd::host::Host;
use iglu_hostd::sim::{SimConfig, SimRuntime};

#[tokio::test]
async fn the_simulated_runtime_conforms() {
    let channels = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sim-channels");
    std::fs::create_dir_all(&channels).expect("made");
    let runtime = SimRuntime::new(SimConfig {
        gated: false,
        repos: BTreeSet::new(),
        channels,
        operation: Duration::from_secs(120),
    });
    let host = Host::new(runtime.clone(), Timeouts::default());
    let fixture = Fixture {
        source: "path:/sim#env".parse().expect("valid source"),
        next: "path:/sim#next".parse().expect("valid source"),
        boot_timeout: Duration::from_secs(10),
        guests: Guests::Modelled,
    };
    let outcomes = conformance::run(&host, &fixture).await;
    for outcome in &outcomes {
        eprintln!("{:?} {}", outcome.result, outcome.check);
    }
    let failed: Vec<_> = outcomes.iter().filter(|o| o.result.is_err()).collect();
    assert!(failed.is_empty(), "failed checks: {failed:#?}");
    assert_eq!(runtime.violations(), Vec::<String>::new());
}
