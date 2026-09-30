# The iglu control box: iglud behind Caddy, which terminates TLS for the
# console and for every preview under the preview domain.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.iglu.control;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};
  settingsFormat = pkgs.formats.json { };
  stateDir = "/var/lib/iglud";
  secretKeyFile = "${stateDir}/secret-key";
  backupDir = "${stateDir}/backups";

  clientOptions = name: {
    clientId = lib.mkOption {
      type = lib.types.str;
      description = "The ${name} client's ID at the identity provider.";
    };
    clientSecretFile = lib.mkOption {
      type = lib.types.str;
      description = "A file holding the ${name} client's secret, readable by root.";
    };
  };
  client = name: lib.types.submodule { options = clientOptions name; };

  # iglud runs as a dynamic user, so client secrets reach it as systemd
  # credentials rather than by path.
  clients = {
    inherit (cfg.oidc) console preview worker;
  };
  clientSettings = name: c: {
    client_id = c.clientId;
    client_secret_file = "/run/credentials/iglud.service/${name}-client-secret";
  };

  settings = {
    inherit (cfg) listen;
    console_origin = "https://${cfg.consoleHost}";
    preview_domain = cfg.previewDomain;
    database = "${stateDir}/iglu.db";
    secret_key_file = secretKeyFile;
    console_assets = cfg.consolePackage;
    oidc = {
      inherit (cfg.oidc) issuer;
    }
    // lib.mapAttrs clientSettings clients
    // {
      worker =
        clientSettings "worker" cfg.oidc.worker
        // lib.optionalAttrs (cfg.oidc.worker.resource != null) {
          inherit (cfg.oidc.worker) resource;
        };
    };
    sign_in =
      map (subject: { inherit subject; }) cfg.signIn.subjects
      ++ map (email: { verified_email = email; }) cfg.signIn.emails;
    hosts = lib.mapAttrsToList (id: url: { inherit id url; }) cfg.hosts;
  }
  // lib.optionalAttrs cfg.backups.enable {
    backups = {
      directory = backupDir;
      interval_minutes = cfg.backups.intervalMinutes;
      inherit (cfg.backups) keep;
    };
  }
  // cfg.extraSettings;

  proxy = {
    useACMEHost = cfg.acmeHost;
    extraConfig = ''
      reverse_proxy ${cfg.listen}
    '';
  };
in
{
  options.services.iglu.control = {
    enable = lib.mkEnableOption "the iglu control plane";

    package = lib.mkOption {
      type = lib.types.package;
      default = packages.iglud;
      defaultText = lib.literalExpression "iglu.packages.\${system}.iglud";
    };

    consolePackage = lib.mkOption {
      type = lib.types.package;
      default = packages.console;
      defaultText = lib.literalExpression "iglu.packages.\${system}.console";
    };

    listen = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:7080";
      description = "iglud's plain-HTTP address, behind Caddy.";
    };

    consoleHost = lib.mkOption {
      type = lib.types.str;
      example = "iglu.example.org";
      description = "The console's host name.";
    };

    previewDomain = lib.mkOption {
      type = lib.types.str;
      example = "dev.example.org";
      description = ''
        Previews are served at `<route>.<previewDomain>`, and preview sign-in
        at `auth.<previewDomain>`. Point a wildcard DNS record at this box.
        Keep it separate from the console's host so previews can't read the
        console's cookies.
      '';
    };

    acmeHost = lib.mkOption {
      type = lib.types.str;
      description = ''
        The `security.acme.certs` entry Caddy serves. Its certificate must
        cover `consoleHost` and `*.previewDomain`; wildcards need a DNS-01
        challenge.
      '';
    };

    oidc = {
      issuer = lib.mkOption {
        type = lib.types.str;
        example = "https://id.example.org";
      };
      console = lib.mkOption {
        type = client "console";
        description = "Redirect URI: `https://<consoleHost>/auth/callback`.";
      };
      preview = lib.mkOption {
        type = client "preview gateway";
        description = "Redirect URI: `https://auth.<previewDomain>/callback`.";
      };
      worker = lib.mkOption {
        type = lib.types.submodule {
          options = clientOptions "worker" // {
            resource = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "https://iglu-hosts.example.org";
              description = ''
                An RFC 8707 resource to request with the client-credentials
                grant. Without one, the identity provider picks the token's
                audience from its own configuration. Hosts accept only the
                audience in `services.iglu.host.auth.audience`. The identity
                provider must issue JWT access tokens.
              '';
            };
          };
        };
        description = "A client-credentials client whose tokens hosts accept.";
      };
    };

    signIn = {
      emails = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "People who may sign in, by verified email.";
      };
      subjects = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "People who may sign in, by the identity provider's subject.";
      };
    };

    hosts = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      example = {
        host-1 = "https://host-1.lan:7443";
      };
      description = "Execution hosts by ID, and the URL each one's hostd serves.";
    };

    backups = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Keep copies of iglud's database in `/var/lib/iglud/backups`: one at
          every start, before any migration, and one every
          `intervalMinutes`. Ship that directory off the box to survive
          losing the disk. Stored secrets in the copies are sealed with
          `/var/lib/iglud/secret-key`, which they don't include; back the
          key up once, separately.
        '';
      };
      intervalMinutes = lib.mkOption {
        type = lib.types.ints.positive;
        default = 360;
      };
      keep = lib.mkOption {
        type = lib.types.ints.positive;
        default = 28;
        description = "How many copies to keep, newest first.";
      };
    };

    extraSettings = lib.mkOption {
      type = settingsFormat.type;
      default = { };
      example = {
        workspaces = {
          cpus = 4;
          memory = 8589934592;
          swap = 8589934592;
          processes = 8192;
          reservation = 1073741824;
          headroom = 1073741824;
        };
      };
      description = "Other iglud settings, such as `workspaces` limits and `sessions` lifetimes.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.signIn.emails != [ ] || cfg.signIn.subjects != [ ];
        message = "services.iglu.control.signIn must admit someone.";
      }
      {
        assertion = cfg.hosts != { };
        message = "services.iglu.control.hosts must name at least one execution host.";
      }
    ];

    systemd.services.iglud = {
      description = "iglu control plane";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      preStart = ''
        if [ ! -s ${secretKeyFile} ]; then
          (umask 077; ${pkgs.openssl}/bin/openssl rand -hex 32 > ${secretKeyFile}.new)
          mv ${secretKeyFile}.new ${secretKeyFile}
        fi
      '';
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} --config ${settingsFormat.generate "iglud.json" settings}";
        Restart = "on-failure";
        RestartSec = 2;
        LoadCredential = lib.mapAttrsToList (
          name: c: "${name}-client-secret:${c.clientSecretFile}"
        ) clients;
        DynamicUser = true;
        StateDirectory = baseNameOf stateDir;
        StateDirectoryMode = "0700";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallArchitectures = "native";
      };
    };

    services.caddy = {
      enable = true;
      virtualHosts.${cfg.consoleHost} = proxy;
      virtualHosts."*.${cfg.previewDomain}" = proxy;
    };

    networking.firewall.allowedTCPPorts = [
      80
      443
    ];
  };
}
