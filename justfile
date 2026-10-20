# iglu's development commands. Run them from `nix develop`.
# CI runs the same checks through the flake (nix/checks.nix), dev-check
# against a dev stack, and the mutation gate; see .github/workflows/.

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

# Run the unit and property tests, the conformance suite on the local and simulated runtimes, and a short simulation.
test:
    cargo build -p iglu-guest
    IGLU_GUEST_TOOLS={{ justfile_directory() }}/target/debug cargo test --workspace

# Type-check, unit-test and bundle the console.
console:
    cd console && npm ci && npm run build

# Check the console's terminal in Chromium and WebKit, without a stack, on newly generated output: just terminal-check --browser chromium mac-keys
terminal-check *args:
    cd console && npx tsc -b vendor/wterm-dom && npx tsc --noEmit -p test/terminal
    python3 console/test/terminal/run.py {{ args }}

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

# Mutation testing. DEVELOPMENT.md explains the tiers, and which gate pull requests.

# Mutate what changed against a base in the tiers that gate pull requests, and fail on a mutant the tests miss: just mutants
mutants base="main":
    #!/usr/bin/env bash
    set -uo pipefail
    mkdir -p .dev/mutants
    diff=.dev/mutants/changes.diff
    git diff --merge-base "{{ base }}" > "$diff"
    failed=0
    # cargo-mutants exits 2 for missed mutants and 3 for timeouts, which
    # count as caught: the tests didn't pass.
    rust() {
        just mutants-tier "$1" --in-diff "$diff" --output ".dev/mutants/$1"
        case $? in 0 | 3) ;; *) failed=1 ;; esac
    }
    rust core
    if grep -qE '^\+\+\+ b/crates/iglu-(hostd|devhost)/' "$diff"; then rust runtime; fi
    (cd console && node stryker-diff.mjs "{{ base }}") || failed=1
    exit $failed

# Mutate one tier in full, core, runtime, shell or console, passing the rest on: just mutants-tier core --shard 0/4
[positional-arguments]
mutants-tier tier *args:
    #!/usr/bin/env bash
    set -euo pipefail
    shift
    # Results go under .dev/mutants, and each mutant is built in a copy of
    # the tree, made here rather than in /tmp.
    mkdir -p .dev/mutants .dev/tmp/mutants
    export TMPDIR="$PWD/.dev/tmp/mutants"
    case "{{ tier }}" in
        core) exec cargo mutants -p iglu-domain -p iglu-proto -p iglu-api "$@" ;;
        runtime)
            # hostd's rules and the local runtime, tested by the conformance
            # suite, which runs the guest tools built here.
            cargo build -p iglu-guest
            IGLU_GUEST_TOOLS="$PWD/target/debug" exec cargo mutants -p iglu-hostd -p iglu-devhost \
                --test-package iglu-hostd --test-package iglu-devhost "$@" ;;
        shell) exec cargo mutants -p iglud -p iglu-guest -p iglu-cli "$@" ;;
        console) cd console && exec npx stryker run "$@" ;;
        *) echo "tiers: core, runtime, shell, console" >&2; exit 1 ;;
    esac

# Coverage, by dev/cover.py. DEVELOPMENT.md explains the tiers.

# Measure coverage of every tier, running a dev stack of its own (stop `just dev` first); reports land in .dev/coverage/.
coverage:
    python3 dev/cover.py all

# Show coverage of the lines changed since a base, after `just coverage`: just coverage-diff main
coverage-diff base="main":
    python3 dev/cover.py diff "{{ base }}"

# Check the changed lines against the bar pull requests must meet, after `just coverage`: just coverage-gate main
coverage-gate base="main":
    python3 dev/cover.py gate "{{ base }}"

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
