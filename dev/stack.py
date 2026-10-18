"""The local dev stack: iglud, a local execution host, and the identity
provider and TLS front they need, run from the dev shell.

    python3 dev/stack.py run           # just dev
    python3 dev/stack.py wait          # until the console answers
    python3 dev/stack.py reset         # just dev-reset
    python3 dev/stack.py cli ARGS...   # just dev-cli
    python3 dev/stack.py trust         # just dev-trust; also untrust, replace-ca

Every process is a child of `run`, and every file is under .dev/, so stopping
`run` stops everything and deleting .dev/ forgets everything. Nothing needs
privileges: the stack serves HTTPS on PORT, and only on loopback.
DEVELOPMENT.md explains it.
"""

import hashlib
import json
import os
import platform
import shutil
import ssl
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# The stack's HTTPS front, Caddy, serves every hostname on this port. Browsers
# and macOS resolve `*.localhost` to loopback, and so does Linux's
# nss-myhostname, so there's no DNS to set up.
PORT = 7300
# Previews share the console's site, as they share a registrable domain when
# deployed: browsers count `localhost` as the suffix, so the site is
# iglu.localhost, and they call a preview's requests to the console
# same-site, the case iglu's preview rules exist for. The IdP and the fixture
# repositories are sites of their own.
CONSOLE_HOST = "iglu.localhost"
PREVIEW_DOMAIN = f"preview.{CONSOLE_HOST}"
AUTH_HOST = "auth.localhost"
GIT_HOST = "git.localhost"
CONSOLE = f"https://{CONSOLE_HOST}:{PORT}"
AUTH = f"https://{AUTH_HOST}:{PORT}"
GIT = f"https://{GIT_HOST}:{PORT}"
IGLUD = "127.0.0.1:7100"
DEVHOST = "127.0.0.1:7200"
AUTHELIA = "127.0.0.1:7301"

CA_NAME = "iglu dev CA (localhost only)"
USERS = ["alice", "bob", "carol"]


def shared() -> Path:
    """What every worktree of this repository shares: the dev CA, Caddy's
    certificates, Authelia's key and data, and fixture repositories. It's in
    the main checkout, or here outside a Git checkout."""
    found = subprocess.run(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd=ROOT, capture_output=True, text=True,
    )
    base = Path(found.stdout.strip()).parent if found.returncode == 0 else ROOT
    return base / ".dev/shared"


def state() -> Path:
    """This worktree's own state: iglud's database and settings and the local
    host's workspaces. Each branch can have its own schema."""
    return ROOT / ".dev/state"


def runtime() -> Path:
    """Local workspaces' runtime directories, which hold terminal sockets.
    Their paths must fit a Unix socket's, which the worktree's path is too
    long for, and a temporary directory's cleaner would empty them under idle
    workspaces."""
    worktree = hashlib.sha256(str(ROOT).encode()).hexdigest()[:8]
    return Path.home() / ".cache/iglu" / worktree


def openssl(*args: str) -> None:
    subprocess.run(["openssl", *args], check=True, capture_output=True)


def private(path: Path, contents: str) -> None:
    path.touch(mode=0o600, exist_ok=True)
    path.write_text(contents)


