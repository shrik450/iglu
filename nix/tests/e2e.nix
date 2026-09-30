# End to end: an identity provider, the control box, an execution host with
# real Incus, a Git server on "public" address space, and a client running
# Chromium. A person signs in and approves the CLI in the browser, creates a
# workspace from a private repository, types in its terminals, gets Claude
# Code's attention, publishes a port, and has a preview page try to act on
# the console. Then workspaces clone over HTTPS, freeze and thaw, survive a
# host restart, wait for memory, and are cleaned up after an interrupted
# delete, and iglud's backups are checked.
{ self, nixpkgs }:
{ lib, pkgs, ... }:

let
  system = "x86_64-linux";
  packages = self.packages.${system};
  image = self.nixosConfigurations.example.config.system.build.igluImage;

  issuer = "https://auth.idp.test:9091";
  gitAddress = "11.0.0.10";

  # Test-only key material, generated at build time.
  pki =
    pkgs.runCommand "iglu-e2e-pki"
      {
        nativeBuildInputs = [
          pkgs.openssl
          pkgs.openssh
        ];
      }
      ''
        mkdir $out && cd $out
        openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 3650 \
          -subj /CN=iglu-e2e-ca -keyout ca.key -out ca.crt \
          -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign
        leaf() {
          openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj "/CN=$1" \
            -keyout "$1.key" -out "$1.csr"
          printf 'subjectAltName=%s\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n' "$2" > "$1.ext"
          openssl x509 -req -in "$1.csr" -CA ca.crt -CAkey ca.key -CAcreateserial -days 3650 \
            -extfile "$1.ext" -out "$1.crt"
        }
        leaf control 'DNS:iglu.example.test,DNS:*.dev.example.test'
        leaf host 'DNS:host'
        leaf idp 'DNS:auth.idp.test'
        leaf git 'IP:${gitAddress}'
        chmod 644 *.key

        openssl genrsa -out oidc.pem 2048
        {
          echo 'identity_providers:'
          echo '  oidc:'
          echo '    jwks:'
          echo '      - key: |'
          sed 's/^/          /' oidc.pem
        } > authelia-jwks.yml

        ssh-keygen -q -t ed25519 -N "" -C deploy -f deploy
        printf 'alice:%s\n' "$(openssl passwd -apr1 pass)" > htpasswd
      '';

  secret = name: pkgs.writeText "${name}-secret" "${name}-secret-for-tests";

  # Playwright's Chromium, trusting the test CA (see the test script) and
  # resolving the console and every preview to the control box.
  browser =
    nodes:
    let
      control = nodes.control.networking.primaryIPAddress;
      idp = nodes.idp.networking.primaryIPAddress;
    in
    pkgs.writeShellApplication {
      name = "browser";
      text = ''
        export HOME=/root
        export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
        export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
        export BROWSER_RESOLVER_RULES='MAP iglu.example.test ${control}, MAP *.dev.example.test ${control}, MAP auth.idp.test ${idp}'
        exec ${pkgs.python3.withPackages (ps: [ ps.playwright ])}/bin/python3 ${./browser.py} "$@"
      '';
    };

  hostsEntries = nodes: ''
    ${nodes.idp.networking.primaryIPAddress} auth.idp.test
    ${nodes.control.networking.primaryIPAddress} iglu.example.test auth.dev.example.test
  '';

  common =
    { nodes, ... }:
    {
      security.pki.certificateFiles = [ "${pki}/ca.crt" ];
      networking.extraHosts = hostsEntries nodes;
    };
