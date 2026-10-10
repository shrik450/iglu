{
  description = "iglu: self-hosted workspaces for coding agents";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
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
              rustPlatform
              buildNpmPackage
              zmx
              ;
          };
        in
        # Off Linux, only the CLI, and zmx for the local runtime.
        if pkgs.stdenv.hostPlatform.isLinux then all else { inherit (all) iglu zmx; }
      );

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
            # A stand-in agent that shows its prompt, so the test can read it back.
            iglu.agents.echo = {
              command = [
                "/bin/sh"
                "-c"
                "printf 'PROMPT<%s>' \"$1\"; exec sleep infinity"
                "echo"
              ];
              prompt = "argument";
            };
            system.stateVersion = "26.05";
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
            rustPlatform
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
            ;
          inherit (self.packages.${system}) zmx;
        }
        // lib.optionalAttrs (system == "x86_64-linux") {
          e2e = pkgs.testers.runNixOSTest (import ./nix/tests/e2e.nix { inherit self nixpkgs; });
        }
      );

      devShells = forSystems systems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
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
            # The local runtime runs workspaces with these.
            self.packages.${pkgs.stdenv.hostPlatform.system}.zmx
            git
            netcat
            lsof
            # The dev stack's clients. The Docker daemon is yours to run.
            docker-client
            openssl
            # Seeding, touring and checking the console in real browsers;
            # see DEVELOPMENT.md.
            (python3.withPackages (ps: [
              ps.playwright
              ps.pillow
            ]))
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          # Chromium and WebKit, at the version the Python package drives.
          PLAYWRIGHT_BROWSERS_PATH = pkgs.playwright-driver.browsers.override { withFirefox = false; };
          # NixOS has none of the paths Playwright checks for; empty elsewhere.
          PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = lib.optionalString pkgs.stdenv.hostPlatform.isLinux "true";
        };
      });

      formatter = forSystems systems (pkgs: pkgs.nixfmt-tree);
    };
}
