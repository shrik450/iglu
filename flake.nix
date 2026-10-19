{
  description = "iglu: self-hosted workspaces for coding agents";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  # Builds the Rust workspace with its dependencies in a derivation of their
  # own, which the packages and checks share and caches keep between commits.
  inputs.crane.url = "github:ipetkov/crane";

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    let
      lib = nixpkgs.lib;
      linux = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      systems = linux ++ [
        "aarch64-darwin"
      ];
      forSystems = list: f: lib.genAttrs list (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forSystems systems (
        pkgs:
        let
          all = import ./nix/packages.nix {
            inherit (pkgs)
              lib
              runCommand
              buildNpmPackage
              zmx
              ;
            craneLib = crane.mkLib pkgs;
          };
          # Static, so they run in any workspace, whatever its system: hosts
          # mount them into every instance. Zig links zmx with musl itself.
          static = import ./nix/packages.nix {
            inherit (pkgs.pkgsStatic)
              lib
              runCommand
              buildNpmPackage
              zmx
              ;
            craneLib = crane.mkLib pkgs.pkgsStatic;
            crates = [
              "iglu-guest"
              "iglu-cli"
            ];
          };
          zmx = all.zmx.overrideAttrs (old: {
            zigBuildFlags = (old.zigBuildFlags or [ ]) ++ [
              "-Dtarget=${pkgs.stdenv.hostPlatform.parsed.cpu.name}-linux-musl"
            ];
          });
          guest-tools =
            pkgs.runCommand "iglu-guest-tools"
              {
                # For caches that keep what changes only with the lock files.
                passthru = {
                  inherit zmx;
                  inherit (static.iglu-guest) dependencies;
                };
              }
              ''
                mkdir -p $out/bin
                cp ${static.iglu-guest}/bin/* ${static.iglu}/bin/iglu ${zmx}/bin/zmx $out/bin/
              '';
          # The browser column's VNC server, which workspaces take from here
          # so the patch meets the version it was written for. It types
          # keysyms: text follows the person's keyboard layout, and any
          # character can be typed.
          wayvnc = pkgs.wayvnc.overrideAttrs (old: {
            patches = (old.patches or [ ]) ++ [ ./nix/wayvnc-keysyms.patch ];
          });
        in
        # Off Linux, only the CLI, and zmx for the local runtime.
        if pkgs.stdenv.hostPlatform.isLinux then
          all // { inherit guest-tools wayvnc; }
        else
          { inherit (all) iglu zmx; }
      );

      # What each CI job keeps between runs: what's slow to build and changes
      # only with the lock files. The rest comes from cache.nixos.org, or is
      # built from each commit's sources anyway.
      legacyPackages.x86_64-linux.ci-cache =
        let
          pkgs = nixpkgs.legacyPackages.x86_64-linux;
          packages = self.packages.x86_64-linux;
          # cache.nixos.org has these, but the VM test job wants them while
          # it downloads its machines' closures too, and got the toolchain
          # last, seven minutes in, holding up the build.
          toolchain = [
            pkgs.rustc
            pkgs.cargo
            pkgs.pkgsStatic.buildPackages.rustc
            pkgs.pkgsStatic.buildPackages.cargo
          ];
        in
        {
          checks = toolchain ++ [
            self.checks.x86_64-linux.clippy.cargoArtifacts
            packages.iglud.dependencies
            packages.guest-tools.dependencies
            packages.guest-tools.zmx
            packages.zmx
          ];
          console = [ packages.zmx ];
          e2e = toolchain ++ [
            packages.iglud.dependencies
            packages.guest-tools.dependencies
            packages.guest-tools.zmx
          ];
        };

      nixosModules = {
        workspace = import ./nix/modules/workspace.nix { inherit self; };
        host = import ./nix/modules/host.nix { inherit self; };
        control = import ./nix/modules/control.nix { inherit self; };
      };

      # A minimal environment: `iglu env add example <this flake>#example`.
      # The end-to-end test builds it on its execution host.
      nixosConfigurations.example = lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          self.nixosModules.workspace
          {
            iglu.user = "dev";
            system.stateVersion = "26.05";
          }
        ];
      };

      # The same environment, built again with a change: what the tests
      # update a workspace to.
      nixosConfigurations.example-next = lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          self.nixosModules.workspace
          {
            iglu.user = "dev";
            system.stateVersion = "26.05";
            environment.etc."iglu-example".text = "next";
          }
        ];
      };

      templates.workspace = {
        path = ./templates/workspace;
        description = "An iglu environment: a NixOS workspace for coding agents";
      };

      # Lints and unit tests everywhere; on x86_64-linux, also the VM test.
      checks = forSystems systems (
        pkgs:
        let
          system = pkgs.stdenv.hostPlatform.system;
        in
        self.packages.${system}
        // import ./nix/checks.nix {
          inherit (pkgs)
            lib
            runCommand
            cargo
            clippy
            rustfmt
            nixfmt
            actionlint
            shellcheck
            git
            netcat
            lsof
            buildNpmPackage
            python3
            playwright-driver
            mesa
            makeFontsConf
            dejavu_fonts
            ;
          inherit (self.packages.${system}) zmx;
          # The terminal tier runs the console in Chromium and WebKit, which
          # the sandbox has on Linux only.
          console = if pkgs.stdenv.hostPlatform.isLinux then self.packages.${system}.console else null;
          craneLib = crane.mkLib pkgs;
        }
        // lib.optionalAttrs (system == "x86_64-linux") {
          e2e = pkgs.testers.runNixOSTest (import ./nix/tests/e2e.nix { inherit self nixpkgs; });
        }
      );

      devShells = forSystems systems (
        pkgs:
        let
          # Chromium and WebKit, at the version the Python package drives.
          playwrightBrowsers = pkgs.playwright-driver.browsers.override { withFirefox = false; };
        in
        {
          default = pkgs.mkShell {
            packages =
              with pkgs;
              [
                cargo
                rustc
                clippy
                rustfmt
                rust-analyzer
                nodejs
                sqlite
                just
                nixfmt
                actionlint
                shellcheck
                # Mutation testing and coverage; see DEVELOPMENT.md.
                cargo-mutants
                cargo-llvm-cov
                # The local runtime runs workspaces with these.
                self.packages.${pkgs.stdenv.hostPlatform.system}.zmx
                git
                netcat
                lsof
              ]
              # Local browser columns run Chromium on a headless sway, shown
              # over wayvnc, which only Linux has.
              ++ lib.optionals stdenv.hostPlatform.isLinux [
                chromium
                sway-unwrapped
                self.packages.${pkgs.stdenv.hostPlatform.system}.wayvnc
                # `just dev-trust` adds the dev CA to the NSS database.
                nssTools
              ]
              ++ [
                # The dev stack's TLS front and identity provider, and its CA.
                caddy
                authelia
                openssl
                # Seeding, touring and checking the console in real browsers,
                # and coverage of the lines a change touches; see DEVELOPMENT.md.
                (python3.withPackages (ps: [
                  ps.playwright
                  ps.pillow
                  ps.diff-cover
                ]))
              ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            # cargo-llvm-cov reads profiles with the LLVM rustc was built with.
            LLVM_COV = "${pkgs.rustc.llvmPackages.llvm}/bin/llvm-cov";
            LLVM_PROFDATA = "${pkgs.rustc.llvmPackages.llvm}/bin/llvm-profdata";
            # The dev stack's terminals run this, not your login shell, so the
            # console's checks type into the same shell on every machine.
            IGLU_DEV_SHELL = lib.getExe pkgs.bashInteractive;
            # WebKit needs an EGL display; without a GPU, Mesa's software one.
            IGLU_MESA_EGL = lib.optionalString pkgs.stdenv.hostPlatform.isLinux "${pkgs.mesa}/share/glvnd/egl_vendor.d";
            PLAYWRIGHT_BROWSERS_PATH = playwrightBrowsers;
            # NixOS has none of the paths Playwright checks for; empty elsewhere.
            PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = lib.optionalString pkgs.stdenv.hostPlatform.isLinux "true";
          };
        }
      );

      formatter = forSystems systems (pkgs: pkgs.nixfmt-tree);
    };
}
