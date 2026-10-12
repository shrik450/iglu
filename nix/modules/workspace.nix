# Makes a NixOS configuration an iglu environment: an Incus system
# container image with one unprivileged workspace user and the files hostd
# expects. The guest tools aren't in it: hosts mount their own into every
# workspace, so an environment works with whatever iglu its hosts run.
#
# An environment is `<flake>#<attr>` where `nixosConfigurations.<attr>`
# imports this module. hostd builds `config.system.build.igluImage`.
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

  # Where hosts mount their guest tools (iglu-domain's INSTANCE_TOOLS).
  tools = "/var/lib/iglu/tools";

  manifest = pkgs.writeText "iglu.json" (
    builtins.toJSON {
      arch = pkgs.stdenv.hostPlatform.parsed.cpu.name;
      user = {
        name = cfg.user;
        uid = account.uid;
        gid = group.gid;
        home = account.home;
      };
      browser = cfg.browser.enable;
    }
  );

  claudeHook = {
    type = "command";
    command = "${tools}/bin/iglu-status claude-hook";
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
  imports = [
    "${modulesPath}/virtualisation/lxc-container.nix"
    (lib.mkRemovedOptionModule
      [
        "iglu"
        "agents"
      ]
      "Columns are shells and browsers now: run agents from a shell, and install them as any other package."
    )
    (lib.mkRemovedOptionModule [
      "iglu"
      "package"
    ] "The guest tools come from the host, which mounts its own into every workspace.")
  ];

  options.iglu = {
    user = lib.mkOption {
      type = lib.types.str;
      example = "dev";
      description = ''
        The account terminals and provisioning run as. iglu never runs
        anything of a workspace's as root; only upgrading its system to a
        newer build, with the host's own copy of it, runs as root. The account
        is created if the configuration doesn't define it, and needs a fixed
        uid.
      '';
    };

    browser.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Install Chromium and fonts for its pages, so a workspace can open a
        browser column: a browser the console shows and agents drive over
        the DevTools protocol at `$IGLU_BROWSER` (http://127.0.0.1:9222).
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

      # Where `iglu` reaches the control plane from: the socket hostd's Incus
      # runtime puts in every instance (iglu-domain's INSTANCE_CHANNEL). The
      # guest tools pass it to columns too; this is for every other login.
      environment.variables.IGLU_SOCKET = "/var/lib/iglu-api.sock";

      # The host's tools: `iglu`, which acts as the workspace from inside it
      # through its channel, `iglu-status` and `caffeinate` for agents, and
      # zmx and `iglu-guest`, which hostd drives.
      environment.profiles = [ tools ];
      environment.systemPackages = [ pkgs.git ];

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

      # Git's own helper answers from the credentials the guest tools
      # deliver, and only answers: what Git would store or erase there
      # isn't its to change.
      programs.git = {
        enable = true;
        config.credential.helper = "!f() { test \"$1\" = get && exec git credential-store --file=/run/user/${uid}/iglu/secrets/git-credentials get; }; f";
      };

      nix.settings.experimental-features = lib.mkDefault [
        "nix-command"
        "flakes"
      ];

      # A workspace isn't an installer; users bring their own flakes.
      system.installer.channel.enable = lib.mkDefault false;

      # The system itself too, which workspaces from an older build upgrade
      # to in place: kept with the image, the host has it to copy into them.
      system.build.igluImage = pkgs.runCommand "iglu-image" { } ''
        mkdir $out
        ln -s ${config.system.build.metadata}/tarball/*.tar.xz $out/metadata.tar.xz
        ln -s ${config.system.build.squashfs}/*.squashfs $out/rootfs.squashfs
        ln -s ${config.system.build.toplevel} $out/system
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

    (lib.mkIf cfg.browser.enable {
      environment.systemPackages = [ pkgs.chromium ];
      # The port iglu-guest's browser listens on (iglu-domain's
      # BROWSER_DEBUG_PORT), for Playwright, chrome-devtools-mcp and the like.
      environment.variables.IGLU_BROWSER = "http://127.0.0.1:9222";
      fonts.packages = [
        pkgs.noto-fonts
        pkgs.noto-fonts-color-emoji
      ];
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
