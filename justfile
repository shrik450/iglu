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

# Run the unit tests.
test:
    cargo test --workspace

# Type-check and bundle the console.
console:
    cd console && npm ci && npm run build

# Run every flake check: CI's, plus the VM test on x86_64-linux with KVM.
check:
    nix flake check -L

# Run the end-to-end VM test. Needs x86_64-linux with KVM, here or as a remote builder.
e2e:
    nix build .#checks.x86_64-linux.e2e -L --no-link
