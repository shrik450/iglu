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
.github/workflows/ # CI, mutation testing, coverage, and the weekly deep run
```

## Commands

Everything comes from the flake. Don't install tools globally. Enter the dev
shell with `nix develop`, then use `just`:

```sh
just fmt            # format Rust and Nix
just lint           # rustfmt, clippy (pedantic, warnings are errors), nixfmt, actionlint
just test           # unit and property tests, the conformance suite, a short simulation; fast
just mutants        # mutation-test what you changed against main, as pull requests are
just console        # type-check, unit-test and bundle the console
just terminal-check # the console's terminal in Chromium and WebKit, without a stack
just dev-check      # the console in a browser against a running dev stack (`just dev`)
just api-types      # regenerate console/src/generated after changing iglu-api; a check fails if stale
just check          # every flake check, the VM test included on x86_64-linux
just e2e            # the VM test; needs x86_64-linux with KVM, which workspaces lack, so CI runs it
just dev            # run iglud and the console locally; see DEVELOPMENT.md
```

`DEVELOPMENT.md` explains each tier of tests, the local dev stack and its
`dev-*` recipes, and the dev CA it uses for HTTPS.

Look at console changes before calling them done. With the dev stack running
and seeded (`just dev-seed`), `just dev-tour` screenshots every view in Chromium
and WebKit, on desktop and phone, in both themes, with accessibility audits,
and `just dev-compare` shows what a change touched.

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

- Unit-test pure functions only. Most live in the core; a pure helper in a
  shell crate is tested where it is, or moved to the core. Test behavior
  through the public API, not implementation details.
- Test everything else against real systems: real Incus and Btrfs, real
  SQLite, a real IdP, real processes and a real browser. No mocks of our own
  code or of external systems. The one stand-in is the simulated runtime,
  and it earns its place the way every runtime does: it passes the
  conformance suite.
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
  the console alike: the default is quick enough for `just test`, and the
  weekly deep run raises it to 5,000.
- When a property fails, proptest saves the case under
  `proptest-regressions/`. Commit those files: the case then runs first,
  everywhere, from then on. Fix the code, not the property; when a
  generator was wrong instead, fix the generator and delete what it saved.

### Where a test goes

Put each test in the cheapest tier that can see the bug:

| What changed or broke                                                     | Where its test goes                                                                                                  |
| ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| A pure function                                                           | Unit tests and properties beside it, or in the crate's `tests/properties/`; `*.properties.test.ts` in the console      |
| Something hostd relies on from every runtime that the types can't say     | A check in `crates/iglu-hostd/src/conformance.rs`, which the local, simulated and Incus runtimes all run             |
| iglud and hostd together under faults, restarts and reordering            | An invariant or seeded test in the simulation, `crates/iglud/src/sim.rs`                                             |
| The terminal itself: what it draws, sends or answers                      | A case in the terminal tier, `console/test/terminal/`                                                                |
| The console with the rest of iglu around it: sessions, iglud, its views   | A step in `nix/tests/browser.py`, added to `dev/ui/check.py`                                                         |
| What only a real host shows: the NixOS modules, the workspace image, Incus | The VM test, `nix/tests/e2e.py`; only browser steps that need a real host run there                                 |

A failure the simulation finds replays from its seed. Keep it as a seeded
test, and keep that test once the bug is fixed, as the regression check.

### What a pull request must pass

These run on every pull request, and each must pass to merge:

- **CI** (`.github/workflows/ci.yml`): every flake check, which covers lint,
  the unit, property and conformance tests, the short simulation, the
  console's unit tests and the terminal tier; `just dev-check` against a dev
  stack, the CLI included; the VM test; and the packages built for
  aarch64-linux. Adding a check to `nix/checks.nix` adds it to CI.
- **Mutation** (`.github/workflows/mutation.yml`): the tests must catch
  every mutant in the lines the pull request changes, in the core, runtime
  and console tiers. A missed mutant means a test is missing; add it, rather
  than leave the behaviour unchecked. A mutant that can't change behaviour
  is excluded where it is, with the reason (see `DEVELOPMENT.md`).

- **Coverage** (`.github/workflows/coverage.yml`): at least 90% of the
  lines the pull request changes must run in some tier the dev shell
  measures, or no more than 3 of them may not. Tests, harnesses and code
  only the VM test runs don't count (`gated()` in `dev/cover.py`). An
  uncovered line wants a test in the cheapest tier that reaches it.

Totals of coverage and mutation score are reported and tracked, not gated:
the bar is on what a change touches. The weekly deep run (`.github/workflows/deep.yml`), which runs the properties
at 5,000 cases and the simulation over 20,000 schedules; when it fails, fix
what it found.

Before pushing, run `just lint test` and `just mutants`, and `just
dev-check` when you change the console or the CLI. `just coverage` and
`just coverage-gate` check the coverage bar, in about ten minutes.

## Change hygiene

- Hard cutovers. No backwards-compatibility shims, fallbacks, or dual code paths
  unless explicitly requested. Schema and protocol changes update both sides.
- Declare every dependency in Nix. Don't rely on system packages.
- Keep comments for why, not what. Match the surrounding code.
- Don't commit plans, specs, scratch files, or research notes.
