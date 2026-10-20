"""The console's terminal in real browsers, without iglud, a workspace or a
shell: the terminal tier.

    python3 console/test/terminal/run.py [--browser NAME]... [--built DIR]
        [--runs N] [--seed N] [CASE]...

It bundles harness.ts, a page with the console's own TerminalPane on it
(build.mjs), and serves it on loopback. Each case opens
columns on a fresh page and plays the program in each: it writes what the
program would print to the pane's WebSocket, and reads what the terminal
sends back, byte for byte. So a case says exactly what reaches a program,
where a test through a shell can only say what the shell made of it.

Terminal behaviour belongs here. What needs the rest of iglu, such as the
round trip through a session, iglud's preferences or the console around a
column, belongs in nix/tests/browser.py, which `just dev-check` and the VM
test run. Each case is named for the bug it guards, as those steps are.

The differential case checks wterm's renderer against Ghostty's terminal
core, which holds the screen it draws, on generated output (streams.ts) and
on output recorded from real programs (recorded/, made by record.py).
"""

import argparse
import base64
import functools
import http.server
import io
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any, Callable

from PIL import Image
from playwright.sync_api import Browser, Page, WebSocketRoute, expect, sync_playwright

HERE = Path(__file__).resolve().parent
CONSOLE = HERE.parents[1]
TYPES = {".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".woff2": "font/woff2", ".html": "text/html"}


class Session:
    """The session behind one column's WebSocket: the program in it writes
    to the terminal, and reads what the terminal sends, as bytes."""

    def __init__(self, page: Page, name: str) -> None:
        self.page = page
        self.name = name
        self.socket: WebSocketRoute | None = None
        self.sent = bytearray()
        self.sizes: list[Any] = []

    def attached(self, socket: WebSocketRoute) -> None:
        self.socket = socket
        socket.on_message(self.message)

    def message(self, message: str | bytes) -> None:
        if isinstance(message, bytes):
            self.sent += message
        else:
            self.sizes.append(json.loads(message))

    def write(self, data: str | bytes, chunk: int = 0) -> None:
        """Prints `data`, in pieces of `chunk` bytes if given, as a socket
        may deliver it."""
        assert self.socket, f"{self.name} isn't attached"
        raw = data.encode() if isinstance(data, str) else data
        step = chunk or len(raw) or 1
        for start in range(0, len(raw), step):
            self.socket.send(raw[start : start + step])

    def prints(self, data: str, mark: str) -> None:
        """Prints `data`, then `mark`, and waits for the mark to show, so
        the terminal has taken what came before it, modes included, before
        the next key."""
        self.write(data + mark)
        self.shows(mark)

    def screen(self) -> str:
        return str(self.page.evaluate("(s) => harness.screen(s)", self.name))

    def shows(self, text: str, timeout: float = 10) -> str:
        """Waits for the screen to show `text`, though it wraps across rows."""
        deadline = time.monotonic() + timeout
        while text not in self.screen().replace("\n", ""):
            assert time.monotonic() < deadline, f"{text!r} never showed: {self.screen()!r}"
            self.page.wait_for_timeout(50)
        return self.screen()

    def receives(self, expected: bytes, timeout: float = 10) -> bytes:
        """Waits until the terminal has sent `expected`, and nothing else."""
        deadline = time.monotonic() + timeout
        while len(self.sent) < len(expected):
            assert time.monotonic() < deadline, {"sent": bytes(self.sent), "expected": expected}
            self.page.wait_for_timeout(50)
        # Anything more would come at once.
        self.page.wait_for_timeout(300)
        assert bytes(self.sent) == expected, {"sent": bytes(self.sent), "expected": expected}
        self.sent.clear()
        return expected

    def settled(self, quiet: float = 1.0, timeout: float = 10) -> bytes:
        """What the terminal sent, once it has stopped sending for `quiet` seconds."""
        deadline = time.monotonic() + timeout
        last, since = -1, time.monotonic()
        while time.monotonic() - since < quiet:
            assert time.monotonic() < deadline, bytes(self.sent)
            if len(self.sent) != last:
                last, since = len(self.sent), time.monotonic()
            self.page.wait_for_timeout(50)
        return bytes(self.sent)

    def column(self):
        return self.page.locator(f'section[data-column="{self.name}"]')

    def rows(self):
        """The screen's rows as drawn, below any history."""
        return self.column().locator(".term-row:not(.term-scrollback-row)")

    def row_height(self) -> float:
        return float(self.column().locator(".wterm").evaluate("(w) => parseFloat(getComputedStyle(w).getPropertyValue('--term-row-height'))"))


class Files(http.server.SimpleHTTPRequestHandler):
    """The page build.mjs made, served as iglud serves the console's."""

    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, **TYPES}

    def log_message(self, format: str, *args: Any) -> None:
        pass


