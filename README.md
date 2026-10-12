# iglu

iglu gives every coding task its own NixOS system container: a **workspace**
with the repository checked out on its own branch, persistent terminals in the
browser, and previews of the dev servers it runs, each at its own subdomain.
It's built for running coding agents such as Claude Code side by side, and for
finding the ones that need you.

It's self-hosted and small: one control box, one or more execution hosts, and
your own OpenID Connect identity provider.

## How it fits together

```text
 browser / iglu CLI
        │  HTTPS: console, API, previews
        ▼
┌─────────────── control box ────────────────┐
│ Caddy (TLS) ─► iglud                       │     identity provider (OIDC)
│   console · API · reconciler · previews    │ ◄── sign-in
│   SQLite: users, workspaces, secrets        │
└──────────────┬─────────────────────────────┘
               │  HTTPS, iglud's client certificate
               ▼
┌────────── execution host ("dumb host") ────┐
│ hostd ─► Incus (Btrfs) ─► workspace containers
│        ─► Nix daemon: builds environment images
└────────────────────────────────────────────┘
```

- **iglud** is the control plane. It holds desired state in SQLite: who exists,
  what workspaces they want and in which state, their published routes, and
  their secrets, which it seals with XChaCha20-Poly1305. A reconciler drives
  each workspace toward its desired state through hostd. iglud also serves the
  console, the JSON API, terminal relays, and the preview gateway.
- **hostd** is the execution host's only privileged service. It exposes a fixed
  set of typed commands and keeps no state of its own; Incus is the record of
  what exists. Users never sign in to the host, and hostd accepts only iglud,
  by its client certificate.
- **Workspaces** are Incus system containers running NixOS, one per task, each
  with a private Nix store. hostd runs everything in them as the workspace
  user, never as root, but for upgrading their system: the system's own Nix
  tools and the new system's activation, on store paths hostd chose. Their network reaches the Internet but not the host, the
  LAN, or other private address space.
- **Environments** are ordinary NixOS configurations in a flake that import
  `iglu.nixosModules.workspace`. hostd builds them into images with the host's
  Nix daemon.

### Lifecycle

A workspace is **running**, **frozen**, or **stopped**. Freezing pauses the
container and pushes its memory to swap; a thaw has dev servers answering
again in under a second. Stopping ends every process but keeps the files.
Durability comes from Git: push your work. A host restart stops frozen
workspaces.

### Terminals and attention

