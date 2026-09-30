{
  description = "An iglu environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Point this at wherever you get iglu from.
    iglu.url = "github:OWNER/iglu";
    iglu.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    { nixpkgs, iglu, ... }:
    {
      # Register with `iglu env add <name> <this flake>#default`.
      nixosConfigurations.default = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          iglu.nixosModules.workspace
          (
            { pkgs, ... }:
            {
              iglu.user = "dev";

              nixpkgs.config.allowUnfree = true;
              environment.systemPackages = with pkgs; [
                claude-code
                chromium
                ripgrep
                fd
                jq
                gh
              ];

              system.stateVersion = "26.05";
            }
          )
        ];
      };
    };
}