def serve(built: Path) -> http.server.ThreadingHTTPServer:
    """Serves `built` on loopback, which browsers count as secure, as the
    clipboard needs, from a thread of its own."""
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Files, directory=str(built)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class Harness:
    """A fresh page with the harness on it, in a context of its own."""

    def __init__(self, browser: Browser, origin: str, *, mac: bool = False) -> None:
        self.origin = origin
        self.context = browser.new_context(viewport={"width": 1440, "height": 900})
        self.context.set_default_timeout(15_000)
        if browser.browser_type.name == "chromium":
            self.context.grant_permissions(["clipboard-read", "clipboard-write"], origin=origin)
        if mac:
            self.context.add_init_script("Object.defineProperty(Navigator.prototype, 'platform', { get: () => 'MacIntel' })")
        self.page = self.context.new_page()
        self.logged: list[str] = []
        self.page.on("console", lambda message: self.logged.append(f"{message.type}: {message.text}"))
        self.page.on("pageerror", lambda error: self.logged.append(f"error: {error}"))
        self.sessions: dict[str, Session] = {}
        self.page.route_web_socket(re.compile(r"/attach"), self.attach)
        self.page.goto(f"{origin}/")
        deadline = time.monotonic() + 15
        while self.page.evaluate("typeof globalThis.harness") == "undefined":
            assert time.monotonic() < deadline, "the harness never loaded"
            self.page.wait_for_timeout(20)

    def attach(self, socket: WebSocketRoute) -> None:
        name = re.search(r"/columns/([^/]+)/attach", socket.url)
        assert name, socket.url
        self.sessions[name[1]].attached(socket)

    def open(self, name: str) -> Session:
        """Opens a column, and waits until its terminal is drawn and attached."""
        session = self.sessions[name] = Session(self.page, name)
        self.page.evaluate("(s) => harness.open(s)", name)
        deadline = time.monotonic() + 15
        while session.socket is None:
            assert time.monotonic() < deadline, f"{name} never attached"
            self.page.wait_for_timeout(20)
        expect(session.rows().first).to_be_visible()
        return session

    def close(self) -> None:
        self.context.close()


def focused_column(page: Page) -> str | None:
    return page.evaluate("document.activeElement?.closest('.term-host')?.closest('[data-column]')?.dataset.column ?? null")


def select_text(session: Session) -> None:
    """Drags across the start of the screen, and checks the terminal kept the keyboard."""
    page = session.page
    box = session.rows().first.bounding_box()
    assert box
    page.mouse.move(box["x"] + 10, box["y"] + 5)
    page.mouse.down()
    page.mouse.move(box["x"] + 160, box["y"] + 40, steps=8)
    page.mouse.up()
    assert page.evaluate("getSelection().toString()"), "nothing selected"
    assert focused_column(page) == session.name, focused_column(page)


def text_to_select(session: Session) -> None:
    """A few rows of text, for a selection to cover."""
    session.write("".join(f"some text to select, row {n}\r\n" for n in range(1, 6)))
    session.shows("row 5")


