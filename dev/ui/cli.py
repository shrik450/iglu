"""The CLI against the dev stack, as a person signed in through the browser
uses it. dev-check runs these steps on the workspace its browser steps made.

The CLI inside a workspace speaks through the workspace's channel, which
only a real host gives it, so the VM test runs that. `iglu attach` gets a
pseudo-terminal of its own here.
"""

import fcntl
import json
import os
import re
import select
import signal
import struct
import subprocess
import termios
import time
from pathlib import Path
from typing import Any

from playwright.sync_api import Page

import browser
from stack import CONSOLE, ROOT, cli_env, state

# Where dev/cover.py builds the stack's programs when it measures them.
BIN_ENV = "IGLU_DEV_BIN"


class Cli:
    """`iglu`, built with the stack, signed in as nobody else is."""

    def __init__(self) -> None:
        self.program = Path(os.environ.get(BIN_ENV, ROOT / "target/debug")) / "iglu"
        self.env = cli_env(state() / "check-cli")

    def run(self, *args: str, input: str | None = None, ok: bool = True) -> str:
        done = subprocess.run(
            [str(self.program), *args], env=self.env, input=input, capture_output=True, text=True, timeout=120
        )
        if (done.returncode == 0) != ok:
            raise AssertionError(f"iglu {' '.join(args)} exited {done.returncode}: {done.stdout}{done.stderr}")
        return done.stdout + done.stderr

    def json(self, *args: str) -> Any:
        return json.loads(self.run("--json", *args))


def signs_in(page: Page, cli: Cli) -> None:
    cli.run("logout")
    cli.run("whoami", ok=False)
    login = subprocess.Popen(
        [str(cli.program), "login", CONSOLE, "--no-browser"],
        env=cli.env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
    )
    assert login.stdout is not None
    line = login.stdout.readline()
    url = re.search(r"Visit (\S+)", line)
    assert url, line
    browser.approve_cli(page, url.group(1))
    assert login.wait(timeout=60) == 0, login.stdout.read()
    identity = cli.json("whoami")
    assert identity["kind"] == "Person" and identity["email"] == "alice@example.org", identity


def reads(cli: Cli, name: str, project: str, port: str) -> None:
    """What the console shows, the CLI lists."""
    assert any(e["name"] == "default" for e in cli.json("env", "ls"))
    assert any(p["name"] == project for p in cli.json("project", "ls"))
    assert any(w["name"] == name and w["phase"] == "running" for w in cli.json("ls"))
    assert cli.json("show", name)["name"] == name
    assert any(str(r["port"]) == port for r in cli.json("ports", name))
    assert cli.json("log", name), "an empty history"
    assert name in cli.run("ls")


def columns(cli: Cli, name: str) -> None:
    """A column opened, typed into, read, restarted and closed."""
    cli.run("column", "new", "-w", name, "--name", "cli")
    assert any(c["name"] == "cli" for c in cli.json("column", "ls", "-w", name))
    cli.run("column", "send", "cli", "-w", name, "printf 'cli-%s\\n' 42", "--enter")
    until(lambda: "cli-42" in cli.run("column", "output", "cli", "-w", name), "the column's output")
    cli.run("column", "send", "cli", "-w", name, "--paste", "--enter", input="printf 'pasted-%s\\n' 7")
    until(lambda: "pasted-7" in cli.run("column", "output", "cli", "-w", name, "--lines", "20"), "the paste")
    cli.run("column", "restart", "cli", "-w", name)
    cli.run("column", "close", "cli", "-w", name)
    assert not any(c["name"] == "cli" for c in cli.json("column", "ls", "-w", name))