def setup() -> None:
    """Creates what's missing: the dev CA, Authelia's key, iglud's secrets,
    and both daemons' settings."""
    ca = shared() / "ca"
    for directory in [ca, shared() / "authelia/data", shared() / "git", state() / "logs"]:
        directory.mkdir(parents=True, exist_ok=True)
    if not (ca / "ca.crt").exists():
        # Name-constrained to localhost and excluding every IP address, so
        # trusting it can't vouch for any real site; see DEVELOPMENT.md.
        openssl(
            "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
            "-days", "3650", "-subj", f"/CN={CA_NAME}",
            "-keyout", str(ca / "ca.key"), "-out", str(ca / "ca.crt"),
            "-addext", "basicConstraints=critical,CA:TRUE,pathlen:1",
            "-addext", "keyUsage=critical,keyCertSign,cRLSign",
            "-addext", "extendedKeyUsage=serverAuth",
            "-addext", "nameConstraints=critical,permitted;DNS:localhost,"
            "excluded;IP:0.0.0.0/0.0.0.0,excluded;IP:0:0:0:0:0:0:0:0/0:0:0:0:0:0:0:0",
        )
        (ca / "ca.key").chmod(0o600)
        print("Created the dev CA. To trust it in your own browser: just dev-trust")
    jwks = shared() / "authelia/jwks.pem"
    if not jwks.exists():
        openssl("genrsa", "-out", str(jwks), "2048")
        jwks.chmod(0o600)

    key = state() / "secret-key"
    if not key.exists():
        private(key, os.urandom(32).hex())
    # The local host is on loopback, without TLS, so nothing checks this one.
    if not (state() / "host-client.crt").exists():
        openssl(
            "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
            "-days", "3650", "-subj", "/CN=iglud-dev",
            "-keyout", str(state() / "host-client.key"), "-out", str(state() / "host-client.crt"),
        )

    clients = ROOT / "dev/clients"
    iglud = {
        "listen": IGLUD,
        "console_origin": CONSOLE,
        "preview_domain": PREVIEW_DOMAIN,
        "database": str(state() / "iglu.db"),
        "secret_key_file": str(key),
        "console_assets": str(ROOT / "console/dist"),
        "extra_ca_certificates": str(ca / "ca.crt"),
        "oidc": {
            "issuer": AUTH,
            "console": {"client_id": "console", "client_secret_file": str(clients / "console.secret")},
            "preview": {"client_id": "preview", "client_secret_file": str(clients / "preview.secret")},
        },
        "sign_in": [{"verified_email": f"{user}@example.org"} for user in USERS],
        "hosts": [{"id": "local", "url": f"http://{DEVHOST}"}],
        "host_client": {
            "certificate": str(state() / "host-client.crt"),
            "key": str(state() / "host-client.key"),
        },
    }
    devhost = {
        "host_id": "local",
        "listen": DEVHOST,
        "runtime": {
            "state_dir": str(state() / "devhost"),
            "runtime_dir": str(runtime()),
            "guest_tools": str(ROOT / "target/debug"),
            # The dev shell's bash, not your login shell, so checks type into
            # the same shell everywhere; run yours from it.
            "shell": os.environ["IGLU_DEV_SHELL"],
            # The local runtime doesn't meter memory; it reports this much as free.
            "memory_available": 64 * 1024**3,
            "localhost_ca": {"certificate": str(ca / "ca.crt"), "port": PORT},
        },
    }
    (state() / "iglud.json").write_text(json.dumps(iglud, indent=2))
    (state() / "devhost.json").write_text(json.dumps(devhost, indent=2))


def services_env() -> dict[str, str]:
    """What dev/Caddyfile and dev/authelia/configuration.yml read."""
    return {
        **os.environ,
        "IGLU_DEV": str(ROOT / "dev"),
        "IGLU_DEV_SHARED": str(shared()),
        "IGLU_DEV_PORT": str(PORT),
        "IGLU_DEV_CONSOLE_HOST": CONSOLE_HOST,
        "IGLU_DEV_PREVIEW_DOMAIN": PREVIEW_DOMAIN,
        "IGLU_DEV_AUTH_HOST": AUTH_HOST,
        "IGLU_DEV_GIT_HOST": GIT_HOST,
        "IGLU_DEV_IGLUD": IGLUD,
        "IGLU_DEV_AUTHELIA": AUTHELIA,
        "X_AUTHELIA_CONFIG_FILTERS": "template",
    }


def run() -> None:
    """Builds and runs the whole stack in the foreground, rebuilding the
    console on save, until Ctrl-C or one of its processes ends."""
    setup()
    subprocess.run(
        ["cargo", "build", "-p", "iglud", "-p", "iglu-devhost", "-p", "iglu-guest", "-p", "iglu-cli"],
        cwd=ROOT, check=True,
    )
    console = ROOT / "console"
    # npm records the lock file it installed from; reinstall when the lock
    # file has changed since.
    installed = console / "node_modules/.package-lock.json"
    if not installed.exists() or installed.stat().st_mtime < (console / "package-lock.json").stat().st_mtime:
        subprocess.run(["npm", "ci"], cwd=console, check=True)

    logs = state() / "logs"
    env = services_env()
    target = ROOT / "target/debug"
    # Caddy and Authelia are chatty and rarely interesting, so they log to
    # files; the rest log here.
    commands = [
        ("caddy", ["caddy", "run", "--config", str(ROOT / "dev/Caddyfile"), "--adapter", "caddyfile"], logs / "caddy.log"),
        ("authelia", ["authelia", "--config", str(ROOT / "dev/authelia/configuration.yml")], logs / "authelia.log"),
        ("console", ["node", "build.mjs", "--watch"], None),
        ("devhost", [str(target / "iglu-devhost"), "--config", str(state() / "devhost.json")], None),
        ("iglud", [str(target / "iglud"), "--config", str(state() / "iglud.json")], None),
    ]
    children: list[tuple[str, subprocess.Popen]] = []
    try:
        for name, command, log in commands:
            output = log.open("ab") if log else None
            cwd = console if name == "console" else ROOT
            children.append((name, subprocess.Popen(command, cwd=cwd, env=env, stdout=output, stderr=output)))
        print(f"iglu: {CONSOLE} ({', '.join(USERS)}, password \"password\")")
        print(f"Caddy and Authelia log to {logs}")
        while True:
            for name, child in children:
                if child.poll() is not None:
                    sys.exit(f"{name} exited with {child.returncode}")
            time.sleep(0.5)
    except KeyboardInterrupt:
        pass
    finally:
        for _, child in reversed(children):
            child.terminate()
        for _, child in reversed(children):
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()