def answers_queries(h: Harness) -> Any:
    """The terminal answers the queries shells wait on, in the order they
    were asked: fish held every keystroke for ten seconds after each command
    when it didn't. Asked what it is, it says it can set the clipboard, which
    nvim checks before copying through it, and gives its name to programs
    that ask. The console's own answers come in order among wterm's, though
    a query arrives split between two pieces of output."""
    s = h.open("one")
    s.write("\x1b[c\x1b[")
    s.write(">c\x1b]12;?\x07\x1b]11;?\x07\x1b[>q")
    deadline = time.monotonic() + 10
    pattern = re.compile(rb"^\x1b\[\?62;22;52c\x1b\[>1;10;0c\x1b\]12;rgb:[0-9a-f/]{14}\x07\x1b\]11;rgb:[0-9a-f/]{14}(\x07|\x1b\\)\x1bP>\|iglu\x1b\\$")
    while not pattern.match(bytes(s.sent)):
        assert time.monotonic() < deadline, bytes(s.sent)
        h.page.wait_for_timeout(50)
    return {"answers": bytes(s.sent).decode()}


def names_itself(h: Harness) -> Any:
    """A program's title is what its column shows, as plain text, and a new
    one takes its place."""
    s = h.open("one")
    for title, shown in (("title-one", "title-one"), ("two\u202eturned  round", "two turned round")):
        s.write(f"\x1b]2;{title}\x07")
        deadline = time.monotonic() + 10
        while h.page.evaluate("(s) => harness.title(s)", "one") != shown:
            assert time.monotonic() < deadline, h.page.evaluate("(s) => harness.title(s)", "one")
            h.page.wait_for_timeout(50)
    return {"title": shown}


# Four quadrants in red that meet in a square across two cells and two
# rows, and box crosses, light and double, in green; then dim and concealed
# text on a blue background.
BLOCKS = (
    "\x1b[H\x1b[2J\x1b[38;2;255;0;0m▗▖\r\n▝▘ \x1b[38;2;0;255;0m╶─┼─╴╬\x1b[0m\r\n"
    "\x1b[2;48;2;0;0;200m dim \x1b[0m \x1b[8;48;2;0;0;200m hid \x1b[0m\r\nblocks-end\r\n"
)


def draws_blocks(h: Harness) -> Any:
    """Block glyphs that meet across cells meet without a seam, and box
    strokes are whole pixels: wterm split quadrants at 8.5px, so Claude
    Code's logo showed a faint line under each eye and arm, and drew 1px
    strokes across two pixels at half strength. Dim and concealed text keep
    their background, which faded and vanished with the text."""
    s = h.open("one")
    s.write(BLOCKS)
    s.shows("blocks-end")
    h.page.evaluate("() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))")
    rows = s.rows()
    pixels = [px for row in (0, 1) for px in Image.open(io.BytesIO(rows.nth(row).screenshot())).convert("RGB").get_flattened_data()]
    # Every pixel the glyph touches is its colour exactly; a blended one is
    # an edge drawn between two pixels.
    found = {}
    for label, ink, touched in (
        ("quadrants", (255, 0, 0), lambda r, g, b: r - max(g, b) > 24),
        ("box", (0, 255, 0), lambda r, g, b: g - max(r, b) > 24),
    ):
        hits = [px for px in pixels if touched(*px)]
        blended = [px for px in hits if px != ink]
        assert hits, f"no {label} drawn"
        assert not blended, {label: len(blended), "of": len(hits), "some": sorted(set(blended))[:5]}
        found[label] = len(hits)
    # Ten cells of background, less the dim text's ink.
    backed = Image.open(io.BytesIO(rows.nth(2).screenshot())).convert("RGB").get_flattened_data()
    found["backgrounds"] = sum(px == (0, 0, 200) for px in backed)
    cell = rows.nth(2).evaluate("(r) => r.getBoundingClientRect().height") * 8
    assert found["backgrounds"] > 7.5 * cell, found
    return found


# Font Awesome's house, a nerd-font icon, in red, followed by green text:
# first after a blank cell, then right against it.
ICONS = "\x1b[H\x1b[2J\x1b[38;2;255;0;0m \x1b[38;2;0;255;0mab\r\n\x1b[38;2;255;0;0m\x1b[38;2;0;255;0mab\x1b[0m\r\nicons-end\r\n"

