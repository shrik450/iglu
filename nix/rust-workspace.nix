# The Rust workspace as Nix builds it, shared by the packages and the checks.
#
# Dependencies compile once, in a derivation of their own that changes only
# with Cargo.lock and the manifests, so a cache keeps it between commits. The
# release build, clippy and the tests all start from it, and build only
# iglu's own crates.
{
  lib,
  craneLib,
  # The crates to build; all of them unless given.
  crates ? null,
}:

let
  common = {
    pname = "iglu";
    version = (lib.importTOML ../Cargo.toml).workspace.package.version;
    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        ../Cargo.toml
        ../Cargo.lock
        ../crates
      ];
    };
    strictDeps = true;
    # Every build of these crates selects the same ones, so each sees the
    # same features on the dependencies and can reuse them.
    cargoExtraArgs =
      "--locked "
      + (
        if crates == null then "--workspace" else lib.concatMapStringsSep " " (c: "--package ${c}") crates
      );
  };
in
{
  inherit common;
  inherit (common) version src;
  dependencies = {
    # For the packages.
    release = craneLib.buildDepsOnly (common // { doCheck = false; });
    # For clippy and the tests, which keep overflow checks and debug
    # assertions.
    dev = craneLib.buildDepsOnly (common // { CARGO_PROFILE = "dev"; });
  };
}