in
{
  name = "iglu-e2e";

  nodes = {
    idp =
      { ... }:
      {
        imports = [ common ];
        networking.firewall.allowedTCPPorts = [ 9091 ];

        environment.etc."authelia/users.yml" = {
          user = "authelia-main";
          mode = "0400";
          text = ''
            users:
              alice:
                displayname: Alice
                # "password"
                password: "$argon2id$v=19$m=65536,t=3,p=4$2ohUAfh9yetl+utr4tLcCQ$AsXx0VlwjvNnCsa70u4HKZvFkC8Gwajr2pHGKcND/xs"
                email: alice@example.org
          '';
        };

        services.authelia.instances.main = {
          enable = true;
          secrets.manual = true;
          settingsFiles = [ "${pki}/authelia-jwks.yml" ];
          settings = {
            server.address = "tcp://0.0.0.0:9091/";
            server.tls = {
              certificate = "${pki}/idp.crt";
              key = "${pki}/idp.key";
            };
            log.level = "info";
            authentication_backend.file.path = "/etc/authelia/users.yml";
            access_control.default_policy = "one_factor";
            session = {
              secret = "session-secret-for-tests-only-0123456789";
              cookies = [
                {
                  domain = "idp.test";
                  authelia_url = issuer;
                }
              ];
            };
            storage = {
              encryption_key = "storage-key-for-tests-only-0123456789";
              local.path = "/var/lib/authelia-main/db.sqlite3";
            };
            notifier.filesystem.filename = "/var/lib/authelia-main/notifications.txt";
            identity_validation.reset_password.jwt_secret = "jwt-secret-for-tests-only-0123456789";
            identity_providers.oidc = {
              hmac_secret = "hmac-secret-for-tests-only-0123456789";
              claims_policies.iglu.id_token = [
                "email"
                "email_verified"
                "name"
                "preferred_username"
              ];
              clients =
                map
                  (
                    { id, redirect }:
                    {
                      client_id = id;
                      client_name = id;
                      client_secret = "$plaintext$" + "${id}-secret-for-tests";
                      authorization_policy = "one_factor";
                      consent_mode = "implicit";
                      claims_policy = "iglu";
                      redirect_uris = [ redirect ];
                      scopes = [
                        "openid"
                        "email"
                        "profile"
                      ];
                      token_endpoint_auth_method = "client_secret_basic";
                    }
                  )
                  [
                    {
                      id = "console";
                      redirect = "https://iglu.example.test/auth/callback";
                    }
                    {
                      id = "preview";
                      redirect = "https://auth.dev.example.test/callback";
                    }
                  ]
                ++ [
                  {
                    client_id = "worker";
                    client_name = "iglu worker";
                    client_secret = "$plaintext$worker-secret-for-tests";
                    authorization_policy = "one_factor";
                    grant_types = [ "client_credentials" ];
                    response_types = [ ];
                    scopes = [ ];
                    audience = [ "iglu-hosts" ];
                    # iglud requests no audience, so grant the client's own.
                    requested_audience_mode = "implicit";
                    token_endpoint_auth_method = "client_secret_basic";
                    access_token_signed_response_alg = "RS256";
                  }
                ];
            };
          };
        };
      };

    control =
      { ... }:
      {
        imports = [
          common
          self.nixosModules.control
        ];
        virtualisation.memorySize = 2048;

        services.iglu.control = {
          enable = true;
          consoleHost = "iglu.example.test";
          previewDomain = "dev.example.test";
          acmeHost = "unused-in-tests";
          oidc = {
            inherit issuer;
            console = {
              clientId = "console";
              clientSecretFile = "${secret "console"}";
            };
            preview = {
              clientId = "preview";
              clientSecretFile = "${secret "preview"}";
            };
            worker = {
              clientId = "worker";
              clientSecretFile = "${secret "worker"}";
            };
          };
          signIn.emails = [ "alice@example.org" ];
          backups.keep = 2;
          hosts.host = "https://host:7443";
          extraSettings.workspaces = {
            cpus = 2;
            memory = 1073741824;
            swap = 1073741824;
            processes = 4096;
            reservation = 268435456;
            headroom = 268435456;
          };
        };

        # Tests have no ACME; Caddy serves the test CA's certificate instead.
        services.caddy.virtualHosts =
          lib.genAttrs
            [
              "iglu.example.test"
              "*.dev.example.test"
            ]
            (_: {
              useACMEHost = lib.mkForce null;
              extraConfig = lib.mkForce ''
                tls ${pki}/control.crt ${pki}/control.key
                reverse_proxy 127.0.0.1:7080
              '';
            });

        environment.systemPackages = [ pkgs.sqlite ];
      };

    host =
      { ... }:
      {
        imports = [
          common
          self.nixosModules.host
        ];
        virtualisation = {
          memorySize = 6144;
          cores = 4;
          diskSize = 12288;
          emptyDiskImages = [ 8192 ];
          vlans = [
            1
            2
          ];
          # The environment builds offline: its flake, nixpkgs and the image
          # it evaluates to are already in the store.
          additionalPaths = [
            image
            "${self}"
            "${nixpkgs}"
          ];
        };
        networking.interfaces.eth2.ipv4.addresses = lib.mkForce [
          {
            address = "11.0.0.1";
            prefixLength = 24;
          }
        ];
        # Test VMs force swapDevices to [ ]; freezing needs swap.
        zramSwap.enable = true;

        services.iglu.host = {
          enable = true;
          hostId = "host";
          openFirewall = true;
          tls = {
            certificate = "${pki}/host.crt";
            key = "${pki}/host.key";
          };
          auth = {
            inherit issuer;
            audience = "iglu-hosts";
            subjects = [ "worker" ];
          };
          storage.source = "/dev/vdb";
        };
      };

    git =
      { config, ... }:
      {
        virtualisation.vlans = [ 2 ];
        networking.interfaces.eth1.ipv4.addresses = lib.mkForce [
          {
            address = gitAddress;
            prefixLength = 24;
          }
        ];
        networking.firewall.allowedTCPPorts = [ 443 ];
        services.openssh.enable = true;
        users.users.git.isNormalUser = true;
        environment.systemPackages = [ pkgs.git ];

        # Smart HTTP behind basic auth, so cloning needs the credential helper.
        services.fcgiwrap.instances.git = {
          process.user = "git";
          process.group = "users";
          socket.user = "nginx";
          socket.group = "nginx";
        };
        services.nginx = {
          enable = true;
          virtualHosts.git = {
            default = true;
            onlySSL = true;
            sslCertificate = "${pki}/git.crt";
            sslCertificateKey = "${pki}/git.key";
            locations."/".extraConfig = ''
              auth_basic git;
              auth_basic_user_file ${pki}/htpasswd;
              fastcgi_pass unix:${config.services.fcgiwrap.instances.git.socket.address};
              include ${config.services.nginx.package}/conf/fastcgi_params;
              fastcgi_param SCRIPT_FILENAME ${pkgs.git}/libexec/git-core/git-http-backend;
              fastcgi_param GIT_PROJECT_ROOT /srv/git;
              fastcgi_param GIT_HTTP_EXPORT_ALL "";
              fastcgi_param PATH_INFO $uri;
              fastcgi_param REMOTE_USER $remote_user;
            '';
          };
        };
      };

    client =
      { nodes, ... }:
      {
        imports = [ common ];
        virtualisation.memorySize = 2048;
        environment.systemPackages = [
          packages.iglu
          (browser nodes)
          pkgs.curl
          pkgs.jq
          pkgs.nssTools
          pkgs.websocat
        ];
      };
  };

  testScript =
    { nodes, ... }:
    ''
      PKI = "${pki}"
      SELF = "${self}"
      GIT_ADDRESS = "${gitAddress}"
      HOST_IP = "${nodes.host.networking.primaryIPAddress}"
      CONTROL_IP = "${nodes.control.networking.primaryIPAddress}"
      PYTHON = "${lib.getExe pkgs.python3}"
    ''
    + builtins.readFile ./e2e.py;
}
