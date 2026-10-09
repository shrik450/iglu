# iglu's development commands. Run them from `nix develop`.
# CI runs the same checks, except the VM test, through the flake; see
# nix/checks.nix.

# List the recipes.
default:
    @just --list

# Format Rust and Nix sources.
fmt:
    cargo fmt --all
    nix fmt

# Check formatting, clippy (pedantic), and the CI workflows.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    git ls-files -z '*.nix' | xargs -0 nixfmt --check
    actionlint
    shellcheck dev/agents/*

# Run the unit tests, and the local runtime's conformance suite.
test:
    cargo build -p iglu-guest
    IGLU_GUEST_TOOLS={{ justfile_directory() }}/target/debug cargo test --workspace

# Type-check and bundle the console.
console:
    cd console && npm ci && npm run build

# Run every flake check: CI's, plus the VM test on x86_64-linux with KVM.
check:
    nix flake check -L

# Run the end-to-end VM test. Needs x86_64-linux with KVM, here or as a remote builder.
e2e:
    nix build .#checks.x86_64-linux.e2e -L --no-link

# Regenerate the console's TypeScript API types from the Rust ones.
api-types:
    rm -rf console/src/generated
    TS_RS_EXPORT_DIR={{justfile_directory()}}/console/src/generated cargo test -p iglu-api --features ts

# The local dev stack: iglud on your machine behind Caddy and Authelia in
# Docker. DEVELOPMENT.md explains it.

# Shared by every worktree of this repository: the dev CA and Authelia's key.
# Outside a Git checkout, such as an unpacked source archive, it's this directory's.
dev_git_dir := `git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true`
dev_shared := (if dev_git_dir == "" { justfile_directory() } else { parent_directory(dev_git_dir) }) / ".dev/shared"
dev_state := justfile_directory() / ".dev/state"
# Local workspaces' runtime directories, which hold terminal sockets. Their
# paths must fit a Unix socket's, which the worktree's path is too long for,
# and /tmp's cleaner would empty them under idle workspaces.
dev_runtime := env_var("HOME") / ".cache/iglu" / replace_regex(sha256(justfile_directory()), "^(.{8}).*", "$1")
dev_ca_name := "iglu dev CA (localhost only)"
dev_keychain := "~/Library/Keychains/login.keychain-db"
dev_compose := "IGLU_DEV_SHARED='" + dev_shared + "' docker compose -f dev/compose.yaml"

# Create the dev CA and Authelia's signing key, if they don't exist yet.
dev-setup:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p '{{ dev_shared }}/ca' '{{ dev_shared }}/authelia/data' '{{ dev_shared }}/git'
    cd '{{ dev_shared }}'
    if [ ! -f ca/ca.crt ]; then
        openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 3650 \
            -subj '/CN={{ dev_ca_name }}' -keyout ca/ca.key -out ca/ca.crt \
            -addext 'basicConstraints=critical,CA:TRUE,pathlen:1' \
            -addext 'keyUsage=critical,keyCertSign,cRLSign' \
            -addext 'extendedKeyUsage=serverAuth' \
            -addext 'nameConstraints=critical,permitted;DNS:localhost,excluded;IP:0.0.0.0/0.0.0.0,excluded;IP:0:0:0:0:0:0:0:0/0:0:0:0:0:0:0:0'
        chmod 600 ca/ca.key
        echo "Created the dev CA. Trust it once with: just dev-trust"
    fi
    if [ ! -f authelia/jwks.pem ]; then
        openssl genrsa -out authelia/jwks.pem 2048
        chmod 600 authelia/jwks.pem
    fi

# Trust the dev CA in your login Keychain, for TLS only. macOS asks for your password.
dev-trust:
    security add-trusted-cert -r trustRoot -p ssl -k {{ dev_keychain }} '{{ dev_shared }}/ca/ca.crt'

# Check that your login Keychain trusts the dev CA for TLS.
dev-check-trust:
    security verify-cert -c '{{ dev_shared }}/ca/ca.crt' -p ssl

# Remove the dev CA and its trust setting from your login Keychain.
dev-untrust:
    security delete-certificate -t -c '{{ dev_ca_name }}' {{ dev_keychain }}

# Replace the dev CA with a new one, trust it, and have Caddy reissue its certificates from it.
dev-replace-ca:
    #!/usr/bin/env bash
    set -euo pipefail
    # The Keychain finds the CA by name, so the old one must go before the new
    # one, which has the same name, is added.
    if security find-certificate -c '{{ dev_ca_name }}' {{ dev_keychain }} >/dev/null 2>&1; then
        security delete-certificate -t -c '{{ dev_ca_name }}' {{ dev_keychain }}
    fi
    rm -rf '{{ dev_shared }}/ca'
    '{{ just_executable() }}' -f '{{ justfile() }}' dev-setup dev-trust dev-services-down dev-services

# Start the shared services: Caddy and Authelia.
dev-services: dev-setup
    {{ dev_compose }} up -d

# Stop the shared services. Caddy reissues its certificates on the next start; Authelia keeps its data.
dev-services-down:
    {{ dev_compose }} down --volumes

# Follow the shared services' logs, or one service's: just dev-logs authelia
dev-logs *service:
    {{ dev_compose }} logs -f {{ service }}

# Run iglud and a local execution host at https://iglu.localhost, rebuilding the console on save.
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    state='{{ dev_state }}'
    clients='{{ justfile_directory() }}/dev/clients'
    mkdir -p "$state"
    if [ ! -f "$state/secret-key" ]; then
        (umask 077 && head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' > "$state/secret-key")
    fi
    cat > "$state/iglud.json" <<JSON
    {
      "listen": "127.0.0.1:7100",
      "console_origin": "https://iglu.localhost",
      "preview_domain": "preview.localhost",
      "database": "$state/iglu.db",
      "secret_key_file": "$state/secret-key",
      "console_assets": "{{ justfile_directory() }}/console/dist",
      "oidc": {
        "issuer": "https://auth.localhost",
        "console": { "client_id": "console", "client_secret_file": "$clients/console.secret" },
        "preview": { "client_id": "preview", "client_secret_file": "$clients/preview.secret" },
        "worker": { "client_id": "worker", "client_secret_file": "$clients/worker.secret" }
      },
      "sign_in": [
        { "verified_email": "alice@example.org" },
        { "verified_email": "bob@example.org" }
      ],
      "hosts": [ { "id": "local", "url": "http://127.0.0.1:7200" } ]
    }
    JSON
    # The local runtime doesn't meter memory; it reports this much as free.
    cat > "$state/devhost.json" <<JSON
    {
      "host_id": "local",
      "listen": "127.0.0.1:7200",
      "auth": { "issuer": "https://auth.localhost", "audience": "iglu-hosts", "subjects": ["worker"] },
      "runtime": {
        "state_dir": "$state/devhost",
        "runtime_dir": "{{ dev_runtime }}",
        "guest_tools": "{{ justfile_directory() }}/target/debug",
        "memory_available": 68719476736,
        "localhost_ca": "{{ dev_shared }}/ca/ca.crt",
        "agents": [
          { "name": "claude", "command": ["claude"], "prompt": "argument", "attention": "claude-hooks" },
          { "name": "haiku", "command": ["claude", "--model", "claude-haiku-5-5"], "prompt": "argument", "attention": "claude-hooks" },
          { "name": "scripted", "command": ["{{ justfile_directory() }}/dev/agents/scripted"], "prompt": "argument", "attention": "status-command" }
        ]
      }
    }
    JSON
    cargo build -p iglud -p iglu-devhost -p iglu-guest
    # npm records the lock file it installed from; reinstall when the lock file has changed since.
    [ console/node_modules/.package-lock.json -nt console/package-lock.json ] || (cd console && npm ci)
    (cd console && exec node build.mjs --watch) &
    watcher=$!
    target/debug/iglu-devhost --config "$state/devhost.json" &
    devhost=$!
    trap 'kill $watcher $devhost 2>/dev/null' EXIT
    echo "iglu: https://iglu.localhost (alice or bob, password \"password\")"
    target/debug/iglud --config "$state/iglud.json"

# Fill the running dev stack with workspaces in every state worth seeing; run it after dev-reset.
dev-seed:
    python3 dev/ui/seed.py

# Screenshot every view of the seeded console, with its accessibility tree, audit and errors: just dev-tour --quick
dev-tour *args:
    python3 dev/ui/tour.py {{ args }}

# Show what changed between two tours: just dev-compare before after
dev-compare before after:
    python3 dev/ui/compare.py {{ before }} {{ after }}

# Run the VM test's browser steps against the running dev stack.
dev-check:
    python3 dev/ui/check.py

# Run the CLI against the dev stack, signed in separately from your real iglu: just dev-cli login https://iglu.localhost
[positional-arguments]
dev-cli *args:
    XDG_CONFIG_HOME='{{ dev_state }}/config' cargo run -q -p iglu-cli -- "$@"

# Delete the dev stack's database, settings and workspaces, ending their terminal sessions, so the next run starts fresh.
dev-reset:
    #!/usr/bin/env bash
    set -euo pipefail
    for sessions in '{{ dev_runtime }}'/*/zmx; do
        [ -d "$sessions" ] || continue
        ZMX_DIR="$sessions" zmx list 2>/dev/null | cut -f1 | sed 's/^session_name=//' |
            while read -r session; do ZMX_DIR="$sessions" zmx kill "$session" || true; done
    done
    rm -rf '{{ dev_state }}' '{{ dev_runtime }}'
