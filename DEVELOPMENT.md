# Developing iglu

This guide covers working on iglu on your own machine: the tools, the checks,
and the local dev stack, which runs iglud and the console against a real
identity provider over HTTPS in seconds. `AGENTS.md` holds the design rules
every change follows; read it first.

## Tools

You need [Nix](https://nixos.org) with flakes, on Linux or macOS. Every tool
comes from the dev shell: run `nix develop`, then use `just`. Nothing is
installed globally, no daemon runs outside the dev stack, and nothing needs
root.

## Checks

| Command          | What it does                                                     |
| ---------------- | ---------------------------------------------------------------- |
| `just fmt`       | Formats Rust and Nix.                                            |
| `just lint`      | rustfmt, clippy (pedantic, warnings are errors), nixfmt, actionlint. |
| `just test`      | The unit tests, and the local runtime's conformance suite. Fast. |
| `just console`   | Type-checks and bundles the console.                             |
| `just dev-check` | Checks the console in a browser against the running dev stack.   |
| `just e2e`       | The VM test. Needs x86_64-linux with KVM.                        |
| `just check`     | Every flake check, including the VM test where it can run.       |

CI runs three jobs, and a pull request merges only when all pass: the flake
checks except the VM test, `just dev-check` against a dev stack, and the VM
test (`.github/workflows/ci.yml`). Run `just lint test` before you push, and
`just dev-check` when you change the console. The VM test needs KVM, which
workspaces don't have, so CI is usually where it runs.

The two browser checks split the work. `just dev-check` covers the console's
behaviour, over the local host. The VM test covers iglu as it's deployed: the
NixOS modules, the workspace image and its guest tools, Incus and its
isolation, and a handful of the same browser steps through all of them.

## The local dev stack

The dev stack runs all of iglu on your machine, so you can work on the
console and iglud's API, and use workspaces, without a real execution host.
`just dev` runs it all from the dev shell, as its own child processes:

- **iglud**, built from your checkout. It serves the console from
  `console/dist`, which rebuilds whenever you save.
- **iglu-devhost**: hostd with workspaces as local processes instead of
  Incus containers. See [The local host](#the-local-host).
- **Authelia**, as the identity provider. Its configuration follows the VM
  test's.
- **Caddy**, in front of everything. It terminates TLS the way it does on a
  real control box, and routes each hostname to the right service.

```text
browser ──► Caddy (port 7300, TLS from the dev CA)
              ├─ auth.localhost                           ──► Authelia (port 7301)
              ├─ git.localhost                            ──► fixture repositories
              └─ iglu.localhost, *.preview.iglu.localhost ──► iglud (port 7100)
                                                                └─► iglu-devhost (port 7200)
```

Every hostname is under `localhost`, which browsers, macOS and Linux resolve
to your own machine, so there's no DNS to set up. Everything listens on
loopback only, on ports above 1024, so nothing needs privileges.
`dev/stack.py` runs the stack and holds its names, ports and addresses, which
it passes to Caddy and Authelia.

Previews are under the console's name because a deployment's previews share
its console's registrable domain, and the preview rules in
`iglu-domain/src/preview.rs` are there for what follows: browsers call a
preview's requests to the console *same-site*. Browsers count `localhost` as
the suffix, so `iglu.localhost` is a site, and under it the dev stack's
previews are same-site with the console too. Under `preview.localhost` they
were cross-site, which `SameSite` cookies block anyway, so the console's
checks couldn't tell whether those rules work.

### The local host

iglu-devhost runs hostd's own code, with its rules, commands and protocol,
over a local runtime instead of Incus. A workspace is a directory, and its
processes run as you. Each workspace gets its own `HOME`, with the repository
cloned into it, and its own `XDG_RUNTIME_DIR`, which holds its delivered
secrets, attention status and terminal sessions. Terminals are real zmx
sessions running your login shell, so they survive restarts of `just dev`.

Workspaces use your machine's own tools. Your Nix profile is linked into
each workspace's home, so a login shell finds your tools there, and the
guest tools are on its `PATH`. Claude Code's hooks are set up in each home
and report attention to the console as they do in a real workspace.

Environments aren't built. Local workspaces don't use a NixOS image, and a
Mac can't build one, so adding or rebuilding an environment just records it,
and it's ready at once. Any flake reference works.

What's different from a real host:

- **No isolation.** Workspaces can read your files, reach your network and
  share your loopback ports, so two workspaces can't both serve port 3000.
  The devhost serves only on loopback, and hostd refuses to serve a runtime
  that doesn't isolate anywhere else.
- **No memory metering.** The host always reports 64 GiB available. Lower
  `memory_available` in `dev/stack.py` and restart to see workspaces wait
  for room.
- **Freezing** pauses a workspace's processes with `SIGSTOP`. A process that
  leaves the workspace's process tree and clears its environment escapes it.
- **Browser columns need Linux.** A local browser column runs the dev
  shell's Chromium on a headless sway, shown over wayvnc, and sway is
  Linux's: on a Mac, environments have no browser. Only one local workspace
  can have one, since its debugging port, 9222, and its display's, 5900, are
  on the shared loopback. The VM test checks the browser.
- **Restarting your machine** stops every workspace, like a host restart. A
  workspace whose runtime directory is deleted while it runs reads as failed
  until you start it again, which ends what was left of it first.

The runtime conformance suite checks that the local runtime behaves like
Incus wherever hostd relies on it; `just test` runs it.

### Running it

```sh
just dev
```

Then open `https://iglu.localhost:7300` and sign in as `alice`, `bob` or
`carol`, all with the password `password`. Several people let you try the
console as different owners; the dev tools never touch carol, so she sees
iglu as someone new does.

`just dev` creates what's missing (the dev CA, Authelia's key, iglud's
secrets), builds iglud, the devhost and the guest tools, and runs everything
in the foreground. Ctrl-C stops it all. Caddy and Authelia log to
`.dev/state/logs/`; the rest log to the terminal. Edit anything under
`console/src` and refresh the page; the bundle is unminified and has a source
map. The page, `console/src/index.html`, is rewritten on each rebuild, so a
change to it alone shows after the next one. A change to iglud or hostd needs
a restart.

Your browser warns about the certificate until you trust the dev CA; see
[Local HTTPS and the dev CA](#local-https-and-the-dev-ca). The dev tools
don't need it.

### Where things are

| What        | Value                                                      |
| ----------- | ---------------------------------------------------------- |
| Console     | `https://iglu.localhost:7300`                              |
| Previews    | `https://<route>.preview.iglu.localhost:7300`              |
| iglud       | `127.0.0.1:7100`                                           |
| Local host  | `127.0.0.1:7200`                                           |
| State       | `.dev/state/` in the current worktree                      |
| Workspaces  | `.dev/state/devhost/instances/<instance>/home`             |
| Kept        | `.dev/state/devhost/kept/<person>`                         |
| Sessions    | `~/.cache/iglu/<worktree hash>/<slot>/`                    |
| Fixtures    | `https://git.localhost:7300`, from `.dev/shared/git/`      |
| Logs        | `.dev/state/logs/`, for Caddy and Authelia                 |
| Tours       | `.dev/tour/<label>/index.html`                             |

iglud's state is its database, its secret-sealing key and the dev CLI's login,
and the devhost's is your workspaces. It lives in the worktree you run
`just dev` from, because each branch can have its own schema. Delete it, and
end every workspace's sessions, with `just dev-reset`. Only one worktree can
run the stack at a time, since they all use the same port and hostnames.

Each workspace's runtime directory, which holds its terminal sessions and
delivered secrets, is under `~/.cache/iglu/`, named by a short number
rather than the workspace. Terminal sessions are Unix sockets, whose paths
are limited to about 100 bytes, which a path inside the worktree is too long
for. The devhost refuses to start if your home directory is too long for the
longest session name.

Every worktree shares one dev CA, Caddy's certificates, and Authelia's key and
data. They live in `.dev/shared/` of the main checkout, so every worktree uses
the same ones. Authelia's data matters: it assigns each person
the subject identifier iglud knows them by, so deleting it makes alice and bob
new people to every worktree's iglud. If that happens, run `just dev-reset` in
each.

### The CLI

`just dev-cli` runs the CLI against the dev stack, with a login kept in its
state instead of your real `~/.config/iglu`. It trusts the dev CA as your
system does: on Linux it's told to, and on macOS it needs `just dev-trust`.

```sh
just dev-cli login https://iglu.localhost:7300
just dev-cli ls
```

### Seeing and checking the console

The console is checked in real browsers, Chromium and WebKit, which the dev
shell provides through Playwright. Four recipes drive the running stack the
way a person would:

```sh
just dev-reset && just dev   # a fresh stack, in one terminal
just dev-seed                # then, in another, fill it
just dev-tour --quick        # screenshot every view; .dev/tour/<label>/index.html
just dev-check               # the console's browser checks, as CI runs them
```

- **`just dev-seed`** makes fixture repositories and signs in as alice and
  bob to build workspaces in every state worth looking at: an agent waiting,
  working, done and crashed; uncommitted and unpushed work; a published
  preview; frozen, stopped, and a clone that fails. Caddy serves the fixture
  repositories at `git.localhost` from `.dev/shared/git/`, and the devhost
  has workspaces' Git trust the dev CA for `*.localhost` only.
- **`just dev-tour`** visits every view, dialog and seeded workspace on
  desktop and phone, in dark and light, in Chromium and WebKit. For each it
  keeps a screenshot, console errors and horizontal overflow; for the
  desktop views it also keeps the accessibility tree and an axe audit. Add
  `--quick` for desktop and dark only, `--only <text>` for some stops, and
  `--label <name>` to name the run.
- **`just dev-compare <before> <after>`** marks what changed between two
  tours, most changed first, so a change shows everything it touched.
- **`just dev-check`** runs `nix/tests/browser.py`'s steps against the local
  stack in about two minutes, as CI does. A console fix gets a step there,
  added to `dev/ui/check.py`; add it to the VM test too only when it depends
  on a real host or the workspace image.

The tools' browsers accept the dev stack's certificates without trusting the
dev CA, so they work the same on every machine.

Agents run in a shell, as anywhere: `claude`, or `claude --model
claude-haiku-5-5` to try things cheaply. `dev/agents/scripted` is a stand-in
that does no work. Run in a shell, it reports whichever attention state you
type, such as `wait Approve the migration` or `done Fixed it`, so agent
states can be shown and tested without a real agent; it lists what it
understands.

### Troubleshooting

- **`the identity provider isn't reachable yet`** in iglud's log, briefly at
  start. Authelia is still starting. If it stays, read
  `.dev/state/logs/authelia.log`.
- **An address is in use.** Another dev stack, from another worktree, is
  running, or something else uses port 7100, 7200, 7300 or 7301.
- **A certificate warning in your browser.** The dev CA isn't trusted there.
  Run `just dev-trust`, or accept the warning.
- **iglud behaves strangely after switching branches.** Its database may be
  from another schema. Run `just dev-reset`.

## Local HTTPS and the dev CA

The dev stack runs over HTTPS because iglu needs it, not for show:

- iglud expects TLS to end in front of it, and its preview sign-in only accepts
  `https` addresses.
- Its cookies are `Secure`, and the preview cookie spans the preview domain.
- Authelia refuses to run its sign-in over plain HTTP.

Faking HTTPS away would mean a development-only path through iglu's security
code, so the stack uses real certificates instead. Something has to issue
them, and whatever connects has to trust the issuer:

- **iglud** trusts it through `extra_ca_certificates` in its configuration,
  a setting any deployment with a private CA can use.
- **Workspaces' Git** trusts it for `https://*.localhost` only, so local
  workspaces can clone fixtures.
- **The dev tools' browsers** accept the stack's certificates without it.
- **Your own browser** trusts it only if you choose to; see
  [Trusting it](#trusting-it).

### What the CA can sign

`just dev` creates a certificate authority, the dev CA, the first time it
runs. Caddy issues every certificate in the stack from it. Its certificate
limits what it can vouch for in three ways:

- **Names.** An X.509 name constraint permits only the DNS name `localhost`
  and names under it, such as `iglu.localhost`.
- **IP addresses.** The same constraint excludes every IPv4 and IPv6 address,
  so it can't vouch for a site reached by address, such as `https://192.168.1.1`.
  A constraint on DNS names alone wouldn't cover addresses.
- **Purpose.** Its extended key usage is TLS server authentication only.

`just dev-trust` adds a matching limit on macOS: the Keychain trusts the CA
for TLS only, not for signed email, code signing or anything else.

These limits are what make trusting the CA safe, and browsers and macOS
enforce them: with the CA trusted, a certificate it signs for `example.com`
or for `192.168.1.1` fails verification. So if the CA's key ever leaked, it couldn't
be used to impersonate a real site to your machine. It could only vouch for
TLS to names that already point at your own computer.

| Property       | Value                                                      |
| -------------- | ---------------------------------------------------------- |
| Name           | `iglu dev CA (localhost only)`                             |
| Key            | ECDSA P-256, generated on your machine; never leaves it    |
| Lifetime       | 10 years                                                   |
| Permits        | the DNS name `localhost` and its subdomains                |
| Excludes       | every IP address, IPv4 and IPv6                            |
| Key usage      | TLS server authentication                                  |
| Trust          | TLS only, where the system can limit it                    |
| Path length    | 1, so Caddy can add its own intermediate                   |
| Files          | `.dev/shared/ca/ca.crt` and `ca.key`, in the main checkout |

The key is created with the dev shell's OpenSSL, and only you can read it.

### Trusting it

Only your own browser needs this, to show the dev console without a warning:

```sh
just dev-trust
```

On macOS this adds the CA to your login Keychain as a root trusted for TLS,
and asks for your password. On Linux it adds it to the NSS database in
`~/.pki/nssdb`, which Chromium reads. Elsewhere, add `.dev/shared/ca/ca.crt`
to your browser's trusted authorities yourself. This is the only step that
changes anything outside the repository, and it isn't something Nix can
declare.

### Removing it

```sh
just dev-untrust
```

This removes the CA from where `just dev-trust` put it. It finds the CA by
name, so do it before you create a new one: with two CAs of the same name,
it can't tell them apart.

### Replacing it

If you lose the key or want a new one, untrust the old CA, then:

```sh
just dev-replace-ca
```

This deletes the CA and Caddy's certificates and creates a new CA. Caddy
issues new certificates from it the next time `just dev` starts. Trust the new
one with `just dev-trust` if you trusted the old one.
