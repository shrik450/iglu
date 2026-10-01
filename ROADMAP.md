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

## 1.0

1.0 is when daily use stops turning up rough edges. Then the trial host is
retired, a permanent one replaces it, and changes from there on keep
existing installs working.
