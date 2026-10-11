# Roadmap

## Now: 0.x

iglu runs for real, on a trial execution host, while its rough edges get
worked out. Expect breaking changes between releases. The protocol, the
database schema and the NixOS modules can change without a migration path,
and a release can need the control box and every host upgraded together.

Known gaps:

- The console can't add or rebuild environments or manage secrets; the CLI
  does.
- Environments are only flake references. A small one could live in iglu,
  with its lock, and be edited from the console.

## Ideas

Not planned yet, but worth keeping:

- Choose what's kept: which programs' configuration your workspaces share
  through the kept directory, rather than iglu's fixed list (Claude Code,
  Codex, gh).
- GitHub access scoped to a workspace's own repository. GitHub can't mint
  personal access tokens through its API, but it can open the new
  fine-grained token page filled in from a link: the name, the one
  repository, permissions and expiry. iglu could offer that link for each
  project, take the token you generate, and give it, as `GH_TOKEN` and the
  Git credential, only to that project's workspaces. That needs secrets
  scoped to a project; today every secret reaches all your workspaces.

## 1.0

1.0 is when daily use stops turning up rough edges. Then the trial host is
retired, a permanent one replaces it, and changes from there on keep
existing installs working.
