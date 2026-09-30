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
│   console · API · reconciler · previews    │ ◄── sign-in, service tokens
│   SQLite: users, workspaces, secrets        │
└──────────────┬─────────────────────────────┘
               │  HTTPS + service token (client credentials)
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
  what exists. Users never sign in to the host, and hostd accepts only iglud's
  service tokens from the identity provider.
- **Workspaces** are Incus system containers running NixOS, one per task, each
  with a private Nix store. hostd runs everything in them as the workspace
  user, never as root. Their network reaches the Internet but not the host, the
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
tmux inside them. Claude Code hooks, and anything else that calls
`iglu-status`, report whether a session is working, waiting for you, or done.
The console sorts workspaces by what needs you.

### Previews

`iglu port <workspace> <port>` publishes a guest port at
`https://<adjective>-<verb>.<preview domain>`. Previews need a sign-in through
the same identity provider, and only the workspace's owner can open them.
Requests from other sites are refused unless they're top-level navigations.

## Setting it up

You need:

- an OIDC identity provider that supports authorization code with PKCE for
  people, and client credentials with JWT access tokens for iglud's service
  identity (Authelia, Keycloak, Zitadel, and others do);
- a control box with DNS for the console host and a wildcard for the preview
  domain, plus a certificate covering both (a DNS-01 ACME challenge works);
- an execution host running NixOS, with a disk or Btrfs filesystem for
  workspaces and swap for freezing.

Register three clients with the identity provider:

| client  | grant                    | redirect URI                              |
| ------- | ------------------------ | ----------------------------------------- |
| console | authorization code       | `https://<console host>/auth/callback`    |
| preview | authorization code       | `https://auth.<preview domain>/callback`  |
| worker  | client credentials (JWT) | none; an audience such as `iglu-hosts`   |

The console and preview clients need the `email` and `email_verified` claims in
the ID token if you admit people by email.

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
      worker = {
        clientId = "iglu-worker";
        clientSecretFile = "/run/secrets/iglu-worker";
        audience = "iglu-hosts";
      };
    };
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
    tls = { certificate = "/var/lib/iglu/host.crt"; key = "/var/lib/iglu/host.key"; };
    auth = {
      issuer = "https://id.example.org";
      audience = "iglu-hosts";
      subjects = [ "iglu-worker" ]; # the worker token's `sub`
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

Set `iglu.user`, add the tools your agents need, push the flake somewhere the
host can fetch it, and register it:

```sh
iglu login https://iglu.example.org
iglu env add default github:you/iglu-env#default
```

`iglu env build default` rebuilds it after you change the flake. New
workspaces use the newest image; existing ones keep theirs.

## Using it

```sh
iglu new https://github.com/acme/app.git --wait      # a workspace on a new branch
iglu ls
iglu port <workspace> 3000                           # publish a preview
iglu freeze <workspace>                              # or stop, start, rm
iglu secret set anthropic --env CLAUDE_CODE_OAUTH_TOKEN < token.txt
iglu secret set github --git github.com --username x-access-token < pat.txt
iglu secret set deploy-key --file .ssh/id_ed25519 < id_ed25519
```

Secrets reach every workspace you own: environment variables in new
terminals, files under the home directory, and answers from `iglu-guest`,
which is the Git credential helper for HTTPS remotes. For Claude Code, run
`claude setup-token` and store the result as `CLAUDE_CODE_OAUTH_TOKEN`. That
token is meant for sharing across machines; copied OAuth login files aren't.

Every command takes `--json`.

## Developing

See [AGENTS.md](AGENTS.md) for the layout, commands, and design rules. In
short: `nix develop`, then `just lint test` while you work and `just check`
for every check. CI runs all of them except the end-to-end VM test in
`nix/tests/e2e.nix`, which uses a real identity provider, Incus, and Git
server; run it with `just e2e` on x86_64-linux with KVM.
