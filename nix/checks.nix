# Checks that don't need a VM: lints, formatting, and the unit tests.
# `just lint` and `just test` run the same commands from the dev shell.
{
  lib,
  craneLib,
  runCommand,
  cargo,
  clippy,
  rustfmt,
  nixfmt,
  actionlint,
  shellcheck,
  zmx,
  lsof,
  git,
  netcat,
}:

let
  workspace = import ./rust-workspace.nix { inherit lib craneLib; };

  # Runs a cargo command over the whole workspace, from its compiled
  # dependencies.
  cargoCheck =
    name: nativeBuildInputs: command:
    craneLib.mkCargoDerivation (
      workspace.common
      // {
        pname = "iglu-${name}";
        CARGO_PROFILE = "dev";
        cargoArtifacts = workspace.dependencies.dev;
        inherit nativeBuildInputs;
        buildPhaseCargoCommand = command;
        installPhaseCommand = "touch $out";
        doInstallCargoArtifacts = false;
      }
    );

  nixFiles = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.fileFilter (file: file.hasExt "nix") ../.;
  };
in
{
  clippy = cargoCheck "clippy" [
    clippy
  ] "cargo clippy --workspace --all-targets --offline -- -D warnings";

  # The local runtime's conformance suite runs the real guest tools and zmx.
  cargo-test =
    cargoCheck "cargo-test"
      [
        zmx
        git
        netcat
        lsof
      ]
      ''
        cargo build -p iglu-guest --offline
        IGLU_GUEST_TOOLS=$PWD/target/debug cargo test --workspace --offline
      '';

  # The console's API types must match what the Rust types generate.
  api-types = cargoCheck "api-types" [ ] ''
    TS_RS_EXPORT_DIR=$PWD/api-types cargo test -p iglu-api --features ts --offline
    diff -r ${../console/src/generated} api-types \
      || { echo "console/src/generated is stale; run 'just api-types'"; exit 1; }
  '';

  rustfmt =
    runCommand "iglu-rustfmt"
      {
        nativeBuildInputs = [
          cargo
          rustfmt
        ];
      }
      ''
        cd ${workspace.src}
        cargo fmt --all --check
        touch $out
      '';

  nixfmt = runCommand "iglu-nixfmt" { nativeBuildInputs = [ nixfmt ]; } ''
    find ${nixFiles} -name '*.nix' -exec nixfmt --check {} +
    touch $out
  '';

  actionlint =
    runCommand "iglu-actionlint"
      {
        nativeBuildInputs = [
          actionlint
          shellcheck
        ];
      }
      ''
        actionlint ${../.github/workflows}/*.yml
        touch $out
      '';
}