def attaches(cli: Cli, name: str) -> None:
    """A column in a terminal of the CLI's own: typed into, resized and
    detached from with Ctrl-], leaving its session running."""
    cli.run("column", "new", "-w", name, "--name", "attached")
    refused = cli.run("attach", name, "nope", ok=False)
    assert "no column called nope" in refused, refused
    terminal, inside = os.openpty()
    fcntl.ioctl(inside, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    attach = subprocess.Popen(
        [str(cli.program), "attach", name, "attached"],
        env=cli.env, stdin=inside, stdout=inside, stderr=inside, start_new_session=True,
    )
    os.close(inside)
    shown = Screen(terminal)
    try:
        os.write(terminal, b"stty size\r")
        shown.until("30 100")
        fcntl.ioctl(terminal, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        attach.send_signal(signal.SIGWINCH)
        # The column learns the new size as the next line runs, or soon after.
        deadline = time.monotonic() + 30
        while "40 120" not in shown.text:
            if time.monotonic() > deadline:
                raise TimeoutError(f"the resize never reached the column: {shown.text[-500:]!r}")
            os.write(terminal, b"stty size\r")
            shown.read(1)
        os.write(terminal, b"printf 'attached-%s\\n' 9\r")
        shown.until("attached-9")
        os.write(terminal, b"\x1d")
        shown.until(f"detached from {name} attached")
        assert attach.wait(timeout=30) == 0
    finally:
        attach.kill()
        os.close(terminal)
    # Detaching left the session as it was.
    assert "attached-9" in cli.run("column", "output", "attached", "-w", name)
    cli.run("column", "close", "attached", "-w", name)


class Screen:
    """What a terminal has shown so far."""

    def __init__(self, terminal: int) -> None:
        self.terminal = terminal
        self.text = ""

    def read(self, wait: float) -> bool:
        ready, _, _ = select.select([self.terminal], [], [], wait)
        if ready:
            try:
                self.text += os.read(self.terminal, 65536).decode(errors="replace")
            except OSError:
                return False
        return bool(ready)

    def until(self, text: str, timeout: float = 30) -> None:
        deadline = time.monotonic() + timeout
        while text not in self.text:
            if time.monotonic() > deadline:
                raise TimeoutError(f"{text!r} never showed; the terminal shows {self.text[-500:]!r}")
            self.read(0.5)


def changes(cli: Cli, name: str) -> None:
    """Labels, access and secrets, changed and put back."""
    cli.run("label", name, "Checked from the CLI")
    assert cli.json("show", name)["label"] == "Checked from the CLI"
    cli.run("label", name)
    assert cli.json("show", name).get("label") is None
    cli.run("access", "set", name, "--allow", "view,read_output")
    shown = cli.run("access", "show", name)
    assert "read_output" in shown, shown
    cli.run("access", "set", name, "--allow")
    cli.run("secret", "set", "cli-check", "--env", "CLI_CHECK", input="from-the-cli")
    assert any(s["name"] == "cli-check" for s in cli.json("secret", "ls"))
    cli.run("secret", "rm", "cli-check")
    assert not any(s["name"] == "cli-check" for s in cli.json("secret", "ls"))
    # A name that isn't a secret's is refused before anything is sent.
    cli.run("secret", "set", "Not A Name", "--env", "X", input="x", ok=False)


def lifecycle(cli: Cli, name: str) -> None:
    """Stopped, started, then deleted: refused while it has work nowhere
    else, such as a branch never pushed, unless forced."""
    cli.run("stop", name, "--wait")
    assert cli.json("show", name)["phase"] == "stopped"
    cli.run("start", name, "--wait")
    assert cli.json("show", name)["phase"] == "running"
    refused = cli.run("rm", name, ok=False)
    assert "pass --force" in refused, refused
    cli.run("rm", name, "--force", "--wait")
    assert not any(w["name"] == name for w in cli.json("ls"))


def until(check: Any, what: str, timeout: float = 30) -> None:
    deadline = time.monotonic() + timeout
    while not check():
        if time.monotonic() > deadline:
            raise TimeoutError(f"waited for {what}")
        time.sleep(0.5)