Terminals are [zmx](https://github.com/neurosnap/zmx) sessions: they survive
disconnects, several browsers can attach to the same one, and you can still use
tmux inside them. Columns are shells, where you run agents as anything else,
and the workspace's browser. Claude Code hooks, and anything else that calls
`iglu-status`, report whether a session is working, waiting for you, or done.
The console sorts workspaces by what needs you.

### Previews

`iglu port <workspace> <port>` publishes a guest port at
`https://<adjective>-<verb>.<preview domain>`. Previews need a sign-in through
the same identity provider, and only the workspace's owner can open them.
Requests from other sites are refused unless they're top-level navigations.

## Setting it up

You need:

- an OIDC identity provider that supports authorization code with PKCE
  (Authelia, Keycloak, Zitadel, Pocket ID and others do);
- a control box with DNS for the console host and a wildcard for the preview
  domain, plus a certificate covering both (a DNS-01 ACME challenge works);
- an execution host running NixOS, with a disk or Btrfs filesystem for
  workspaces and swap for freezing.

Register two clients with the identity provider:

| client  | grant              | redirect URI                             |
| ------- | ------------------ | ---------------------------------------- |
| console | authorization code | `https://<console host>/auth/callback`   |
| preview | authorization code | `https://auth.<preview domain>/callback` |

They need the `email` and `email_verified` claims in the ID token if you
admit people by email.

Hosts know iglud by a client certificate from a CA kept for that alone, apart
from the one behind hosts' own certificates, or any certificate that CA signs
could drive a host:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 3650 \
  -subj /CN=iglu-clients -keyout clients.key -out clients.crt \
  -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj /CN=iglud \
  -keyout iglud.key -out iglud.csr
openssl x509 -req -in iglud.csr -CA clients.crt -CAkey clients.key -days 3650 \
  -extfile <(printf 'extendedKeyUsage=clientAuth\nbasicConstraints=CA:FALSE\n') -out iglud.crt
```

Give the control box `iglud.crt` and `iglud.key`, give hosts `clients.crt`, and
keep `clients.key` offline.

### Control box

```nix
{
  imports = [ iglu.nixosModules.control ];
  services.iglu.control = {
    enable = true;
    consoleHost = "iglu.example.org";
    previewDomain = "dev.example.org";
    acmeHost = "example.org"; # a security.acme.certs entry covering both
    oidc = {
      issuer = "https://id.example.org";
      console = { clientId = "iglu-console"; clientSecretFile = "/run/secrets/iglu-console"; };
      preview = { clientId = "iglu-preview"; clientSecretFile = "/run/secrets/iglu-preview"; };
    };
    hostClient = { certificate = "/var/lib/iglu/iglud.crt"; key = "/run/secrets/iglud.key"; };
    signIn.emails = [ "you@example.org" ];
    hosts.host-1 = "https://host-1.lan:7443";
  };
}
```

### Execution host

```nix
{
  imports = [ iglu.nixosModules.host ];
  services.iglu.host = {
    enable = true;
    hostId = "host-1";
    tls = {
      certificate = "/var/lib/iglu/host.crt";
      key = "/var/lib/iglu/host.key";
      clients = "/var/lib/iglu/clients.crt"; # the CA of iglud's certificate
    };
    storage.source = "/dev/disk/by-id/nvme-example-part3";
  };
  swapDevices = [ { device = "/swapfile"; size = 32768; } ];
}
```

iglud verifies hostd's certificate against the control box's system trust
store. If you use a private CA, add it with `security.pki.certificateFiles`.

### Environments

Start from the template:

```sh
nix flake init -t <iglu flake>#workspace
```

Set `iglu.user`, add the tools your agents need, push the flake and its
`flake.lock` somewhere the host can fetch it, and register it:

```sh
iglu login https://iglu.example.org
iglu env add default github:you/iglu-env#default
```

`iglu env build default` rebuilds it after you change the flake. New
workspaces use the newest image; existing ones keep theirs until you update
them, with `iglu update <workspace>` or from the console, which offers it
once a newer image is ready. Updating upgrades the workspace's NixOS in
place, as `nixos-rebuild boot` would: the host copies the store paths of the
new image's system that the workspace lacks into its store, makes it the
system it boots, and restarts it. What was running ends; everything on its
disk stays, and the old system is removed from its store. An image whose
account (its name, uid, gid or home) differs can't be updated to: the
workspace's files would be someone else's. Hosts keep each image's system in
their Nix store for as long as they keep the image, and remove
an image once no environment's newest build and no workspace uses it, and
it's more than an hour old. A build also
unpacks the image onto the host's storage, so the first workspace made from
it starts as quickly as the rest.

The guest tools, which hostd drives and agents use (`iglu`, `iglu-status`,
`caffeinate`, zmx), aren't part of an environment: each host mounts its
own static build of them, read-only, into every workspace at
`/var/lib/iglu/tools`. Upgrading a host upgrades them in every workspace on
it, running or not, and an environment's flake only needs `nix flake update
iglu` for changes to the workspace module itself.

The flake can be private. Builds use your Git credential secrets (see
[Using it](#using-it)) as Nix access tokens, so the token that lets a
workspace clone from GitHub also lets the host fetch
`github:you/private-flake`. Nix sends these tokens for `github:` and `gitlab:`
references, not `git+https:` ones. The host keeps what it fetched in its Nix
store, and every owner's builds share that store.

### Idle workspaces

A workspace nobody has used for two hours freezes: its memory moves to swap
and its processes pause. An attached terminal, a preview request, a working
or waiting agent, and having it open in the console all count as use; a
project can set its own timing, or never idle. Attaching
to a frozen workspace or opening its preview thaws it. A long build in a
plain terminal isn't an agent, so run it as `caffeinate cargo build` (or run
`caffeinate` alone until Ctrl-C) to keep the workspace awake. Change the timing in
`services.iglu.control.extraSettings.idle`: `freeze_after_minutes` and
`stop_after_minutes`, either `null` for never. Stopping is off by default.

### Backups

iglud copies its database to `/var/lib/iglud/backups` at every start, before
any migration, and every six hours, keeping the newest 28
(`services.iglu.control.backups`). Ship that directory off the box. The copies
hold stored secrets sealed with `/var/lib/iglud/secret-key`, which they don't
include: back the key up once, separately. People's kept directories (see
[Using it](#using-it)) are on the execution host, not in these copies.

To restore, stop `iglud`, replace `/var/lib/iglud/iglu.db` with a copy and
delete any `iglu.db-wal` and `iglu.db-shm` beside it, then start `iglud`.
Workspaces created since the copy are left running and logged, because iglud
never deletes an instance it has no record of. Workspaces deleted since the
copy come back, freshly cloned.

## Using it

```sh
iglu project add --repo https://github.com/acme/app.git   # a project named app
iglu new app --wait                                  # a workspace on a new branch
iglu new general --label "Tidy my notes"             # no repository, named tidy-my-notes
iglu attach <workspace> [column]                     # a column in this terminal; Ctrl-] detaches
iglu ls
iglu port <workspace> 3000                           # publish a preview
iglu freeze <workspace>                              # or stop, start, rm
iglu secret set anthropic --env CLAUDE_CODE_OAUTH_TOKEN < token.txt
iglu secret set github --git github.com --username x-access-token < pat.txt
iglu secret set deploy-key --file .ssh/id_ed25519 < id_ed25519
```

The console does all of this too: environments, projects and secrets live
in Settings, under your initial in the top bar.

Every workspace belongs to a project. A project names a repository, or
none, and an environment; `general` is the one everyone starts with, without
a repository.

Secrets reach every workspace you own: environment variables in new
terminals, files under the home directory, and answers from `iglu-guest`,
which is the Git credential helper for HTTPS remotes. For Claude Code, run
`claude setup-token` and store the result as `CLAUDE_CODE_OAUTH_TOKEN`. That
token is meant for sharing across machines; copied OAuth login files aren't.

Claude Code, Codex and GitHub's `gh` keep their configuration in your kept
directory, one per person on each host, which all your workspaces share,
whatever their environment. Sign in, finish onboarding, trust a checkout and
change a setting once, and every workspace has it. Sessions find the
directory in `IGLU_KEPT`, and `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and
`GH_CONFIG_DIR` point into it. Being shared, a hook or MCP server an agent
adds there runs in all your workspaces, and two agents saving settings at
the same moment can lose one change. Codex replaces its sign-in token as it
uses it, so a workspace's Codex may find itself signed out after another's
refreshed it; sign in again there. On a host it's the Incus volume
`iglu-kept-<your id>-<uid>` in the `iglu` pool, which hostd never deletes.

So these programs don't read their directories in your home: what an
environment puts in `~/.claude`, `~/.codex` or `~/.config/gh`, with Home
Manager or otherwise, goes unread. An environment declares its agents'
configuration in their system layers instead, which every workspace on it
reads alongside what you keep: Claude Code's `/etc/claude-code` (a
`CLAUDE.md`, skills in `.claude/skills`, settings in `managed-settings.d`),
and Codex's `/etc/codex` (skills, rules, and `requirements.toml`, whose
`additional_developer_instructions` stand in for an `AGENTS.md`). Codex
skips rules files that are symlinks, so write those with `mode = "0444"`.
gh has no system layer: its settings are yours, kept with its sign-in.

