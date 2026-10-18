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
├── iglu-hostd/    # execution-host daemon: hostd's rules over a Runtime, with Incus for production
├── iglu-devhost/  # hostd over local processes, for the dev stack; not isolating, loopback only
├── iglud/         # control plane: API, console, preview gateway, OIDC, reconciler
├── iglu-cli/      # the `iglu` command
└── iglu-guest/    # tools hosts mount into every workspace (sessions, status, provisioning, secrets)
console/           # TypeScript web console (wterm terminals); src/generated is from iglu-api
nix/               # NixOS modules, packages, checks, and VM integration tests
templates/         # `nix flake init -t` starting points for environments
dev/               # the local dev stack (dev/stack.py), its Caddy and Authelia configs, and UI tools
justfile           # development commands
.github/workflows/ # CI: the flake checks, the console against the dev stack, and the VM test
```

## Commands

Everything comes from the flake. Don't install tools globally. Enter the dev
shell with `nix develop`, then use `just`:

```sh
just fmt       # format Rust and Nix
just lint      # rustfmt, clippy (pedantic, warnings are errors), nixfmt, actionlint
just test      # the unit tests and the local runtime's conformance suite; fast
just console   # type-check and bundle the console
just api-types # regenerate console/src/generated after changing iglu-api; a check fails if stale
just check     # every flake check: CI's, plus the VM test on x86_64-linux
just e2e       # the VM test; needs x86_64-linux with KVM, which workspaces lack, so CI runs it
just dev       # run iglud and the console locally; see DEVELOPMENT.md
```

`DEVELOPMENT.md` explains the local dev stack, its `dev-*` recipes, and the
dev CA it uses for HTTPS.

Look at console changes before calling them done. With the dev stack running
and seeded (`just dev-seed`), `just dev-tour` screenshots every view in Chromium
and WebKit, on desktop and phone, in both themes, with accessibility audits;
`just dev-compare` shows what a change touched; and `just dev-check` checks
the console in a browser. A console bug fix gets a test where the bug lives.
A bug in the terminal itself, in what it draws, sends or answers, gets a
case in the terminal tier, `console/test/terminal/run.py`, which plays the
program against the console's own terminal in Chromium and WebKit without a
stack (`just terminal-check`). A bug that needs the rest of iglu, such as a
session, iglud or the views around a column, gets a step in
`nix/tests/browser.py`, run by `dev/ui/check.py`; the VM test runs only the
steps that depend on a real host or the workspace image.

CI runs three jobs on pull requests and on `main`, and each must pass to
merge (`.github/workflows/ci.yml`): every flake check but the VM test; `just
dev-check` against a dev stack; and the VM test, which checks iglu as
deployed: the NixOS modules, the workspace image, and Incus and its
isolation. The checks live in `nix/checks.nix` and the flake, so adding one
there adds it to CI. Run `just lint test` before pushing, and `just dev-check`
after changing the console.

Nothing about the dev stack or its tools may depend on the machine it runs
on beyond Nix: no system packages, daemons, privileges or OS trust stores.
Workspaces, laptops and CI all run it the same way.

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

### Runtimes

- hostd runs workspaces through `Runtime` (`crates/iglu-hostd/src/runtime.rs`),
  in hostd's vocabulary, never Incus's. A new host feature lands in that
  contract first: a typed method or a `GuestCommand`/`GuestFile` variant, so
  every runtime fails to build until it handles it. Behaviour the types
  can't express goes in the conformance suite (`conformance.rs`), which each
  runtime runs in its real environment.
- Only an `Isolating` runtime may serve beyond loopback. A runtime earns it
  with negative isolation tests of its own, like `incus/checks.rs`.

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
- State what a pure function promises as a property, checked on generated
  inputs, alongside the examples that show it: proptest in Rust, each
  crate's under `tests/properties/` or in the module for private
  functions, and fast-check in the console, in `*.properties.test.ts`.
  The properties that pay are round trips (what's parsed, shown, sent or
  stored reads back the same), never panicking on untrusted input,
  invariants (bounds, no control characters, nothing reaching another
  host), agreement with the system that consumes the output (a browser's
  URL parser, Git's credential format), and, for anything fed in chunks,
  that splitting the input anywhere changes nothing. Finite domains, such
  as the access rules, are checked on every combination instead.
- `PROPTEST_CASES` sets how many cases every property runs, in Rust and
  the console alike: the default is quick enough for `just test`, and CI's
  deeper run raises it.
- When a property fails, proptest saves the case under
  `proptest-regressions/`. Commit those files: the case then runs first,
  everywhere, from then on. Fix the code, not the property; when a
  generator was wrong instead, fix the generator and delete what it saved.

## Change hygiene

- Hard cutovers. No backwards-compatibility shims, fallbacks, or dual code paths
  unless explicitly requested. Schema and protocol changes update both sides.
- Declare every dependency in Nix. Don't rely on system packages.
- Keep comments for why, not what. Match the surrounding code.
- Don't commit plans, specs, scratch files, or research notes.
