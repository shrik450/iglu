# iglu's packages. Each Rust package builds only its own crate, so a guest
# image carries the guest tools and nothing else.
{
  lib,
  rustPlatform,
  buildNpmPackage,
}:

let
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
    ];
  };

  crate =
    {
      pname,
      crateName,
      description,
      mainProgram,
    }:
    rustPlatform.buildRustPackage {
      inherit pname version src;
      cargoLock.lockFile = ../Cargo.lock;
      cargoBuildFlags = [
        "--package"
        crateName
      ];
      cargoTestFlags = [
        "--package"
        crateName
      ];
      meta = {
        inherit description mainProgram;
        license = lib.licenses.mit;
      };
    };
in
{
  iglud = crate {
    pname = "iglud";
    crateName = "iglud";
    description = "iglu's control plane: console, API, reconciler and preview gateway";
    mainProgram = "iglud";
  };

  hostd = crate {
    pname = "iglu-hostd";
    crateName = "iglu-hostd";
    description = "iglu's execution host agent";
    mainProgram = "hostd";
  };

  iglu = crate {
    pname = "iglu";
    crateName = "iglu-cli";
    description = "The iglu command line";
    mainProgram = "iglu";
  };

  iglu-guest = crate {
    pname = "iglu-guest";
    crateName = "iglu-guest";
    description = "Tools iglu installs inside workspaces";
    mainProgram = "iglu-guest";
  };

  console = buildNpmPackage {
    pname = "iglu-console";
    inherit version;
    src = lib.fileset.toSource {
      root = ../console;
      fileset = lib.fileset.unions [
        ../console/package.json
        ../console/package-lock.json
        ../console/build.mjs
        ../console/tsconfig.json
        ../console/src
        ../console/public
      ];
    };
    npmDepsHash = "sha256-siZdqvlujpsfea9Zbq9WEkREkO4LgnGuXQYkVgFOrTg=";
    installPhase = ''
      runHook preInstall
      cp -r dist $out
      runHook postInstall
    '';
  };
}