Every command takes `--json`.

### From inside a workspace

`iglu` also runs inside a workspace, where it acts as that workspace through
the workspace's own channel to iglu, `$IGLU_SOCKET`, with no token to keep.
Agents use it to open, read and type into the workspace's columns:

```sh
iglu whoami
iglu column new --name tests
iglu column send tests "make test" --enter
iglu column output tests --lines 50
```

A workspace reaches another of yours only as far as you allow, in its
Details in the console or with
`iglu access set <workspace> --on <other> --allow read_output,send_input`.
It can't touch secrets, environments or projects.

### The browser

A browser column (Add a column, then Browser) shows a Chromium that runs in
the workspace, so `localhost:3000` is the workspace's own server. Agents
drive the same browser over Chrome's debugging protocol at `$IGLU_BROWSER`,
for example with `npx @playwright/mcp --cdp-endpoint $IGLU_BROWSER`, and you
watch what they do. A workspace has one browser. It keeps sign-ins in
`~/.cache/iglu/browser` and starts each time with one blank tab. Copying and
pasting go through your own clipboard. The console shows a page's `<select>`
menus and asks you for the files a file input wants, which go to the
workspace as pasted files do, under names safe for a shell. Downloads go to
`~/Downloads` in the workspace, and you save them from there with Save to
this computer; while the column is open, an agent's own choice of where
downloads go is overridden. Date and color pickers, and pages that ask for
files with `showOpenFilePicker`, don't work. Turn it off with
`iglu.browser.enable = false` in your environment.

## Developing

See [AGENTS.md](AGENTS.md) for the layout, commands, and design rules. In
short: `nix develop`, then `just lint test` while you work and `just check`
for every check. CI runs all of them except the end-to-end VM test in
`nix/tests/e2e.nix`, which uses a real identity provider, Incus, Git server,
and browser; run it with `just e2e` on x86_64-linux with KVM.

[DEVELOPMENT.md](DEVELOPMENT.md) covers working on iglu locally, including
the dev stack that runs iglud and the console on your machine with `just dev`.
