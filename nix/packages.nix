# iglu's packages. The Rust ones come from one build of the workspace, and
# each installs only its own binaries, so a guest image carries the guest
# tools and nothing else.
{
  lib,
  craneLib,
  runCommand,
  buildNpmPackage,
  zmx,
  # The crates to build; all of them unless given.
  crates ? null,
}:

let
  workspace = import ./rust-workspace.nix { inherit lib craneLib crates; };
  inherit (workspace) version;
  inherit (workspace.common) cargoExtraArgs;

  build = craneLib.buildPackage (
    workspace.common
    // {
      cargoArtifacts = workspace.dependencies.release;
      buildPhaseCargoCommand = ''
        cargoBuildLog=$(mktemp cargoBuildLogXXXX.json)
        cargoWithProfile build ${cargoExtraArgs} --message-format json-render-diagnostics >"$cargoBuildLog"
      ''
      + lib.optionalString (crates == null) ''
        # The conformance suite's binary, built on its own so that hostd's
        # binary doesn't take the feature.
        cargoWithProfile build ${cargoExtraArgs} --features iglu-hostd/conformance \
          --bin iglu-hostd-conformance --message-format json-render-diagnostics >>"$cargoBuildLog"
      '';
      # The workspace test check runs every crate's tests once.
      doCheck = false;
    }
  );

  binaries =
    {
      pname,
      bins,
      description,
    }:
    runCommand "${pname}-${version}"
      {
        inherit pname version;
        # For caches that keep what changes only with the lock files.
        passthru.dependencies = workspace.dependencies.release;
        meta = {
          inherit description;
          mainProgram = builtins.head bins;
          license = lib.licenses.mit;
        };
      }
      ''
        mkdir -p $out/bin
        cp ${lib.concatMapStringsSep " " (bin: "${build}/bin/${bin}") bins} $out/bin/
      '';
in
{
  # The guest tools' terminal sessions. With ZMX_NO_CREATE set, `zmx attach`
  # only attaches: one racing a column's close can't open the session again.
  # `zmx history` fails when it can't read a session, rather than printing
  # nothing, and writes all of a long one. A client attaching again gets the
  # whole history: the screenful just above the screen was lost.
  zmx = zmx.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ [
      ./zmx-attach-only.patch
      ./zmx-history.patch
      ./zmx-replay-history.patch
    ];
  });

  iglud = binaries {
    pname = "iglud";
    bins = [ "iglud" ];
    description = "iglu's control plane: console, API, reconciler and preview gateway";
  };

  hostd = binaries {
    pname = "iglu-hostd";
    bins = [ "iglu-hostd" ];
    description = "iglu's execution host agent";
  };

  # The runtime conformance suite and Incus's isolation checks, which the
  # VM test runs on its execution host.
  hostd-conformance = binaries {
    pname = "iglu-hostd-conformance";
    bins = [ "iglu-hostd-conformance" ];
    description = "Checks an iglu execution host's runtime against hostd's contract";
  };

  iglu = binaries {
    pname = "iglu";
    bins = [ "iglu" ];
    description = "The iglu command line";
  };

  iglu-guest = binaries {
    pname = "iglu-guest";
    bins = [
      "iglu-guest"
      "iglu-status"
      "caffeinate"
      "xclip"
      "xsel"
      "xdg-open"
    ];
    description = "Tools iglu installs inside workspaces";
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
        ../console/vendor
      ];
    };
    npmDepsHash = "sha256-IiNkd9c40g67C/CpNNjrasfF6kHjPk/46aH+67ZjB1M=";
    installPhase = ''
      runHook preInstall
      cp -r dist $out
      runHook postInstall
    '';
  };
}
