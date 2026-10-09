# Makes a NixOS configuration an iglu environment: an Incus system
# container image with the guest tools, one unprivileged workspace user, and
# the files hostd expects.
#
# An environment is `<flake>#<attr>` where `nixosConfigurations.<attr>`
# imports this module. hostd builds `config.system.build.igluImage`.
{ self }:
{
  config,
  lib,
  options,
  pkgs,
  modulesPath,
  ...
}:

let
  cfg = config.iglu;
  account = config.users.users.${cfg.user};
  group = config.users.groups.${account.group};
  uid = toString account.uid;
  guest = cfg.package;

  # The interface comes from the installed guest tools themselves, so the
  # image can't claim one its tools don't speak.
  manifest =
    pkgs.runCommand "iglu.json"
      {
        nativeBuildInputs = [ pkgs.jq ];
        base = builtins.toJSON {
          arch = pkgs.stdenv.hostPlatform.parsed.cpu.name;
          user = {
            name = cfg.user;
            uid = account.uid;
            gid = group.gid;
            home = account.home;
          };
          agents = lib.mapAttrsToList (name: agent: {
            inherit name;
            inherit (agent) command prompt attention;
          }) cfg.agents;
        };
      }
      ''
        jq --argjson interface "$(${guest}/bin/iglu-guest interface)" \
          '. + { guest_interface: $interface }' <<<"$base" > $out
      '';

  claudeHook = {
    type = "command";
    command = "${guest}/bin/iglu-status claude-hook";
  };
  claudeEvents = [
    "SessionStart"
    "UserPromptSubmit"
    "Notification"
    "Stop"
    "SessionEnd"
  ];
  claudeToolEvents = [
    "PreToolUse"
    "PostToolUse"
  ];
in
{
  imports = [ "${modulesPath}/virtualisation/lxc-container.nix" ];

  options.iglu = {
    user = lib.mkOption {
      type = lib.types.str;
      example = "dev";
      description = ''
        The account terminals, agents and provisioning run as. iglu never
        runs anything in a workspace as root. The account is created if the
        configuration doesn't define it, and needs a fixed uid.
      '';
    };

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.iglu-guest;
      defaultText = lib.literalExpression "iglu.packages.\${system}.iglu-guest";
      description = "The guest tools: `iglu-guest`, `iglu-status` and `caffeinate`.";
    };

    agents = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options = {
            command = lib.mkOption {
              type = lib.types.nonEmptyListOf lib.types.str;
              example = [ "claude" ];
              description = "The program and its arguments. Run as given, never through a shell.";
            };
            prompt = lib.mkOption {
              type = lib.types.enum [
                "argument"
                "none"
              ];
              default = "argument";
              description = ''
                How the agent takes the prompt a workspace starts it with:
                as one more argument, or not at all.
              '';
            };
            attention = lib.mkOption {
              type = lib.types.enum [
                "claude-hooks"
                "status-command"
              ];
              default = "status-command";
              description = ''
                How the agent reports whether it needs you: Claude Code's
                hooks (see `iglu.claudeCode.hooks`), or `iglu-status set`
                from the agent or a wrapper.
              '';
            };
          };
        }
      );
      default = { };
      example = lib.literalExpression ''
        {
          claude = { command = [ "claude" ]; attention = "claude-hooks"; };
        }
      '';
      description = ''
        The agents this environment's workspaces can run in a column. Their
        programs must be installed in the environment.
      '';
    };

    claudeCode.hooks = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Report Claude Code activity to the console through hooks in a Claude
        Code managed-settings drop-in, which adds to any hooks the user or
        other managed settings define. Turn it off to call
        `iglu-status claude-hook` from your own hooks instead.
      '';
    };
  };

  config = lib.mkMerge [
    {
      assertions = [
        {
          assertion = lib.all (name: builtins.match "[a-z][a-z0-9-]{0,23}" name != null) (
            lib.attrNames cfg.agents
          );
          message = "iglu.agents names must be 1-24 of a-z, 0-9 and '-', starting with a letter.";
        }
        {
          assertion = cfg.user != "root";
          message = "iglu.user must be an unprivileged account.";
        }
        {
          assertion = account.uid != null && account.uid > 0;
          message = "iglu.user's account (${cfg.user}) needs a fixed, non-zero uid.";
        }
        {
          assertion = group.gid != null && group.gid > 0;
          message = "The primary group of iglu.user (${account.group}) needs a fixed, non-zero gid.";
        }
      ];

      users.users.${cfg.user} = {
        isNormalUser = lib.mkDefault true;
        uid = lib.mkDefault 1000;
        # Terminals attach to a user manager that runs without a login.
        linger = true;
      };

      environment.systemPackages = [
        guest
        pkgs.zmx
        pkgs.git
      ];

      # hostd treats the workspace as booted once this marker exists: the
      # system is up and the user's manager is running, so the user's
      # runtime directory, where the guest tools keep delivered secrets and
      # attention status, exists too. The marker goes with the unit.
      systemd.services.iglu-ready = {
        description = "Tell iglu the workspace has booted";
        wantedBy = [ "multi-user.target" ];
        wants = [ "user@${uid}.service" ];
        after = [ "user@${uid}.service" ];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          RuntimeDirectory = "iglu";
          ExecStart = "${pkgs.coreutils}/bin/touch /run/iglu/ready";
        };
      };

      programs.git = {
        enable = true;
        config.credential.helper = "${guest}/bin/iglu-guest git-credential";
      };

      nix.settings.experimental-features = lib.mkDefault [
        "nix-command"
        "flakes"
      ];

      # A workspace isn't an installer; users bring their own flakes.
      system.installer.channel.enable = lib.mkDefault false;

      system.build.igluImage = pkgs.runCommand "iglu-image" { } ''
        mkdir $out
        ln -s ${config.system.build.metadata}/tarball/*.tar.xz $out/metadata.tar.xz
        ln -s ${config.system.build.squashfs}/*.squashfs $out/rootfs.squashfs
        cp ${manifest} $out/iglu.json
      '';
    }

    (lib.mkIf cfg.claudeCode.hooks {
      environment.etc."claude-code/managed-settings.d/50-iglu.json".text = builtins.toJSON {
        hooks =
          lib.genAttrs claudeEvents (_: [ { hooks = [ claudeHook ]; } ])
          // lib.genAttrs claudeToolEvents (_: [
            {
              matcher = "*";
              hooks = [ claudeHook ];
            }
          ]);
      };
    })

    # Home Manager as a NixOS module activates the user's files from a
    # system service. The user's manager, and so every terminal, waits for it.
    (lib.mkIf (options ? home-manager && config.home-manager.users ? ${cfg.user}) {
      systemd.services."user@${uid}" = {
        after = [ "home-manager-${cfg.user}.service" ];
        wants = [ "home-manager-${cfg.user}.service" ];
      };
    })
  ];
}