# How wide a character is at 100px in the icon font, falling back to
# `monospace`, and in `monospace` alone.
ICON_FONT_WIDTH = """(text) => {
    const context = document.createElement("canvas").getContext("2d");
    const width = (font) => { context.font = `100px ${font}`; return context.measureText(text).width; };
    return [width("'Symbols Nerd Font Mono', monospace"), width("monospace")];
}"""


def draws_icons(h: Harness) -> Any:
    """Nerd-font icons show without a nerd font installed, from the one the
    console serves, as Ghostty shows them from its own. Each fits its cell,
    or two when a blank follows, so it neither covers the text after it nor
    moves it: at its own width, an em, it spilled over most of the next cell.
    The font has a few characters that aren't icons, such as a prompt's ❯,
    which it drew an em wide over the text after it: they're left to the
    device's fonts, as before."""
    s = h.open("one")
    s.write(ICONS)
    s.shows("icons-end")
    h.page.wait_for_function("[...document.fonts].some((f) => f.family.replaceAll('\"', '') === 'Symbols Nerd Font Mono' && f.status === 'loaded')")
    h.page.evaluate("() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))")
    cell = float(s.column().locator(".wterm").evaluate("(w) => parseFloat(getComputedStyle(w).getPropertyValue('--term-cell-width'))"))
    found: dict[str, Any] = {}
    for row, label, cells in ((0, "spaced", 2), (1, "against", 1)):
        image = Image.open(io.BytesIO(s.rows().nth(row).screenshot())).convert("RGB")
        inked = [(x, image.getpixel((x, y))) for x in range(image.width) for y in range(image.height)]
        icon = [x for x, (r, g, b) in inked if r - max(g, b) > 24]
        text = [x for x, (r, g, b) in inked if g - max(r, b) > 24]
        assert icon and text, {label: {"icon": len(icon), "text": len(text)}}
        found[label] = {"cell": cell, "icon": [min(icon), max(icon)], "text": min(text)}
        assert max(icon) < cells * cell, found
        assert min(text) >= cells * cell, found
    # The house is wider than a cell: given two, it takes them.
    assert found["spaced"]["icon"][1] >= cell, found
    house, prompt = h.page.evaluate(ICON_FONT_WIDTH, ""), h.page.evaluate(ICON_FONT_WIDTH, "❯")
    found["widths"] = {"house": house, "prompt": prompt}
    assert house[0] == 100, found
    assert prompt[0] == prompt[1], found
    return found


def wheels_rows(h: Harness) -> Any:
    """A program tracking the mouse gets a wheel report for each row a swipe
    travels: a trackpad's many small deltas were a report each, and
    scrolled it many times too far."""
    s = h.open("one")
    s.prints("\x1b[?1049h\x1b[?1000h\x1b[?1006h", "ready")
    box = s.column().locator(".term-host").bounding_box()
    assert box
    h.page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    row = s.row_height()
    for _ in range(13):
        h.page.mouse.wheel(0, row / 4)
    reports = re.findall(rb"\x1b\[<65;\d+;\d+M", s.settled())
    assert len(reports) == 3, s.sent
    return {"reports": len(reports)}


def wheels_pager(h: Harness) -> Any:
    """On the alternate screen, the wheel sends arrow keys unless the
    program tracks the mouse, so pagers and editors scroll with it; it did
    nothing there, with no history to scroll."""
    s = h.open("one")
    s.prints("\x1b[?1049h", "ready")
    box = s.column().locator(".term-host").bounding_box()
    assert box
    h.page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    row = s.row_height()
    h.page.mouse.wheel(0, row)
    h.page.mouse.wheel(0, -row)
    return {"sent": s.receives(b"\x1b[B\x1b[A").decode()}


def clipboard(page: Page) -> str:
    return str(page.evaluate("navigator.clipboard.readText()"))


