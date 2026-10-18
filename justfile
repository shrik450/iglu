# iglu's development commands. Run them from `nix develop`.
# CI runs the same checks through the flake (nix/checks.nix), and dev-check
# against a dev stack; see .github/workflows/ci.yml.

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

# The local dev stack: iglud, a local host, Caddy and Authelia, all run from
# the dev shell by dev/stack.py. DEVELOPMENT.md explains it.

# Run the dev stack at https://iglu.localhost:7300, rebuilding the console on save.
dev:
    python3 dev/stack.py run

# Wait until a dev stack started elsewhere answers.
dev-wait:
    python3 dev/stack.py wait

# Trust the dev CA in your own browsers, for TLS only. The stack doesn't need it.
dev-trust:
    python3 dev/stack.py trust

# Remove the dev CA from your browsers' trust.
dev-untrust:
    python3 dev/stack.py untrust

# Replace the dev CA with a new one; untrust the old one first, and trust the new one after.
dev-replace-ca:
    python3 dev/stack.py replace-ca

# Fill the running dev stack with workspaces in every state worth seeing; run it after dev-reset.
dev-seed:
    python3 dev/ui/seed.py

# Screenshot every view of the seeded console, with its accessibility tree, audit and errors: just dev-tour --quick
dev-tour *args:
    python3 dev/ui/tour.py {{ args }}

# Show what changed between two tours: just dev-compare before after
dev-compare before after:
    python3 dev/ui/compare.py {{ before }} {{ after }}

# Check the console in a real browser against the running dev stack, as CI does.
dev-check:
    python3 dev/ui/check.py

# Run the CLI against the dev stack, signed in separately from your real iglu: just dev-cli login https://iglu.localhost:7300
[positional-arguments]
dev-cli *args:
    python3 dev/stack.py cli "$@"

# Delete the dev stack's database, settings and workspaces, ending their terminal sessions, so the next run starts fresh.
dev-reset:
    python3 dev/stack.py reset
