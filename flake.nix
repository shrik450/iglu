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
          all = import ./nix/packages.nix { inherit (pkgs) lib rustPlatform buildNpmPackage; };
        in
        # Only the CLI runs off Linux.
        if pkgs.stdenv.hostPlatform.isLinux then all else { inherit (all) iglu; }
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
            ;
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
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });

      formatter = forSystems systems (pkgs: pkgs.nixfmt-tree);
    };
}
