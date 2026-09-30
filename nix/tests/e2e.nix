# End to end: an identity provider, the control box, an execution host with
# real Incus, and a Git server on "public" address space. A user signs in,
# signs in the CLI, builds an environment, creates a workspace from a private
# repository, uses a terminal, publishes a port, freezes, thaws and deletes.
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
        leaf control 'DNS:iglu.test,DNS:*.preview.test'
        leaf host 'DNS:host'
        leaf idp 'DNS:auth.idp.test'
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
      '';

  secret = name: pkgs.writeText "${name}-secret" "${name}-secret-for-tests";

  hostsEntries = nodes: ''
    ${nodes.idp.networking.primaryIPAddress} auth.idp.test
    ${nodes.control.networking.primaryIPAddress} iglu.test auth.preview.test
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
                      redirect = "https://iglu.test/auth/callback";
                    }
                    {
                      id = "preview";
                      redirect = "https://auth.preview.test/callback";
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
          consoleHost = "iglu.test";
          previewDomain = "preview.test";
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
              audience = "iglu-hosts";
            };
          };
          signIn.emails = [ "alice@example.org" ];
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
              "iglu.test"
              "*.preview.test"
            ]
            (_: {
              useACMEHost = lib.mkForce null;
              extraConfig = lib.mkForce ''
                tls ${pki}/control.crt ${pki}/control.key
                reverse_proxy 127.0.0.1:7080
              '';
            });

        environment.systemPackages = [
          packages.iglu
          pkgs.curl
          pkgs.jq
          pkgs.websocat
        ];
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
      { ... }:
      {
        virtualisation.vlans = [ 2 ];
        networking.interfaces.eth1.ipv4.addresses = lib.mkForce [
          {
            address = gitAddress;
            prefixLength = 24;
          }
        ];
        services.openssh.enable = true;
        users.users.git.isNormalUser = true;
        environment.systemPackages = [ pkgs.git ];
      };
  };

  testScript =
    { nodes, ... }:
    ''
      import json
      import re
      import shlex
      import urllib.parse

      def found(pattern: str, text: str) -> str:
          match = re.search(pattern, text)
          assert match, f"{pattern} not in {text}"
          return match.group(1)

      def diagnose() -> None:
          for machine, units in [
              (control, "iglud caddy"),
              (host, "iglu-hostd incus incus-preseed"),
              (idp, "authelia-main"),
          ]:
              flags = " ".join(f"-u {unit}" for unit in units.split())
              _, logs = machine.execute(f"journalctl --no-pager -n 150 {flags}")
              print(f"===== {machine.name}: {units} =====\n{logs}")

      try:
          start_all()

          with subtest("the Git server has a private repository"):
              git.wait_for_unit("sshd.service")
              git.succeed(
                  "install -d -o git -m 700 /home/git/.ssh",
                  "install -o git -m 600 ${pki}/deploy.pub /home/git/.ssh/authorized_keys",
                  "install -d -o git -m 755 /srv/git",
                  "su git -c 'set -e; git init -q --bare -b main /srv/git/app.git; "
                  "git init -q -b main /tmp/seed; cd /tmp/seed; echo hello > README; git add README; "
                  "git -c user.name=t -c user.email=t@t commit -q -m init; git push -q /srv/git/app.git main'",
              )

          with subtest("services come up"):
              idp.wait_for_unit("authelia-main.service")
              idp.wait_for_open_port(9091)
              host.wait_for_unit("iglu-hostd.service")
              host.wait_for_open_port(7443)
              control.wait_for_unit("iglud.service")
              control.wait_for_open_port(443)

          with subtest("hostd put the egress policy on the bridge"):
              acl = json.loads(host.succeed("incus query /1.0/network-acls/iglu-egress"))
              assert any(rule["action"] == "allow" for rule in acl["egress"]), acl
              bridge = host.succeed("incus network get iglubr0 security.acls").strip()
              assert bridge == "iglu-egress", bridge

          jar = "/tmp/browser-cookies"
          curl = f"curl -sS --fail-with-body -c {jar} -b {jar}"

          def location(url, *extra):
              args = " ".join(extra)
              return control.succeed(f"{curl} {args} -o /dev/null -w '%{{redirect_url}}' {shlex.quote(url)}").strip()

          with subtest("a person signs in to the console through the IdP"):
              authorize = location("https://iglu.test/auth/login")
              assert authorize.startswith("${issuer}/api/oidc/authorization"), authorize
              location(authorize)
              body = json.dumps({"username": "alice", "password": "password", "keepMeLoggedIn": False})
              control.succeed(f"{curl} -H 'Content-Type: application/json' -d {shlex.quote(body)} ${issuer}/api/firstfactor")
              callback = location(authorize)
              assert callback.startswith("https://iglu.test/auth/callback?"), callback
              assert location(callback) == "https://iglu.test/", "the callback should land on the console"
              me = json.loads(control.succeed(f"{curl} https://iglu.test/v1/me"))
              assert me["email"] == "alice@example.org", me

          with subtest("cross-origin mutations are refused"):
              status = control.succeed(
                  f"curl -sS -b {jar} -o /dev/null -w '%{{http_code}}' -X POST -H 'Sec-Fetch-Site: cross-site' "
                  "-H 'Content-Type: application/json' -d '{}' https://iglu.test/v1/workspaces"
              )
              assert status == "403", status

          with subtest("the CLI signs in through the console"):
              control.succeed(
                  "systemd-run --unit=cli-login --setenv=HOME=/root "
                  "-p StandardOutput=file:/tmp/login.log -p StandardError=file:/tmp/login.log "
                  "${packages.iglu}/bin/iglu login https://iglu.test"
              )
              control.wait_until_succeeds("grep -q 'Opening ' /tmp/login.log")
              url = found(r"Opening (\S+)", control.succeed("cat /tmp/login.log"))
              page = control.succeed(f"{curl} {shlex.quote(url)}")
              csrf = found(r'name="csrf" value="([^"]+)"', page)
              query = dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(url).query))
              form = " ".join(
                  f"--data-urlencode {shlex.quote(k + '=' + v)}"
                  for k, v in [*query.items(), ("csrf", csrf)]
              )
              loopback = location("https://iglu.test/auth/cli", "-H 'Sec-Fetch-Site: same-origin'", form)
              assert loopback.startswith("http://127.0.0.1:"), loopback
              control.succeed(f"curl -sS {shlex.quote(loopback)}")
              control.wait_until_succeeds("grep -q 'signed in' /tmp/login.log")

          def iglu(args):
              return json.loads(control.succeed(f"HOME=/root iglu --json {args}"))

          with subtest("an environment builds on the host"):
              iglu("env add example 'path:${self}#example'")
              control.wait_until_succeeds(
                  "HOME=/root iglu --json env ls | jq -e '.[] | select(.name == \"example\") | .latest.status == \"ready\"'",
                  timeout=900,
              )

          with subtest("secrets are stored"):
              control.succeed("HOME=/root iglu secret set deploy-key --file .ssh/id_ed25519 < ${pki}/deploy")
              control.succeed("printf hunter2 | HOME=/root iglu secret set token --env TEST_TOKEN")

          with subtest("a workspace starts from the private repository"):
              ws = iglu("new ssh://git@${gitAddress}/srv/git/app.git --env example --name demo --wait")
              assert ws["phase"] == "running", ws
              instance = "iglu-" + ws["id"].replace("-", "")

          def guest(command):
              return host.succeed(f"incus exec {instance} --user 1000 --group 100 --env HOME=/home/dev -- bash -lc {shlex.quote(command)}")

          with subtest("the checkout and secrets are in place"):
              assert "init" in guest("git -C ~/app log --oneline")
              assert guest("git -C ~/app branch --show-current").strip() == "demo"
              guest("test -L ~/.ssh/id_ed25519")

          with subtest("workspaces reach the Internet but not private networks"):
              guest("nc -z -w 5 ${gitAddress} 22")
              guest("! nc -z -w 5 ${nodes.host.networking.primaryIPAddress} 7443")
              guest("! nc -z -w 5 ${nodes.control.networking.primaryIPAddress} 443")

          token = json.loads(control.succeed("cat /root/.config/iglu/credentials.json"))["token"]
          api = f"curl -sS --fail-with-body -H 'Authorization: Bearer {token}'"

          with subtest("a terminal runs in the workspace with secrets in its environment"):
              terminal = json.loads(control.succeed(f"{api} -X POST https://iglu.test/v1/workspaces/{ws['id']}/terminals"))["name"]
              attach = f"wss://iglu.test/v1/workspaces/{ws['id']}/terminals/{terminal}/attach?cols=80&rows=24"
              control.succeed(
                  f"(sleep 2; printf 'echo token=$TEST_TOKEN\\r'; sleep 3) "
                  f"| websocat {shlex.quote(attach)} -b -H 'Authorization: Bearer {token}' > /tmp/terminal.out || true"
              )
              output = control.succeed("cat /tmp/terminal.out")
              assert "token=hunter2" in output, f"the terminal should see the secret: {output!r}"
              sessions = json.loads(control.succeed(f"{api} https://iglu.test/v1/workspaces/{ws['id']}/terminals"))
              assert any(s["name"] == terminal for s in sessions), sessions

          with subtest("a published port is served behind preview sign-in"):
              host.succeed(
                  f"incus exec {instance} --user 1000 --group 100 -- bash -lc "
                  "\"setsid bash -c 'while true; do printf \\\"HTTP/1.1 200 OK\\r\\nContent-Length: 5\\r\\nConnection: close\\r\\n\\r\\nhello\\\" | nc -N -l 3000; done' >/dev/null 2>&1 &\""
              )
              route = iglu("port demo 3000")
              preview = route["url"]
              label = urllib.parse.urlsplit(preview).hostname
              resolve = f"--resolve {label}:443:127.0.0.1 --resolve auth.preview.test:443:127.0.0.1"
              anonymous = control.succeed(f"curl -sS {resolve} -o /dev/null -w '%{{http_code}} %{{redirect_url}}' {preview}")
              assert anonymous.startswith("30") and "auth.preview.test" in anonymous, anonymous
              authorize = location(anonymous.split(" ", 1)[1], resolve)
              assert authorize.startswith("${issuer}/api/oidc/authorization"), authorize
              callback = location(authorize, resolve)
              back = location(callback, resolve)
              assert back.startswith(preview), back
              assert control.succeed(f"{curl} {resolve} {preview}") == "hello"

          with subtest("freezing reclaims memory and thawing resumes"):
              def resident() -> int:
                  return int(host.succeed(f"cat /sys/fs/cgroup/lxc.payload.{instance}/memory.current"))

              before = resident()
              assert iglu("freeze demo --wait")["phase"] == "frozen"
              after = resident()
              assert after < before // 2, f"freezing should reclaim memory: {before} -> {after} bytes"
              assert iglu("start demo --wait")["phase"] == "running"
              assert "init" in guest("git -C ~/app log --oneline")

          with subtest("stopping and deleting clean up the instance"):
              assert iglu("stop demo --wait")["phase"] == "stopped"
              iglu("rm demo --wait")
              host.wait_until_succeeds(f"! incus info {instance}", timeout=120)
      except Exception:
          diagnose()
          raise
    '';
}
