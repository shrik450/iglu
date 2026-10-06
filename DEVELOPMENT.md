# Developing iglu

This guide covers working on iglu on your own machine: the tools, the checks,
and the local dev stack, which runs iglud and the console against a real
identity provider over HTTPS in seconds. `AGENTS.md` holds the design rules
every change follows; read it first.

## Tools

You need:

- [Nix](https://nixos.org) with flakes. Every build tool comes from the dev
  shell: run `nix develop`, then use `just`.
- A Docker daemon, such as [Colima](https://github.com/abiosoft/colima). The
  dev stack's shared services run in containers. The dev shell provides the
  `docker` client and Compose.
- macOS, for the dev stack as it stands. Trusting the dev CA uses the Keychain;
  see [Local HTTPS and the dev CA](#local-https-and-the-dev-ca).

## Checks

| Command        | What it does                                                     |
| -------------- | ---------------------------------------------------------------- |
| `just fmt`     | Formats Rust and Nix.                                            |
| `just lint`    | rustfmt, clippy (pedantic, warnings are errors), nixfmt, actionlint. |
| `just test`    | The unit tests. Fast, and they run anywhere.                     |
| `just console` | Type-checks and bundles the console.                             |
| `just e2e`     | The VM test. Needs x86_64-linux with KVM, locally or as a remote builder. |
| `just check`   | Every flake check, including the VM test where it can run.       |

CI runs every flake check except the VM test. Run `just lint test` before you
push, and `just e2e` before merging changes to hostd, the guest tools, the
console, or the NixOS modules.

## The local dev stack

The dev stack is for working on the console and iglud's API without a real
execution host. It runs:

- **iglud**, natively, built from your checkout. It serves the console from
  `console/dist`, which rebuilds whenever you save.
- **Authelia**, in Docker, as the identity provider. Its configuration follows
  the VM test's.
- **Caddy**, in Docker, in front of everything. It terminates TLS the way it
  does on a real control box, and routes each hostname to the right service.

```text
browser ──► Caddy (443, TLS from the dev CA)
              ├─ auth.localhost                      ──► Authelia
              └─ iglu.localhost, *.preview.localhost ──► iglud (port 7100)
```

Every hostname is under `localhost`, which browsers and macOS resolve to your
own machine, so there's no DNS to set up.

### What it doesn't have yet

There's no execution host. iglud points at `http://127.0.0.1:7200`, where a
local host will listen, and until then it logs `host unreachable` every few
seconds. Signing in, the console and the API calls that only touch
iglud's database work, such as managing secrets. Anything that needs a host,
such as starting a workspace or building an environment, waits for it.

### Setting it up

Do this once per machine:

```sh
just dev-services   # creates the dev CA and Authelia's key, then starts Caddy and Authelia
just dev-trust      # trusts the dev CA in your login Keychain; macOS asks for your password
```

Read [Local HTTPS and the dev CA](#local-https-and-the-dev-ca) before you run
`just dev-trust`: it explains what you're trusting and why it's safe.

### Running it

```sh
just dev
```

Then open `https://iglu.localhost` and sign in as `alice` or `bob`, both with
the password `password`. Two people let you try the console as different
owners.

`just dev` builds iglud, starts the console's watcher and runs iglud in the
foreground. Stop it with Ctrl-C. Edit anything under `console/src` or
`console/public` and refresh the page; the bundle is unminified and has a source
map. A change to iglud needs a restart.

### Where things are

| What              | Value                                  |
| ----------------- | -------------------------------------- |
| Console           | `https://iglu.localhost`               |
| Previews          | `https://<route>.preview.localhost`    |
| iglud             | `127.0.0.1:7100`                       |
| Local host, later | `127.0.0.1:7200`                       |
| State             | `.dev/state/` in the current worktree  |

iglud's state is its database, its secret-sealing key and the dev CLI's login.
It lives in the worktree you run `just dev` from, because each branch can have
its own schema. Delete it with `just dev-reset`. Only one worktree can run the
stack at a time, since they all use the same port and hostnames.

Every worktree shares one Caddy, one Authelia and one dev CA. The CA and
Authelia's key and data live in `.dev/shared/` of the main checkout, so every
worktree uses the same ones. Authelia's data matters: it assigns each person
the subject identifier iglud knows them by, so deleting it makes alice and bob
new people to every worktree's iglud. If that happens, run `just dev-reset` in
each.

### The CLI

`just dev-cli` runs the CLI against the dev stack, with a login kept in its
state instead of your real `~/.config/iglu`:

```sh
just dev-cli login https://iglu.localhost
just dev-cli ls
```

### Shared services

| Command                  | What it does                                              |
| ------------------------ | --------------------------------------------------------- |
| `just dev-services`      | Starts Caddy and Authelia, creating the CA and key if needed. |
| `just dev-services-down` | Stops them. Caddy's certificates are discarded and reissued on the next start. |
| `just dev-logs`          | Follows their logs. Add `caddy` or `authelia` for one.    |

The services' configuration is in `dev/`. The client secrets in
`dev/clients/` and the passwords in `dev/authelia/` are for local development
only.

### Troubleshooting

- **`UnknownIssuer` in iglud's log, or a certificate warning in the browser.**
  The dev CA isn't trusted. Run `just dev-trust`.
- **`the identity provider isn't reachable yet`.** The shared services aren't
  running. Run `just dev-services`.
- **Port 443 is in use.** Something else on your machine is serving HTTPS.
  Stop it, or stop its containers.
- **`chown: ... Read-only file system` in Authelia's log.** Harmless. Authelia's
  entrypoint tries to take ownership of its configuration, which is mounted
  read-only on purpose.
- **iglud behaves strangely after switching branches.** Its database may be
  from another schema. Run `just dev-reset`.

## Local HTTPS and the dev CA

The dev stack runs over HTTPS because iglu needs it, not for show:

- iglud expects TLS to end in front of it, and its preview sign-in only accepts
  `https` addresses on the default port.
- Its cookies are `Secure`, and the preview cookie spans the preview domain.
- Authelia refuses to run its sign-in over plain HTTP.

Faking HTTPS away would mean a development-only path through iglu's security
code, so the stack uses real certificates instead. Something has to issue them,
and iglud and your browser both have to trust the issuer. On macOS, both check
the Keychain.

### What the CA can sign

`just dev-setup` creates a certificate authority, the dev CA. Caddy issues
every certificate in the stack from it. Its certificate limits what it can
vouch for in three ways:

- **Names.** An X.509 name constraint permits only the DNS name `localhost`
  and names under it, such as `iglu.localhost`.
- **IP addresses.** The same constraint excludes every IPv4 and IPv6 address,
  so it can't vouch for a site reached by address, such as `https://192.168.1.1`.
  A constraint on DNS names alone wouldn't cover addresses.
- **Purpose.** Its extended key usage is TLS server authentication only.

`just dev-trust` adds a matching limit on your side: the Keychain trusts the CA
for TLS only, not for signed email, code signing or anything else.

These limits are what make trusting the CA safe, and macOS enforces them: with
the CA trusted, a certificate it signs for `example.com` or for
`192.168.1.1` fails verification. So if the CA's key ever leaked, it couldn't
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
| Keychain trust | TLS only (`-p ssl`)                                        |
| Path length    | 1, so Caddy can add its own intermediate                   |
| Files          | `.dev/shared/ca/ca.crt` and `ca.key`, in the main checkout |

The key is created with the dev shell's OpenSSL, and only you can read it.

### Trusting it

```sh
just dev-trust
```

This adds the CA to your login Keychain as a root trusted for TLS. macOS asks
for your password. It's the only step of the setup that changes your system,
and it isn't something Nix can declare.

To check that the Keychain trusts it for TLS:

```sh
just dev-check-trust   # prints "certificate verification successful"
```

### Removing it

```sh
just dev-untrust
```

This removes the CA and its trust setting from your Keychain. It finds the CA
by name, so do it before you create a new one: with two CAs of the same name
in the Keychain, it can't tell them apart.

### Replacing it

If you lose the key or want a new one:

```sh
just dev-replace-ca
```

This removes the old CA from the Keychain, if it's there, and deletes it.
Then it creates a new one, trusts it, and restarts the shared services so
Caddy reissues its certificates from it. macOS asks for your password.