def copies_out(h: Harness) -> Any:
    """A program's copy (OSC 52) reaches the page's clipboard, and a large
    one passes without stopping output: a 150 KB copy once wedged the
    terminal until a reload. The large one arrives in pieces, as a socket
    delivers it."""
    s = h.open("one")
    s.write(f"\x1b]52;c;{base64.b64encode(b'copied-small').decode()}\x07small\r\n")
    s.shows("small")
    deadline = time.monotonic() + 10
    while clipboard(h.page) != "copied-small":
        assert time.monotonic() < deadline, clipboard(h.page)
        h.page.wait_for_timeout(100)
    s.write("\x1b]52;c;" + "A" * 150_000 + "\x07large\r\n", chunk=16 * 1024)
    s.shows("large")
    s.write("after\r\n")
    s.shows("after")
    return {"copied": "copied-small"}


def inserts_text(h: Harness) -> Any:
    """Text that arrives without key presses, as dictation, an input method
    or a phone's keyboard sends it, reaches the program; it was dropped."""
    s = h.open("one")
    s.column().locator(".term-host").click()
    h.page.keyboard.insert_text("echo inserted")
    return {"sent": s.receives(b"echo inserted").decode()}


def opens_links(h: Harness) -> Any:
    """A URL printed as plain text opens in a new tab on a modifier-click,
    though it wraps across rows: programs print most links as text, and
    those couldn't be opened at all."""
    s = h.open("one")
    # Long enough to wrap.
    url = f"{h.origin}/links/{'x' * 120}"
    s.write(f"{url}\r\nend\r\n")
    s.shows("end")
    start = s.rows().filter(has_text="links/").last
    # The row after it continues the URL.
    wrapped = start.locator("xpath=following-sibling::div[contains(@class, 'term-row')][1]")
    with h.context.expect_page() as opened:
        wrapped.click(position={"x": 20, "y": 8}, modifiers=["ControlOrMeta"])
    tab = opened.value
    try:
        assert tab.url == url, {"opened": tab.url, "printed": url}
    finally:
        tab.close()
    return {"url": url}


