# AGENTS.md

iglu gives each coding task its own NixOS container ("workspace") with persistent
terminals in the browser and previews on a subdomain. See `README.md` for the
architecture. This file is the contract for anyone, human or agent, changing the
code.

## Layout

```text
crates/
├── iglu-domain/   # pure core: domain types, parsing, planning, policy. No I/O.
├── iglu-proto/    # the iglud ⇄ hostd wire protocol, built from domain types
├── iglu-api/      # iglud's HTTP API types, shared with the CLI and, generated, the console
├── iglu-hostd/    # execution-host daemon: the only thing that talks to Incus
├── iglud/         # control plane: API, console, preview gateway, OIDC, reconciler
├── iglu-cli/      # the `iglu` command
└── iglu-guest/    # tools baked into workspace images (status, provisioning, git credentials)
console/           # TypeScript web console (ghostty-web terminals); src/generated is from iglu-api
nix/               # NixOS modules, packages, checks, and VM integration tests
templates/         # `nix flake init -t` starting points for environments
dev/               # the local dev stack's shared services (Caddy, Authelia), run in Docker
justfile           # development commands
.github/workflows/ # CI, which runs the flake checks except the VM test
```

## Commands

Everything comes from the flake. Don't install tools globally. Enter the dev
shell with `nix develop`, then use `just`:

```sh
just fmt       # format Rust and Nix
just lint      # rustfmt, clippy (pedantic, warnings are errors), nixfmt, actionlint
just test      # the unit tests; fast, run anywhere
just console   # type-check and bundle the console
just api-types # regenerate console/src/generated after changing iglu-api; a check fails if stale
just check     # every flake check: CI's, plus the VM test on x86_64-linux
just e2e       # the VM test; needs x86_64-linux with KVM, locally or as a remote builder
just dev       # run iglud and the console locally; see DEVELOPMENT.md
```

`DEVELOPMENT.md` explains the local dev stack, its `dev-*` recipes, and the
dev CA it uses for HTTPS.

CI runs every flake check except the VM test on pull requests and on `main`
(`.github/workflows/ci.yml`). The checks live in `nix/checks.nix` and the
flake, so adding one there adds it to CI. Run `just lint test` before pushing.

The VM test is too heavy for hosted runners, so it runs only locally. Run
`just e2e` before merging changes to hostd, the guest tools, the console, the NixOS
modules, or anything else that touches a real host or guest.

Clippy's pedantic group is on for the whole workspace. Fix what it finds.
When a lint is wrong for a specific item, silence it there with
`#[expect(clippy::..., reason = "...")]`, never a crate-wide `allow`.

## Design rules

These follow from one goal: good software, meaning understandable, evolvable and
reasonable, not only correct.

### Functional core, imperative shell

- `iglu-domain` is the functional core. It has no async, no I/O, no clocks, no
  randomness, and no logging. It takes values and returns values. If a function
  needs the time, a random seed, or the state of the world, the shell passes it in.
- Decisions live in the core: `plan()` decides the next lifecycle effect,
  `authorize()` decides access, `admit()` decides capacity, and the preview
  rules decide which requests cross workspace boundaries. The shell gathers
  inputs, calls the decision, and performs the result. Shell code should read as
  "observe, decide, act", with no business rules hidden inside it.
- Binaries are thin shells. Keep each shell module about one external system
  (Incus, SQLite, the IdP, HTTP).

### Model the domain with types

- Make illegal states unrepresentable. Prefer an enum with data over a struct of
  optional fields or flags. Prefer two enums over a bool parameter.
- Give every domain value its own newtype: IDs, names, ports, paths, flake
  references, secrets. A `String` or `u16` in a signature that means something
  more specific is a bug waiting to happen.
- Match domain enums exhaustively. Don't use `_ =>` on an enum you own, so that
  adding a variant is a compile error everywhere it matters.
- Keep types honest about ownership: what Incus owns is observed, never assumed;
  what SQLite owns is intent.

### Parse, don't validate

- Parse untrusted input once, at the boundary, into a domain type whose
  constructor guarantees the invariant. After that, code relies on the type and
  never re-checks.
- Constructors that can fail are `TryFrom`/`FromStr` and return a typed error.
  Types that cross the wire use `#[serde(try_from = "...")]` so deserialization
  is parsing.
- Guest output is untrusted. Incus responses, guest files, and IdP responses
  are parsed like any other external input.

### Errors

- Libraries return typed errors (`thiserror`). Binaries may use `anyhow` at the
  top level only.
- No `unwrap()` outside tests. `expect()` is allowed only for invariants the
  types can't express, with a message that says why it holds.
- No lossy `as` casts. Use `TryFrom` and handle the failure.
- Avoid defensive checks for states the types already rule out. If you feel the
  need for one, fix the type instead.

### Security boundaries

- `hostd` exposes a fixed set of typed commands. Never build a shell command,
  host path, or Incus config from request data; map typed values to fixed
  argv or API calls.
- Secrets never appear in logs, errors, activity records, the Nix store, or
  images.
- Every protected request goes through `authorize()`. The preview gateway and
  console hardening rules live in the core so they can be tested exhaustively.

## Testing

- Unit-test pure functions only, which means the core. Test behavior through
  the public API, not implementation details.
- Test everything else against real systems: real Incus and Btrfs, real SQLite,
  a real IdP, and a real browser, in NixOS VM tests under `nix/tests/`. No mocks
  of our own code or of external systems.
- A test that asserts implementation details is worse than no test.
- Negative tests matter as much as positive ones for isolation and access
  control.

## Change hygiene

- Hard cutovers. No backwards-compatibility shims, fallbacks, or dual code paths
  unless explicitly requested. Schema and protocol changes update both sides.
- Declare every dependency in Nix. Don't rely on system packages.
- Keep comments for why, not what. Match the surrounding code.
- Don't commit plans, specs, scratch files, or research notes.