def wait(timeout: float = 300) -> None:
    """Waits until the console and the identity provider answer through
    Caddy, for scripts that start `run` in the background."""
    deadline = time.monotonic() + timeout
    for url in [f"{AUTH}/.well-known/openid-configuration", f"{CONSOLE}/"]:
        while True:
            try:
                # `run` makes the CA as it starts, so it may not be there yet.
                trust = ssl.create_default_context(cafile=str(shared() / "ca/ca.crt"))
                urllib.request.urlopen(url, context=trust, timeout=5)
                break
            except urllib.error.HTTPError as error:
                if error.code < 500:
                    break
                problem = str(error)
            except OSError as error:
                problem = str(error)
            if time.monotonic() > deadline:
                sys.exit(f"{url} didn't answer: {problem}")
            time.sleep(1)
    print(f"{CONSOLE} is up")


def reset() -> None:
    """Deletes this worktree's database, settings and workspaces, ending
    their terminal sessions, so the next run starts fresh. What worktrees
    share is kept."""
    for sessions in runtime().glob("*/zmx"):
        env = {**os.environ, "ZMX_DIR": str(sessions)}
        listed = subprocess.run(["zmx", "list"], env=env, capture_output=True, text=True)
        for line in listed.stdout.splitlines():
            session = line.split("\t")[0].removeprefix("session_name=")
            subprocess.run(["zmx", "kill", session], env=env, capture_output=True)
    shutil.rmtree(state(), ignore_errors=True)
    shutil.rmtree(runtime(), ignore_errors=True)


def cli(args: list[str]) -> None:
    """The CLI against the dev stack, signed in separately from your real
    iglu. It trusts the dev CA as your system does; on Linux, it's told to."""
    env = {**os.environ, "XDG_CONFIG_HOME": str(state() / "config")}
    if platform.system() == "Linux":
        env["SSL_CERT_FILE"] = str(shared() / "ca/ca.crt")
    sys.exit(subprocess.run(["cargo", "run", "-q", "-p", "iglu-cli", "--", *args], cwd=ROOT, env=env).returncode)


# Trusting the dev CA in your own browser is optional: the stack and its tools
# don't need it. How depends on the system.
KEYCHAIN = Path.home() / "Library/Keychains/login.keychain-db"
NSSDB = f"sql:{Path.home() / '.pki/nssdb'}"


def trust() -> None:
    """Trusts the dev CA for TLS in your browsers: the login Keychain on
    macOS, which asks for your password, or the NSS database Chromium and
    other browsers read on Linux."""
    ca = str(shared() / "ca/ca.crt")
    match platform.system():
        case "Darwin":
            subprocess.run(["security", "add-trusted-cert", "-r", "trustRoot", "-p", "ssl", "-k", str(KEYCHAIN), ca], check=True)
        case "Linux":
            (Path.home() / ".pki/nssdb").mkdir(parents=True, exist_ok=True)
            if not (Path.home() / ".pki/nssdb/cert9.db").exists():
                subprocess.run(["certutil", "-d", NSSDB, "-N", "--empty-password"], check=True)
            subprocess.run(["certutil", "-d", NSSDB, "-A", "-t", "C,,", "-n", CA_NAME, "-i", ca], check=True)
        case other:
            sys.exit(f"Trust {ca} for TLS in your browser; this script doesn't know how on {other}.")


def untrust() -> None:
    """Removes the dev CA from where `trust` put it. It finds it by name."""
    match platform.system():
        case "Darwin":
            subprocess.run(["security", "delete-certificate", "-t", "-c", CA_NAME, str(KEYCHAIN)], check=True)
        case "Linux":
            subprocess.run(["certutil", "-d", NSSDB, "-D", "-n", CA_NAME], check=True)
        case other:
            sys.exit(f"Remove {CA_NAME} from your browser's trust; this script doesn't know how on {other}.")


def replace_ca() -> None:
    """Replaces the dev CA with a new one, and has Caddy reissue its
    certificates from it on its next start. Untrust the old one first."""
    shutil.rmtree(shared() / "ca", ignore_errors=True)
    shutil.rmtree(shared() / "caddy", ignore_errors=True)
    setup()


def main() -> None:
    commands = {"run": run, "wait": wait, "reset": reset, "trust": trust, "untrust": untrust, "replace-ca": replace_ca}
    match sys.argv[1:]:
        case ["cli", *args]:
            cli(args)
        case [command] if command in commands:
            commands[command]()
        case _:
            sys.exit(__doc__)


if __name__ == "__main__":
    main()
