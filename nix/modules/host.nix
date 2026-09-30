# An iglu execution host: Incus with a Btrfs pool, a workspace bridge and
# profile, the Nix daemon for environment builds, and hostd.
#
# Users never sign in here. hostd accepts only iglud's service tokens.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.iglu.host;
  incus = config.virtualisation.incus;
  settingsFormat = pkgs.formats.json { };
  runtimeDir = "/run/iglu-hostd";

  settings = {
    host_id = cfg.hostId;
    listen = cfg.listen;
    tls = {
      inherit (cfg.tls) certificate key;
    };
    auth = {
      inherit (cfg.auth) issuer audience subjects;
    };
    incus = {
      socket = "/var/lib/incus/unix.socket";
      binary = lib.getExe' incus.clientPackage "incus";
      profile = "iglu";
      network = cfg.network.name;
      acl = "iglu-egress";
    };
    build = {
      nix = lib.getExe' config.nix.package "nix";
      uid = cfg.build.uid;
      gid = cfg.build.uid;
      home = "/var/lib/iglu-build";
    };
    runtime_dir = runtimeDir;
  };
in
{
  options.services.iglu.host = {
    enable = lib.mkEnableOption "an iglu execution host";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.hostd;
      defaultText = lib.literalExpression "iglu.packages.\${system}.hostd";
    };

    hostId = lib.mkOption {
      type = lib.types.str;
      example = "host-1";
      description = "This host's name in iglud's `hosts` list.";
    };

    listen = lib.mkOption {
      type = lib.types.str;
      default = "0.0.0.0:7443";
      description = "Where hostd serves iglud, over TLS.";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Open hostd's port. Prefer limiting it to the control box's address yourself.";
    };

    tls = {
      certificate = lib.mkOption {
        type = lib.types.str;
        description = "PEM certificate chain for hostd. iglud verifies it against the system trust store.";
      };
      key = lib.mkOption {
        type = lib.types.str;
        description = "PEM private key, readable by root only.";
      };
    };

    auth = {
      issuer = lib.mkOption {
        type = lib.types.str;
        example = "https://id.example.org";
        description = "The OIDC issuer that signs iglud's service tokens.";
      };
      audience = lib.mkOption {
        type = lib.types.str;
        description = "The `aud` of iglud's service tokens, as the identity provider issues them. Decode one token from the worker client to see it.";
      };
      subjects = lib.mkOption {
        type = lib.types.nonEmptyListOf lib.types.str;
        description = "The `sub` values allowed to drive this host: the subject the identity provider gives the worker client's tokens.";
      };
    };

    storage.source = lib.mkOption {
      type = lib.types.str;
      example = "/dev/disk/by-id/nvme-example-part3";
      description = ''
        What backs the Btrfs storage pool: a block device or partition for
        Incus to format, or a directory on an existing Btrfs filesystem.
      '';
    };

    network = {
      name = lib.mkOption {
        type = lib.types.str;
        default = "iglubr0";
        description = "The bridge workspaces attach to.";
      };
      address = lib.mkOption {
        type = lib.types.str;
        default = "10.231.0.1/24";
        description = "The bridge's address and prefix. Workspaces get addresses by DHCP inside it.";
      };
    };

    build.uid = lib.mkOption {
      type = lib.types.int;
      default = 30871;
      description = "uid and gid of the unprivileged account environment builds evaluate as.";
    };
  };

  config = lib.mkIf cfg.enable {
    warnings = lib.optional (config.swapDevices == [ ] && !config.zramSwap.enable) ''
      iglu freezes idle workspaces by pushing their memory to swap. Without
      swap, frozen workspaces keep their memory resident.
    '';

    networking.nftables.enable = true;
    networking.firewall.interfaces.${cfg.network.name} = {
      allowedUDPPorts = [
        53
        67
      ];
      allowedTCPPorts = [ 53 ];
    };
    # The egress ACL keeps workspaces off private networks, but it counts the
    # host's own public addresses as the Internet. Whatever a workspace sends
    # the host itself, beyond DHCP and DNS on the bridge, is dropped here,
    # ahead of the firewall's allowances for SSH, hostd and the like.
    networking.nftables.tables.iglu-host-guard = {
      family = "inet";
      content = ''
        chain input {
          type filter hook input priority filter - 10; policy accept;
          iifname "${cfg.network.name}" ct state established,related accept
          iifname "${cfg.network.name}" udp dport { 53, 67 } accept
          iifname "${cfg.network.name}" tcp dport 53 accept
          iifname "${cfg.network.name}" drop
        }
      '';
    };
    networking.firewall.allowedTCPPorts = lib.optional cfg.openFirewall (
      lib.toInt (lib.last (lib.splitString ":" cfg.listen))
    );

    virtualisation.incus = {
      enable = true;
      preseed = {
        networks = [
          {
            name = cfg.network.name;
            type = "bridge";
            config = {
              "ipv4.address" = cfg.network.address;
              "ipv4.nat" = "true";
              "ipv6.address" = "none";
            };
          }
        ];
        profiles = [
          {
            name = "iglu";
            description = "iglu workspaces";
            devices = {
              root = {
                type = "disk";
                path = "/";
                pool = "iglu";
              };
              eth0 = {
                type = "nic";
                name = "eth0";
                network = cfg.network.name;
              };
            };
          }
        ];
      };
    };

    # Incus rewrites a block device pool's source once it creates the pool, so
    # the preseed, which reapplies its settings at every boot, would fail
    # trying to change it back. Create the pool once instead.
    systemd.services.iglu-storage-pool = {
      description = "Create iglu's Incus storage pool";
      requires = [ "incus.service" ];
      after = [ "incus.service" ];
      before = [ "incus-preseed.service" ];
      requiredBy = [ "incus-preseed.service" ];
      path = [ incus.clientPackage ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = ''
        incus storage show iglu >/dev/null 2>&1 ||
          incus storage create iglu btrfs source=${lib.escapeShellArg cfg.storage.source}
      '';
    };

    users.users.iglu-build = {
      isSystemUser = true;
      uid = cfg.build.uid;
      group = "iglu-build";
      home = "/var/lib/iglu-build";
      createHome = true;
      description = "iglu environment builds";
    };
    users.groups.iglu-build.gid = cfg.build.uid;

    nix.settings.experimental-features = [
      "nix-command"
      "flakes"
    ];

    systemd.services.iglu-hostd = {
      description = "iglu host agent";
      wantedBy = [ "multi-user.target" ];
      requires = [ "incus.service" ];
      wants = [ "network-online.target" ];
      after = [
        "incus.service"
        "incus-preseed.service"
        "network-online.target"
      ];
      path = [ config.nix.package ];
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} --config ${settingsFormat.generate "iglu-hostd.json" settings}";
        Restart = "on-failure";
        RestartSec = 2;
        RuntimeDirectory = baseNameOf runtimeDir;
        # Proxy devices keep listening on sockets here across hostd restarts.
        RuntimeDirectoryPreserve = true;
      };
    };
  };
}
