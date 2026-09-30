# Checks that don't need a VM: lints, formatting, and the unit tests.
# `just lint` and `just test` run the same commands from the dev shell.
{
  lib,
  rustPlatform,
  runCommand,
  cargo,
  clippy,
  rustfmt,
  nixfmt,
  actionlint,
  shellcheck,
}:

let
  workspace = import ./rust-workspace.nix { inherit lib; };

  # Runs a cargo command over the whole workspace with vendored dependencies.
  cargoCheck =
    name: nativeBuildInputs: command:
    rustPlatform.buildRustPackage {
      pname = "iglu-${name}";
      inherit (workspace) version src cargoLock;
      inherit nativeBuildInputs;
      buildPhase = ''
        runHook preBuild
        ${command}
        runHook postBuild
      '';
      doCheck = false;
      installPhase = "touch $out";
      dontFixup = true;
    };

  nixFiles = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.fileFilter (file: file.hasExt "nix") ../.;
  };
in
{
  clippy = cargoCheck "clippy" [
    clippy
  ] "cargo clippy --workspace --all-targets --offline -- -D warnings";

  cargo-test = cargoCheck "cargo-test" [ ] "cargo test --workspace --offline";

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
