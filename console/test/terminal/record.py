"""Records real programs' output for the terminal tier's differential case.

    nix shell --inputs-from . nixpkgs#neovim nixpkgs#vim nixpkgs#less nixpkgs#git \\
        -c python3 console/test/terminal/record.py

Each program runs in a pseudo-terminal of 80 by 24, in an empty home with
fixed content, so nothing of the machine it ran on is in what it printed, and
is driven by keys typed at set moments. What it printed is kept in the
pieces the terminal handed it over, as recorded/<name>.json, for run.py to
draw and compare with the core's screen.
"""

import base64
import fcntl
import json
import os
import pty
import select
import struct
import subprocess
import tempfile
import termios
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
COLS, ROWS = 80, 24

SAMPLE = "\n".join(
    [
        "Plain ASCII, then accents: café, naïve, Zürich.",
        "Wide: 中文 한글 日本語, and emoji: 😀 👍🏽 👨‍👩‍👧.",
        "Combining: é ạ̈, and box drawing: ┌─┬─┐ ╔═╗ ▗▖▝▘ █▌.",
        "\tA tab, then trailing spaces.   ",
        *(f"Line {n:03}: the quick brown fox jumps over the lazy dog {'*' * (n % 40)}" for n in range(1, 120)),
    ]
)

# What each program is, and the keys typed into it, each after a pause.
PROGRAMS: dict[str, tuple[list[str], list[tuple[float, bytes]]]] = {
    "nvim": (
        ["nvim", "--clean", "sample.txt"],
        [(1.5, b"10j"), (0.3, b"\x06"), (0.3, b"/fox\r"), (0.3, b"ccrewritten line \xe2\x9c\x93\x1b"), (0.3, b":vsplit\r"), (0.5, b"\x17l"), (0.3, b"G"), (0.5, b":qa!\r")],
    ),
    "vim": (
        ["vim", "--clean", "sample.txt"],
        [(1.5, b"\x06\x06"), (0.3, b"gg3dd"), (0.3, b"Oinserted \xc3\xa9 \xe4\xb8\xad\x1b"), (0.3, b":set number\r"), (0.5, b":q!\r")],
    ),
    "less": (
        ["less", "sample.txt"],
        [(1.0, b" "), (0.3, b" "), (0.3, b"/fox\r"), (0.3, b"n"), (0.3, b"G"), (0.3, b"g"), (0.5, b"q")],
    ),
    "git-log": (
        ["git", "--no-pager", "log", "--graph", "--all", "--decorate", "--color=always", "--format=%C(auto)%h%d %s %C(blue)%an"],
        [],
    ),
}


def repository(home: Path) -> None:
    """A repository with branches and merges, for `git log --graph`."""
    env = {**os.environ, "HOME": str(home), "GIT_AUTHOR_NAME": "Ada", "GIT_AUTHOR_EMAIL": "ada@example.org", "GIT_COMMITTER_NAME": "Ada", "GIT_COMMITTER_EMAIL": "ada@example.org", "GIT_AUTHOR_DATE": "2026-01-01T00:00:00Z", "GIT_COMMITTER_DATE": "2026-01-01T00:00:00Z"}

    def git(*args: str) -> None:
        subprocess.run(["git", *args], cwd=home, env=env, check=True, capture_output=True)

    git("init", "-q", "-b", "main")
    for n in range(1, 4):
        git("commit", "-q", "--allow-empty", "-m", f"Commit {n} on main")
    for branch in ("feature", "fix"):
        git("switch", "-q", "-c", branch, "main~1")
        for n in range(1, 3):
            git("commit", "-q", "--allow-empty", "-m", f"Work {n} on {branch} ✓")
        git("switch", "-q", "main")
        git("merge", "-q", "--no-ff", "-m", f"Merge {branch}", branch)


def record(argv: list[str], keys: list[tuple[float, bytes]], home: Path) -> list[bytes]:
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(home)
        os.execvpe(argv[0], argv, {"PATH": os.environ["PATH"], "HOME": str(home), "TERM": "xterm-256color", "LANG": "C.UTF-8", "COLORTERM": "truecolor"})
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    chunks: list[bytes] = []
    pending = list(keys)
    due = time.monotonic() + (pending[0][0] if pending else 0)
    while True:
        timeout = max(0.0, due - time.monotonic()) if pending else 2.0
        ready, _, _ = select.select([fd], [], [], timeout)
        if ready:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            chunks.append(data)
        elif pending:
            os.write(fd, pending.pop(0)[1])
            due = time.monotonic() + (pending[0][0] if pending else 0)
        else:
            break
    os.close(fd)
    os.waitpid(pid, 0)
    return chunks


def main() -> None:
    out = HERE / "recorded"
    out.mkdir(exist_ok=True)
    for name, (argv, keys) in PROGRAMS.items():
        with tempfile.TemporaryDirectory(dir=HERE.parents[2] / ".dev") as directory:
            home = Path(directory)
            (home / "sample.txt").write_text(SAMPLE + "\n")
            if name == "git-log":
                repository(home)
            chunks = record(argv, keys, home)
        recording = {"program": " ".join(argv), "cols": COLS, "rows": ROWS, "chunks": [base64.b64encode(chunk).decode() for chunk in chunks]}
        (out / f"{name}.json").write_text(json.dumps(recording, indent=1) + "\n")
        print(f"{name}: {len(chunks)} chunks, {sum(map(len, chunks))} bytes")


if __name__ == "__main__":
    main()