def copies_history(h: Harness) -> Any:
    """A selection dragged through the history, scrolling as it goes, copies
    every line in it: rows scrolled out of the page while it was made were
    left out of the copy."""
    s = h.open("one")
    page = h.page
    s.write("\x1b[3J\x1b[2J\x1b[H" + "".join(f"{n}\r\n" for n in range(1, 1001)) + "end\r\n")
    s.shows("end")
    box = s.column().locator(".wterm").bounding_box()
    assert box
    page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    page.mouse.wheel(0, -2500)
    page.wait_for_timeout(300)
    start = next(
        row
        for row in (r.bounding_box() for r in s.column().locator(".term-scrollback-row").all())
        if row and row["y"] > box["y"] + 4
    )
    bottom = box["y"] + box["height"] - 20
    page.mouse.move(start["x"] + 1, start["y"] + 8)
    page.mouse.down()
    page.mouse.move(start["x"] + 150, bottom, steps=10)
    page.mouse.wheel(0, 1700)
    page.wait_for_timeout(500)
    page.mouse.move(start["x"] + 180, bottom, steps=4)
    page.mouse.up()
    page.evaluate("document.execCommand('copy')")
    copied = clipboard(page)
    numbers = [int(line) for line in copied.split("\n") if line.strip().isdigit()]
    rows = int(box["height"] // s.row_height())
    assert len(numbers) > rows, {"copied": len(numbers), "screen": rows}
    missing = [n for n in range(numbers[0], numbers[-1] + 1) if n not in numbers]
    assert not missing and numbers == sorted(numbers), {"missing": missing[:20], "first": numbers[0], "last": numbers[-1]}
    assert not any(line.endswith(" ") for line in copied.split("\n")), {"padding": repr(copied[-40:])}
    return {"from": numbers[0], "to": numbers[-1]}


def mac_keys(h: Harness) -> Any:
    """On a Mac, where Cmd copies and pastes, Ctrl+C and Ctrl+V reach the
    program even with text selected: the terminal ate Ctrl+C while anything
    was selected and kept Ctrl+V for a paste that never came. Ctrl+Option
    and a letter is Meta with its control byte, which sent nothing, and an
    Option that isn't Meta types its character, which went as Meta anyway."""
    h.page.evaluate("() => harness.keyboard({ option_as_meta: 'off' })")
    s = h.open("one")
    text_to_select(s)
    select_text(s)
    h.page.keyboard.press("Control+c")
    select_text(s)
    h.page.keyboard.press("Control+v")
    h.page.keyboard.press("Escape")
    s.receives(b"\x03\x16\x1b")
    h.page.keyboard.press("Control+Alt+b")
    h.page.keyboard.press("Alt+b")
    return {"sent": s.receives(b"\x1b\x02b").decode()}


def selected_keys(h: Harness) -> Any:
    """With text selected, Escape only clears the selection, since a stray
    interrupt costs an agent's work; and a modifier tapped meanwhile isn't
    held for later keys, which then reached a kitty-keyboard program as Alt
    and Shift arrows. Ctrl+Alt and a letter is Meta with its control byte,
    and with any other key is left to type, as AltGr does: it sent C-M-[,
    a second Escape, for AltGr+8."""
    s = h.open("one")
    # The kitty keyboard protocol's first level.
    s.prints("\x1b[>1u", "kitty-on")
    text_to_select(s)
    select_text(s)
    h.page.keyboard.press("Escape")
    assert not h.page.evaluate("getSelection().toString()"), "still selected"
    select_text(s)
    h.page.keyboard.press("Shift")
    h.page.keyboard.press("Alt")
    h.page.keyboard.press("ArrowUp")
    s.receives(b"\x1b[A")
    s.prints("\x1b[<u", "kitty-off")
    h.page.keyboard.press("Control+Alt+q")
    h.page.keyboard.press("Control+Alt+Digit8")
    h.page.keyboard.type("x")
    return {"sent": s.receives(b"\x1b\x11x").decode()}


def differential(h: Harness, runs: int, seed: int) -> Any:
    """What wterm draws is what Ghostty's core holds, for generated output
    in random pieces and at sizes small enough to wrap, and for output
    recorded from real programs: the renderer once drew rows the core no
    longer held."""
    result = h.page.evaluate("([runs, seed]) => harness.differential(runs, seed)", [runs, seed])
    assert not result["failed"], result
    programs = {}
    for file in sorted((HERE / "recorded").glob("*.json")):
        recording = json.loads(file.read_text())
        mismatches = h.page.evaluate("([chunks, cols, rows]) => harness.recorded(chunks, cols, rows)", [recording["chunks"], recording["cols"], recording["rows"]])
        assert not mismatches, {file.stem: mismatches[:5]}
        programs[file.stem] = len(recording["chunks"])
    return {"runs": runs, "seed": result["seed"], "programs": programs}


# Each case, and the engines it runs in. Reading the clipboard back needs
# a permission only Chromium grants Playwright.
CASES: dict[str, tuple[Callable[..., Any], tuple[str, ...]]] = {
    "answers-queries": (answers_queries, ("chromium", "webkit")),
    "names-itself": (names_itself, ("chromium", "webkit")),
    "draws-blocks": (draws_blocks, ("chromium", "webkit")),
    "draws-icons": (draws_icons, ("chromium", "webkit")),
    "wheels-rows": (wheels_rows, ("chromium", "webkit")),
    "wheels-pager": (wheels_pager, ("chromium", "webkit")),
    "copies-out": (copies_out, ("chromium",)),
    "inserts-text": (inserts_text, ("chromium", "webkit")),
    "opens-links": (opens_links, ("chromium", "webkit")),
    "copies-history": (copies_history, ("chromium",)),
    "mac-keys": (mac_keys, ("chromium", "webkit")),
    "selected-keys": (selected_keys, ("chromium", "webkit")),
    "differential": (differential, ("chromium", "webkit")),
}


def engine_env(engine: str, env: dict[str, str]) -> dict[str, str]:
    """WebKit draws through EGL, which a machine without a GPU, such as a
    workspace or a build sandbox, lacks: given Mesa's (IGLU_MESA_EGL, which
    the dev shell sets), it draws without one. Only WebKit is told, so other
    programs keep the machine's own driver."""
    mesa = env.get("IGLU_MESA_EGL")
    if engine != "webkit" or not mesa:
        return env
    return {**env, "__EGL_VENDOR_LIBRARY_DIRS": mesa, "EGL_PLATFORM": "surfaceless"}


def bundle(out: Path) -> None:
    subprocess.run(["node", str(HERE / "build.mjs"), str(out)], cwd=CONSOLE, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("cases", nargs="*", metavar="CASE", help=f"cases to run, all by default: {', '.join(CASES)}")
    parser.add_argument("--browser", action="append", choices=["chromium", "webkit"], help="engines to run in; both by default")
    parser.add_argument("--built", type=Path, help="a page build.mjs already made; built afresh otherwise")
    parser.add_argument("--runs", type=int, default=300, help="generated streams the differential case checks")
    parser.add_argument("--seed", type=int, default=int(time.time()), help="the differential case's seed, to repeat a run")
    args = parser.parse_args()
    if unknown := set(args.cases) - set(CASES):
        parser.error(f"no such case: {', '.join(sorted(unknown))}")
    built = args.built or CONSOLE.parent / ".dev/terminal"
    if not args.built:
        bundle(built)
    built = built.resolve()
    shots = Path(os.environ.get("TERMINAL_SHOTS", CONSOLE.parent / ".dev/terminal-shots"))
    engines = args.browser or ["chromium", "webkit"]
    failed: list[str] = []
    started = time.monotonic()
    server = serve(built)
    origin = f"http://localhost:{server.server_address[1]}"
    # `dev/cover.py terminal` sets this, to count the console's lines the browser runs.
    if into := os.environ.get("IGLU_COVERAGE_BROWSER"):
        sys.path.insert(0, str(CONSOLE.parent / "dev"))
        import cover

        cover.watch_browsers(Path(into))
    with sync_playwright() as playwright:
        # The dev shell's browsers bring their own libraries; ones the machine
        # puts on the library path can be built against another glibc.
        env = {key: value for key, value in os.environ.items() if key != "LD_LIBRARY_PATH"}
        for engine in engines:
            browser = getattr(playwright, engine).launch(env=engine_env(engine, env), args=["--disable-dev-shm-usage"] if engine == "chromium" else [])
            try:
                browser.new_page().close()
            except Exception as error:
                failed.append(engine)
                print(f"FAILED {engine} can't open a page: {error!r}. Without a GPU, WebKit needs IGLU_MESA_EGL; see engine_env.", file=sys.stderr)
                browser.close()
                continue
            for name, (case, runs_in) in CASES.items():
                if (args.cases and name not in args.cases) or engine not in runs_in:
                    continue
                opening = time.monotonic()
                h: Harness | None = None
                try:
                    h = Harness(browser, origin, mac=name == "mac-keys")
                    began = time.monotonic()
                    result = case(h, args.runs, args.seed) if name == "differential" else case(h)
                    took = f"{time.monotonic() - began:.1f}s, page {began - opening:.1f}s"
                    print(f"ok {engine} {name} ({took}) {json.dumps(result)[:200]}", flush=True)
                except Exception as error:
                    failed.append(f"{engine} {name}")
                    print(f"FAILED {engine} {name}: {error!r}", file=sys.stderr, flush=True)
                    if h:
                        shots.mkdir(parents=True, exist_ok=True)
                        h.page.screenshot(path=shots / f"{engine}-{name}.png")
                        print("\n".join(h.logged[-30:]), file=sys.stderr)
                finally:
                    if h:
                        h.close()
            browser.close()
    print(f"{'failed: ' + ', '.join(failed) if failed else 'all cases passed'} in {time.monotonic() - started:.1f}s")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
