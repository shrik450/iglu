"""Drives Chromium through the console the way a person would.

Each run performs one step and prints its result as JSON. Cookies, including
session cookies, carry over between runs in a state file, so steps build on
each other like one long browser session. A failing step leaves a screenshot.

    browser <step> [args...]

The VM test runs it against its control box. `just dev-check` runs the same
steps against the local dev stack by setting IGLU_CONSOLE, BROWSER_STATE and
BROWSER_SHOTS. BROWSER_RESOLVER_RULES is only for where DNS doesn't resolve
the console.
"""

import io
import json
import os
import re
import sys
import time
from pathlib import Path
from typing import Any, Callable

from PIL import Image
from playwright.sync_api import Locator, Page, expect, sync_playwright

CONSOLE = os.environ.get("IGLU_CONSOLE", "https://iglu.example.test")
STATE = Path(os.environ.get("BROWSER_STATE", "/root/browser-state.json"))
SHOTS = Path(os.environ.get("BROWSER_SHOTS", "/tmp/browser"))


def open_workspace(page: Page, name: str) -> None:
    page.goto(CONSOLE)
    page.get_by_role("link", name=name, exact=True).click()
    page.wait_for_url(f"{CONSOLE}/w/{name}")
    expect(page.locator('.wsv[data-phase="running"]')).to_be_visible(timeout=600_000)


def column(page: Page, name: str) -> Locator:
    """A column by its session name, which stays when the column is renamed."""
    return page.locator(f'section[data-column="{name}"]')


def columns(page: Page) -> list[str]:
    return [str(c.get_attribute("data-column")) for c in page.locator("[data-column]").all()]


def screen(page: Page) -> str:
    return str(page.evaluate("globalThis.iglu.screen()"))


def at_prompt(page: Page, name: str) -> None:
    """Clicks into a column's terminal and waits for its shell's prompt,
    whichever shell it is: text that has stopped changing."""
    page.wait_for_selector("[data-column]")
    column(page, name).locator(".term-host").click()
    deadline = time.monotonic() + 60
    shown = ""
    while not shown.strip() or shown != screen(page):
        assert time.monotonic() < deadline, f"no prompt: {screen(page)!r}"
        shown = screen(page)
        time.sleep(1)


def shows(page: Page, expected: str, times: int = 1) -> str:
    """Waits for the focused terminal to show `expected`, `times` times.
    Output wraps at the column's width; a narrow column breaks it across rows."""
    deadline = time.monotonic() + 60
    while screen(page).replace("\n", "").count(expected) < times:
        assert time.monotonic() < deadline, f"{expected!r} never appeared: {screen(page)!r}"
        time.sleep(0.5)
    return screen(page)


def run_in_column(page: Page, name: str, command: str, expected: str) -> str:
    """Types into a column's terminal, as a person would, and waits for output."""
    at_prompt(page, name)
    page.keyboard.type(command)
    page.keyboard.press("Enter")
    return shows(page, expected)


def focused_column(page: Page) -> str | None:
    """The column whose terminal has the keyboard, if one does."""
    return page.evaluate("document.activeElement?.closest('.term-host')?.closest('[data-column]')?.dataset.column ?? null")


def reaches(page: Page, name: str) -> None:
    """What's typed now lands in `name`'s shell. Focus comes back once what
    had it is gone, a task after it closes, so this waits a moment for it."""
    deadline = time.monotonic() + 2
    while focused_column(page) != name:
        assert time.monotonic() < deadline, {"focused": focused_column(page), "active": page.evaluate("document.activeElement?.outerHTML.slice(0, 120)")}
        time.sleep(0.05)
    mark = str(time.time_ns())
    # The shell prints what isn't typed: a line typed just after the column
    # changed size may be drawn at the old width until the shell hears of it.
    page.keyboard.type(f"printf 'here-%s\\n' {mark}")
    page.keyboard.press("Enter")
    shows(page, f"here-{mark}")


def wholly_shown(page: Page, name: str) -> bool:
    """Whether a column is all within the strip, not cut at either edge."""
    return bool(
        column(page, name).evaluate(
            """(col) => {
              const c = col.getBoundingClientRect(), s = col.closest('.w-cols').getBoundingClientRect();
              return c.left >= s.left - 1 && c.right <= s.right + 1;
            }"""
        )
    )


def prefix(page: Page, key: str) -> None:
    """The prefix, then a key for iglu."""
    page.keyboard.press("Control+Space")
    page.keyboard.press(key)


def on_console(page: Page) -> None:
    """Where the API is a fetch away: each step starts on a blank page."""
    if not page.url.startswith(CONSOLE):
        page.goto(CONSOLE)


def preferences(page: Page) -> Any:
    """The signed-in person's preferences, as iglud keeps them."""
    on_console(page)
    return page.evaluate("async () => (await fetch('/v1/me/preferences')).json()")


def keep_preferences(page: Page, chosen: Any) -> None:
    """Sets the signed-in person's preferences, whole."""
    on_console(page)
    status = page.evaluate(
        """async (chosen) => {
          const me = await (await fetch('/v1/me')).json();
          const headers = { 'content-type': 'application/json', 'x-csrf-token': me.csrf_token };
          return (await fetch('/v1/me/preferences', { method: 'PUT', headers, body: JSON.stringify(chosen) })).status;
        }""",
        chosen,
    )
    assert status == 204, status


def sign_in(page: Page) -> Any:
    page.goto(CONSOLE)
    page.locator("#username-textfield").fill("alice")
    page.locator("#password-textfield").fill("password")
    page.locator("#sign-in-button").click()
    page.wait_for_url(f"{CONSOLE}/**")
    expect(page.get_by_role("link", name="Settings, signed in as alice@example.org")).to_be_visible()
    return {"url": page.url}


def signed_out(page: Page, path: str) -> Any:
    """Someone without a session who opens a console page goes straight to
    sign in, without the console loading only to find out, and comes back to
    that page."""
    browser = page.context.browser
    assert browser is not None
    fresh = browser.new_context()
    other = fresh.new_page()
    asked: list[str] = []
    other.on("request", lambda r: asked.append(r.url) if r.url.startswith(f"{CONSOLE}/v1/") else None)
    other.goto(f"{CONSOLE}{path}")
    other.locator("#username-textfield").wait_for()
    assert not asked, asked
    other.locator("#username-textfield").fill("alice")
    other.locator("#password-textfield").fill("password")
    other.locator("#sign-in-button").click()
    other.wait_for_url(f"{CONSOLE}{path}")
    fresh.close()
    return {"returned": path}


def approve_cli(page: Page, url: str) -> Any:
    page.goto(url)
    page.get_by_role("button", name="Sign in the CLI").click()
    page.wait_for_url("http://127.0.0.1:*/**")
    expect(page.locator("body")).to_contain_text("Signed in.")
    return {}


def create(page: Page, environment: str, repo: str, name: str) -> Any:
    """Adds a project for `repo`, then its first workspace, `name`."""
    page.goto(CONSOLE)
    page.get_by_role("button", name="New project").click()
    project = page.get_by_role("form", name="New project")
    project.get_by_label("Repository").fill(repo)
    project.get_by_label("Environment").select_option(environment)
    project.get_by_role("button", name="Add").click()
    form = page.get_by_role("form", name="New workspace")
    # A project added for a repository is named after it.
    named = repo.rstrip("/").rsplit("/", 1)[-1].removesuffix(".git")
    expect(form.get_by_label("Project").locator("option:checked")).to_have_text(named)
    form.get_by_label("Name").fill(name)
    form.get_by_role("button", name="Create").click()
    page.wait_for_url(f"{CONSOLE}/w/{name}")
    expect(page.get_by_role("heading", name=name)).to_be_visible()
    expect(page.locator('.wsv[data-phase="running"]')).to_be_visible(timeout=600_000)
    return {"name": name, "columns": columns(page)}


def refused_environment(page: Page) -> Any:
    """Adds an environment whose flake has no #attribute; iglu refuses it,
    and the console shows why by the Flake input."""
    page.goto(f"{CONSOLE}/settings")
    form = page.get_by_role("form", name="Add an environment")
    form.get_by_label("Name").fill("broken")
    flake = form.get_by_label("Flake")
    flake.fill("github:you/env")
    form.get_by_role("button", name="Add").click()
    expect(flake).to_have_attribute("aria-invalid", "true")
    expect(flake).to_be_focused()
    expect(flake).to_have_accessible_description(re.compile("invalid environment source"))
    return {"error": form.locator(".field-err").inner_text()}


def project_agent(page: Page, project: str, agent: str) -> Any:
    """Sets the agent a project's workspaces start, and checks that it shows,
    survives saving other settings, and is what a new workspace offers."""
    page.goto(f"{CONSOLE}/p/{project}")
    settings = page.get_by_role("form", name=f"Settings for {project}")
    starts = settings.get_by_label("Starts")

    def save() -> None:
        with page.expect_response(lambda r: r.request.method == "PUT" and "/v1/projects/" in r.url) as saved:
            settings.get_by_role("button", name="Save").click()
        assert saved.value.ok, saved.value.text()
        page.reload()

    starts.select_option(agent)
    save()
    expect(starts).to_have_value(agent)
    # Saving something else keeps it.
    settings.get_by_label("Previews").fill("8000")
    save()
    expect(starts).to_have_value(agent)
    settings.get_by_label("Previews").fill("")
    save()
    page.get_by_role("button", name="New workspace").first.click()
    form = page.get_by_role("form", name="New workspace")
    expect(form.get_by_label("Prompt")).to_have_attribute("placeholder", f"What should {agent} do? Optional…")
    form.get_by_role("button", name="Cancel").click()
    return {"agent": starts.input_value()}


def terminal(page: Page, name: str, command: str, expected: str) -> Any:
    """Runs a command in the workspace's first column."""
    open_workspace(page, name)
    page.wait_for_selector("[data-column]")
    first = columns(page)[0]
    return {"column": first, "screen": run_in_column(page, first, command, expected)}


def new_column(page: Page, name: str, command: str, expected: str) -> Any:
    """Adds a shell column from the strip and runs a command in it."""
    open_workspace(page, name)
    page.wait_for_selector("[data-column]")
    before = columns(page)
    page.get_by_role("button", name="Add a column").click()
    page.get_by_role("group", name="Add a column").get_by_role("button", name="Shell").click()
    expect(page.locator("[data-column]")).to_have_count(len(before) + 1)
    added = next(c for c in columns(page) if c not in before)
    return {"columns": columns(page), "added": added, "screen": run_in_column(page, added, command, expected)}


# Asks the terminal for its device attributes and background colour, as
# fish does after every command, and the secondary attributes and cursor
# colour, which the console answers itself since wterm doesn't. It prints
# the answers without their ESC after a mark, so an earlier run's answers on
# screen don't count.
QUERIES = (
    "bash -c 'ask() {{ printf \"$1\" >/dev/tty; IFS= read -rs -t 5 -d \"$2\" a </dev/tty; printf \" %s\" \"${{a#?}}\"; }}; "
    "printf answers-{mark}; ask \"\\e[c\" c; ask \"\\e[>c\" c; "
    "ask \"\\e]12;?\\a\" $(printf \"\\a\"); ask \"\\e]11;?\\a\" $(printf \"\\a\"); echo'"
)


def answers_queries(page: Page, name: str) -> Any:
    """The terminal answers the queries shells wait on; fish held every
    keystroke for ten seconds after each command when it didn't."""
    open_workspace(page, name)
    first = columns(page)[0]
    mark = str(time.time_ns())
    shown = run_in_column(page, first, QUERIES.format(mark=mark), f"answers-{mark} [?1;2 [>1;10;0 ]12;rgb:")
    answers = re.search(rf"answers-{mark} (\S+ \S+ \]12;rgb:[0-9a-f/]+ \]11;rgb:[0-9a-f/]+)", shown.replace("\n", ""))
    assert answers, f"the console's own answers never came: {shown!r}"
    return {"column": first, "answers": answers and answers[1]}


# Four quadrants in red that meet in a square across two cells and two
# rows, and box crosses, light and double, in green, written as octal UTF-8
# so typing them is plain ASCII in any shell; then dim and concealed text on
# a blue background.
BLOCKS = (
    "clear; printf '\\e[38;2;255;0;0m\\342\\226\\227\\342\\226\\226\\n\\342\\226\\235\\342\\226\\230 "
    "\\e[38;2;0;255;0m\\342\\225\\266\\342\\224\\200\\342\\224\\274\\342\\224\\200\\342\\225\\264\\342\\225\\254\\e[0m\\n"
    "\\e[2;48;2;0;0;200m dim \\e[0m \\e[8;48;2;0;0;200m hid \\e[0m\\nblocks-%s\\n' {mark}"
)


def draws_blocks(page: Page, name: str) -> Any:
    """Block glyphs that meet across cells meet without a seam, and box
    strokes are whole pixels: wterm split quadrants at 8.5px, so Claude
    Code's logo showed a faint line under each eye and arm, and drew 1px
    strokes across two pixels at half strength. Dim and concealed text keep
    their background, which faded and vanished with the text."""
    open_workspace(page, name)
    first = columns(page)[0]
    mark = str(time.time_ns())
    run_in_column(page, first, BLOCKS.format(mark=mark), f"blocks-{mark}")
    # The screen's first two rows, below any history, hold only the glyphs.
    rows = column(page, first).locator(".term-row:not(.term-scrollback-row)")
    pixels = [px for row in (0, 1) for px in Image.open(io.BytesIO(rows.nth(row).screenshot())).convert("RGB").getdata()]
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
    backed = Image.open(io.BytesIO(rows.nth(2).screenshot())).convert("RGB").getdata()
    found["backgrounds"] = sum(px == (0, 0, 200) for px in backed)
    cell = rows.nth(2).evaluate("(r) => r.getBoundingClientRect().height") * 8
    assert found["backgrounds"] > 7.5 * cell, found
    return found


# A program on the alternate screen that doesn't track the mouse, like a
# pager: it shows a mark, then prints what the next six bytes it reads were.
PAGER = (
    "bash -c 'printf \"\\e[?1049hready-%s\" {mark}; IFS= read -rsn6 -t 20 k; "
    "printf \"\\e[?1049l\"; printf \"wheel-%s %q\\n\" {mark} \"$k\"'"
)


def wheels_pager(page: Page, name: str) -> Any:
    """On the alternate screen, the wheel sends arrow keys unless the
    program tracks the mouse, so pagers and editors scroll with it; it did
    nothing there, with no history to scroll."""
    open_workspace(page, name)
    first = columns(page)[0]
    mark = str(time.time_ns())
    run_in_column(page, first, PAGER.format(mark=mark), f"ready-{mark}")
    box = column(page, first).locator(".term-host").bounding_box()
    assert box
    page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    row = float(column(page, first).locator(".wterm").evaluate("(w) => parseFloat(getComputedStyle(w).getPropertyValue('--term-row-height'))"))
    page.mouse.wheel(0, row)
    page.mouse.wheel(0, -row)
    return {"screen": shows(page, f"wheel-{mark} $'\\E[B\\E[A'")}


def copies_history(page: Page, name: str) -> Any:
    """A selection dragged through the history, scrolling as it goes, copies
    every line in it: rows scrolled out of the page while it was made were
    left out of the copy."""
    page.context.grant_permissions(["clipboard-read", "clipboard-write"], origin=CONSOLE)
    open_workspace(page, name)
    first = columns(page)[0]
    mark = str(time.time_ns())
    run_in_column(page, first, f"printf '\\e[3J\\e[2J\\e[H'; seq 1 1000; printf 'end-%s\\n' {mark}", f"end-{mark}")
    box = column(page, first).locator(".wterm").bounding_box()
    assert box
    page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    page.mouse.wheel(0, -2500)
    time.sleep(0.3)
    start = next(
        row
        for row in (r.bounding_box() for r in column(page, first).locator(".term-scrollback-row").all())
        if row and row["y"] > box["y"] + 4
    )
    bottom = box["y"] + box["height"] - 20
    page.mouse.move(start["x"] + 1, start["y"] + 8)
    page.mouse.down()
    page.mouse.move(start["x"] + 150, bottom, steps=10)
    page.mouse.wheel(0, 1700)
    time.sleep(0.5)
    page.mouse.move(start["x"] + 180, bottom, steps=4)
    page.mouse.up()
    page.evaluate("document.execCommand('copy')")
    copied = str(page.evaluate("navigator.clipboard.readText()"))
    numbers = [int(line) for line in copied.split("\n") if line.strip().isdigit()]
    rows = int(box["height"] // 17)
    assert len(numbers) > rows, {"copied": len(numbers), "screen": rows}
    missing = [n for n in range(numbers[0], numbers[-1] + 1) if n not in numbers]
    assert not missing and numbers == sorted(numbers), {"missing": missing[:20], "first": numbers[0], "last": numbers[-1]}
    assert not any(line.endswith(" ") for line in copied.split("\n")), {"padding": repr(copied[-40:])}
    return {"from": numbers[0], "to": numbers[-1]}


def opens_links(page: Page, name: str) -> Any:
    """A URL printed as plain text opens in a new tab on a modifier-click,
    though it wraps across rows: programs print most links as text, and
    those couldn't be opened at all."""
    open_workspace(page, name)
    first = columns(page)[0]
    mark = str(time.time_ns())
    # Long enough to wrap, and put together by the shell, so only the output holds it whole.
    tail = "x" * 120
    url = f"{CONSOLE}/links-{mark}/{tail}"
    run_in_column(page, first, f"printf '%s/links-%s/%s\\n' {CONSOLE} {mark} {tail}; printf 'end-%s\\n' {mark}", f"end-{mark}")
    rows = column(page, first).locator(".term-row:not(.term-scrollback-row)")
    start = rows.filter(has_text=f"links-{mark}").last
    # The row after it continues the URL.
    wrapped = start.locator("xpath=following-sibling::div[contains(@class, 'term-row')][1]")
    with page.context.expect_page() as opened:
        wrapped.click(position={"x": 20, "y": 8}, modifiers=["ControlOrMeta"])
    tab = opened.value
    try:
        assert tab.url == url, {"opened": tab.url, "printed": url}
    finally:
        tab.close()
    return {"url": url}


def inserts_text(page: Page, name: str) -> Any:
    """Text that arrives without key presses, as dictation, an input method
    or a phone's keyboard sends it, reaches the terminal; it was dropped."""
    open_workspace(page, name)
    first = columns(page)[0]
    at_prompt(page, first)
    mark = str(time.time_ns())
    page.keyboard.insert_text(f"echo inserted-{mark}")
    page.keyboard.press("Enter")
    return {"screen": shows(page, f"inserted-{mark}", 2)}


# Puts the terminal in raw mode and prints the next three bytes it reads,
# giving up after 20 seconds without one.
RAW_READ = (
    "bash -c 'stty raw -echo min 0 time 200; printf ready-%s {mark}; k=$(dd bs=1 count=3 2>/dev/null); "
    "stty sane; printf \"\\r\\nread-%s %q\\r\\n\" {mark} \"$k\"'"
)


def select_text(page: Page, name: str) -> None:
    """Drags across the start of a column's screen, below its history, and
    checks the terminal kept the keyboard."""
    box = column(page, name).locator(".term-row:not(.term-scrollback-row)").first.bounding_box()
    assert box
    page.mouse.move(box["x"] + 10, box["y"] + 5)
    page.mouse.down()
    page.mouse.move(box["x"] + 160, box["y"] + 40, steps=8)
    page.mouse.up()
    assert page.evaluate("getSelection().toString()"), "nothing selected"
    assert focused_column(page) == name, focused_column(page)


# RAW_READ for a program using the kitty keyboard protocol's first level.
KITTY_READ = (
    "bash -c 'printf \"\\\\e[>1u\"; stty raw -echo min 0 time 200; printf ready-%s {mark}; k=$(dd bs=1 count=3 2>/dev/null); "
    "stty sane; printf \"\\\\e[<u\\\\r\\\\nread-%s %q\\\\r\\\\n\" {mark} \"$k\"'"
)


def selected_keys(page: Page, name: str) -> Any:
    """With text selected, Escape only clears the selection, since a stray
    interrupt costs an agent's work; and a modifier tapped meanwhile isn't
    held for later keys, which then reached a kitty-keyboard program as Alt
    and Shift arrows. Ctrl+Alt and a letter is Meta with its control byte,
    and with any other key is left to type, as AltGr does: it sent C-M-[,
    a second Escape, for AltGr+8."""
    open_workspace(page, name)
    first = columns(page)[0]
    mark = str(time.time_ns())
    run_in_column(page, first, KITTY_READ.format(mark=mark), f"ready-{mark}")
    select_text(page, first)
    page.keyboard.press("Escape")
    assert not page.evaluate("getSelection().toString()"), "still selected"
    select_text(page, first)
    page.keyboard.press("Shift")
    page.keyboard.press("Alt")
    page.keyboard.press("ArrowUp")
    shows(page, f"read-{mark} $'\\E[A'")
    mark = str(time.time_ns())
    run_in_column(page, first, RAW_READ.format(mark=mark), f"ready-{mark}")
    page.keyboard.press("Control+Alt+q")
    page.keyboard.press("Control+Alt+Digit8")
    page.keyboard.type("x")
    return {"screen": shows(page, f"read-{mark} $'\\E\\021x'")}


# The terminal's background, as a program asks for it, and its size, as the
# shell has it.
STYLE = (
    "bash -c 'printf \"\\e]11;?\\a\" >/dev/tty; IFS= read -rs -t 5 -d $(printf \"\\a\") a </dev/tty; "
    "printf \"style-{mark} %s \" \"${{a#?}}\"; stty size'"
)


def styles_terminal(page: Page, name: str) -> Any:
    """The terminal's type and colours are the person's: chosen in Settings
    in one browser, they reach a terminal open in another as they're chosen.
    Its cells grow whole pixels at a time, the shell learns its new size,
    and a program asking for the background gets the theme's. A theme
    pasted in Ghostty's format works the same."""
    open_workspace(page, name)
    first = columns(page)[0]
    term = column(page, first).locator(".term-host > .wterm")

    def cell_is(width: int, height: int) -> None:
        deadline = time.monotonic() + 5
        cell = "e => [getComputedStyle(e).getPropertyValue('--term-cell-width'), e.querySelector('.term-row').getBoundingClientRect().height]"
        while (now := term.evaluate(cell)) != [f"{width}px", height]:
            assert time.monotonic() < deadline, {"cell": now, "wanted": [width, height]}
            time.sleep(0.05)

    def style(expected: str = "") -> tuple[str, int]:
        mark = str(time.time_ns())
        shown = run_in_column(page, first, STYLE.format(mark=mark), f"style-{mark} {expected}")
        found = re.search(rf"style-{mark} (\S+) (\d+) (\d+)", shown.replace("\n", ""))
        assert found, shown
        return found[1], int(found[3])

    cell_is(8, 17)
    _, before = style()
    kept = preferences(page)
    browser = page.context.browser
    assert browser is not None
    elsewhere = browser.new_context()
    try:
        settings = elsewhere.new_page()
        sign_in(settings)
        settings.goto(f"{CONSOLE}/settings")
        section = settings.locator("section", has=settings.get_by_role("heading", name="Terminal"))
        section.get_by_label("Colours").select_option("Dracula")
        section.get_by_label("Size").select_option(label="17px")
        expect(term).to_have_css("background-color", "rgb(40, 42, 54)")
        cell_is(10, 21)
        background, after = style("]11;rgb:2828/2a2a/3636")
        assert after < before, {"before": before, "after": after}
        section.get_by_label("Colours").select_option("pasted")
        section.get_by_label("A theme in Ghostty's format").fill("background = #102030\nforeground = #e0e0e0\n")
        expect(term).to_have_css("background-color", "rgb(16, 32, 48)")
    finally:
        keep_preferences(page, kept)
        elsewhere.close()
    cell_is(8, 17)
    return {"background": background, "cols": [before, after]}


def mac_keys(page: Page, name: str) -> Any:
    """On a Mac, where Cmd copies and pastes, Ctrl+C and Ctrl+V reach the
    program even with text selected: the terminal ate Ctrl+C while anything
    was selected and kept Ctrl+V for a paste that never came. Ctrl+Option
    and a letter is Meta with its control byte, which sent nothing, and an
    Option that isn't Meta types its character, which went as Meta anyway."""
    kept = preferences(page)
    keep_preferences(page, {**kept, "keyboard": {**kept["keyboard"], "option_as_meta": "off"}})
    mac = page.context.new_page()
    mac.add_init_script("Object.defineProperty(Navigator.prototype, 'platform', { get: () => 'MacIntel' })")
    try:
        open_workspace(mac, name)
        first = columns(mac)[0]
        mark = str(time.time_ns())
        run_in_column(mac, first, RAW_READ.format(mark=mark), f"ready-{mark}")
        select_text(mac, first)
        mac.keyboard.press("Control+c")
        select_text(mac, first)
        mac.keyboard.press("Control+v")
        mac.keyboard.press("Escape")
        shows(mac, f"read-{mark} $'\\003\\026\\E'")
        mark = str(time.time_ns())
        run_in_column(mac, first, RAW_READ.format(mark=mark), f"ready-{mark}")
        mac.keyboard.press("Control+Alt+b")
        mac.keyboard.press("Alt+b")
        return {"screen": shows(mac, f"read-{mark} $'\\E\\002b'")}
    finally:
        keep_preferences(page, kept)
        mac.close()


def finds_output(page: Page, name: str) -> Any:
    """The prefix and s find in a column's output, history included: it starts
    at the newest match and shows it, ↩ goes back to earlier ones, and Escape
    puts the search away and gives the column the keyboard back."""
    open_workspace(page, name)
    first = columns(page)[0]
    mark = str(time.time_ns())
    run_in_column(page, first, f"bash -c 'n=nee; for i in 1 2 3; do echo ${{n}}dle-{mark}-$i; seq 1 300; done; echo done-{mark}'", f"done-{mark}")
    prefix(page, "s")
    bar = page.get_by_role("search", name=re.compile(r"^Find in "))
    expect(bar).to_be_visible()
    bar.get_by_role("searchbox").fill(f"needle-{mark}")
    expect(bar).to_contain_text("3 of 3")

    def shown() -> bool:
        # The active match is scrolled into the terminal's view.
        return bool(column(page, first).locator(".wterm").evaluate(
            "(w) => { const m = w.querySelector('.term-search-active'); if (!m) return false;"
            " const a = m.getBoundingClientRect(), b = w.getBoundingClientRect(); return a.top >= b.top && a.bottom <= b.bottom; }"
        ))

    deadline = time.monotonic() + 5
    while not shown():
        assert time.monotonic() < deadline, "the newest match isn't shown"
        time.sleep(0.1)
    page.keyboard.press("Enter")
    expect(bar).to_contain_text("2 of 3")
    page.keyboard.press("Shift+Enter")
    expect(bar).to_contain_text("3 of 3")
    page.keyboard.press("Enter")
    page.keyboard.press("Enter")
    expect(bar).to_contain_text("1 of 3")
    deadline = time.monotonic() + 5
    while not shown():
        assert time.monotonic() < deadline, "the oldest match isn't shown"
        time.sleep(0.1)
    page.keyboard.press("Escape")
    expect(bar).to_have_count(0)
    expect(column(page, first).locator(".term-search-match, .term-search-active")).to_have_count(0)
    reaches(page, first)
    return {"column": first}


def copies_out(page: Page, name: str) -> Any:
    """A program's copy (OSC 52) reaches the page's clipboard, and a large
    one passes without stopping output: a 150 KB copy once wedged the
    terminal until a reload."""
    page.context.grant_permissions(["clipboard-read", "clipboard-write"], origin=CONSOLE)
    open_workspace(page, name)
    first = columns(page)[0]
    mark = str(time.time_ns())
    # base64 of "copied-<mark>", made by the shell, so typing it is plain ASCII.
    run_in_column(page, first, f"printf '\\e]52;c;%s\\a' $(printf copied-{mark} | base64); echo small-{mark}", f"small-{mark}")
    deadline = time.monotonic() + 10
    while page.evaluate("navigator.clipboard.readText()") != f"copied-{mark}":
        assert time.monotonic() < deadline, page.evaluate("navigator.clipboard.readText()")
        time.sleep(0.2)
    run_in_column(page, first, f"printf '\\e]52;c;%s\\a' $(head -c 150000 /dev/zero | tr '\\0' A); echo large-{mark}", f"large-{mark}")
    run_in_column(page, first, f"echo after-{mark}", f"after-{mark}")
    return {"column": first}


def lands_on_waiting(page: Page, name: str, waiting: str, other: str) -> Any:
    """Coming back to a workspace where an agent waits lands on that agent's
    column, whichever column you were in when you left."""
    open_workspace(page, name)
    column(page, other).locator(".term-host").click()
    expect(column(page, other)).to_have_class(re.compile(r"\bon\b"))
    page.get_by_role("link", name="iglu, overview").click()
    page.get_by_role("link", name=name, exact=True).click()
    # Until the columns' sessions load, the first column stands in as active;
    # judge once they have.
    for name_ in (waiting, other):
        expect(column(page, name_).locator(".term-host")).to_be_visible()
    time.sleep(1.5)
    active = [c for c in columns(page) if re.search(r"\bon\b", column(page, c).get_attribute("class") or "")]
    assert active == [waiting], {
        "active": active,
        "focus": page.evaluate("document.activeElement?.closest('[data-column]')?.dataset.column ?? document.activeElement?.tagName"),
        "headers": [column(page, c).locator(".col-h").inner_text() for c in columns(page)],
    }
    return {"active": waiting}


def palette_from_terminal(page: Page, name: str) -> Any:
    """The palette opened from inside a terminal takes what you type, rather
    than the terminal behind it."""
    open_workspace(page, name)
    first = columns(page)[0]
    column(page, first).locator(".term-host").click()
    before = screen(page).count("previews")
    prefix(page, "Slash")
    page.keyboard.type("previews")
    page.keyboard.press("Enter")
    page.wait_for_url(f"{CONSOLE}/previews")
    open_workspace(page, name)
    column(page, first).locator(".term-host").click()
    assert screen(page).count("previews") == before, screen(page)
    return {"went": "/previews"}


def palette_ranks(page: Page, name: str) -> Any:
    """The palette puts what's named exactly first, finds Settings, and keeps
    a search that found nothing so it can be fixed."""
    open_workspace(page, name)
    search = page.get_by_role("combobox", name="Search")

    def look_for(query: str) -> None:
        page.get_by_role("button", name="Search and commands").click()
        search.fill(query)

    look_for(name)
    expect(page.get_by_role("option", selected=True)).to_have_text(re.compile(f"^{re.escape(name)}"))
    page.keyboard.press("Enter")
    expect(search).to_have_count(0)
    expect(page.locator("form.rename")).to_have_count(0)
    assert page.url == f"{CONSOLE}/w/{name}", page.url
    look_for("zz-nothing-is-called-this")
    page.keyboard.press("Enter")
    expect(search).to_have_value("zz-nothing-is-called-this")
    search.fill("settings")
    page.keyboard.press("Enter")
    page.wait_for_url(f"{CONSOLE}/settings")
    return {"first": name}


def recording_cancels(page: Page) -> Any:
    """Giving up on recording a prefix takes its complaint with it."""
    page.goto(f"{CONSOLE}/settings")
    button = page.get_by_role("button", name=re.compile("^Prefix: "))
    button.click()
    page.keyboard.press("a")
    complaint = page.get_by_role("alert").filter(has_text="Use Ctrl with")
    expect(complaint).to_be_visible()
    page.keyboard.press("Escape")
    expect(complaint).to_have_count(0)
    expect(button).to_have_attribute("aria-pressed", "false")
    return {"cancelled": True}


def keys_stay(page: Page, name: str) -> Any:
    """A terminal keeps the chords shells use: Alt+B moves back a word and
    Ctrl+K kills the rest of the line, where iglu once took both. Escape
    then Tab reaches the program too, as Claude Code takes it, where wterm
    let the Tab move focus out of the terminal. The terminal's input is the
    tab stop, where a selection had taken it out of the tab order, and the
    prefix and Tab leave it for the page."""
    open_workspace(page, name)
    first = columns(page)[0]
    at_prompt(page, first)
    mark = str(time.time_ns())
    page.keyboard.type(f"echo kept{mark} gone{mark}")
    page.keyboard.press("Alt+b")
    page.keyboard.press("Control+k")
    page.keyboard.press("Enter")
    shown = shows(page, f"kept{mark}", 2)
    assert f"gone{mark}" not in shown.replace("\n", ""), shown
    assert page.url == f"{CONSOLE}/w/{name}", page.url
    expect(page.get_by_role("dialog")).to_have_count(0)
    page.keyboard.type(f"bash -c 'printf ready-%s {mark}; IFS= read -rsn2 -t 20 k; printf \"\\nread-%s %q\\n\" {mark} \"$k\"'")
    page.keyboard.press("Enter")
    shows(page, f"ready-{mark}")
    page.keyboard.press("Escape")
    page.keyboard.press("Tab")
    shows(page, f"read-{mark} $'\\E\\t'")
    assert focused_column(page) == first, focused_column(page)
    # The terminal's input is the tab stop, named for its column, and says
    # how to leave; the prefix and Tab leave it for the page.
    term = column(page, first).locator(".wterm")
    assert term.get_attribute("tabindex") == "-1", term.get_attribute("tabindex")
    field = term.locator("textarea")
    assert field.get_attribute("tabindex") == "0", field.get_attribute("tabindex")
    assert (field.get_attribute("aria-label") or "").startswith("Terminal, "), field.get_attribute("aria-label")
    assert "then Tab leaves the terminal" in (field.get_attribute("aria-description") or ""), field.get_attribute("aria-description")
    assert "Escape, then Tab" not in (field.get_attribute("aria-description") or "")
    prefix(page, "Tab")
    assert focused_column(page) is None, page.evaluate("document.activeElement?.outerHTML.slice(0, 120)")
    page.keyboard.press("Shift+Tab")
    assert focused_column(page) == first, page.evaluate("document.activeElement?.outerHTML.slice(0, 120)")
    return {"column": first}


def prefix_moves(page: Page, name: str) -> Any:
    """From a terminal, the prefix and a key reach iglu: the next column
    comes wholly into view and takes the keyboard."""
    open_workspace(page, name)
    names = columns(page)
    assert len(names) >= 2, names
    at_prompt(page, names[0])
    waiting = page.get_by_role("status", name="iglu is waiting for a key")
    page.keyboard.press("Control+Space")
    expect(waiting).to_be_visible()
    page.keyboard.press("l")
    expect(waiting).to_be_hidden()
    expect(column(page, names[1])).to_have_class(re.compile(r"\bon\b"))
    assert wholly_shown(page, names[1]), page.evaluate("document.querySelector('.w-cols').scrollLeft")
    reaches(page, names[1])
    prefix(page, "h")
    expect(column(page, names[0])).to_have_class(re.compile(r"\bon\b"))
    reaches(page, names[0])
    return {"moved": [names[1], names[0]]}


def focus_returns(page: Page, name: str) -> Any:
    """Whatever opens over a workspace takes the keyboard while it's open,
    and gives it back to the column you were in once it closes; before,
    typing went to the page, where letters were shortcuts."""
    open_workspace(page, name)
    first = columns(page)[0]
    at_prompt(page, first)
    here = column(page, first)
    width = here.locator(".wbtn")
    opened = width.inner_text()

    def asks(key: str, takes: Locator) -> None:
        prefix(page, key)
        expect(takes).to_be_focused()
        page.keyboard.press("Escape")

    cases: dict[str, Callable[[], None]] = {
        "palette": lambda: asks("Slash", page.get_by_role("combobox", name="Search")),
        "shortcuts": lambda: asks("Shift+Slash", page.get_by_role("dialog", name="Keyboard shortcuts").get_by_role("button", name="Close")),
        "rename": lambda: asks("r", page.get_by_role("textbox", name="Workspace name")),
        "add a column": lambda: asks("c", page.get_by_role("group", name="Add a column").get_by_role("button", name="Shell")),
        "end the column": lambda: asks("x", page.get_by_role("button", name=f"End {first}", exact=True)),
        "width button": width.click,
        "details": lambda: (page.get_by_role("button", name="Details", exact=True).click(), page.get_by_role("button", name="Details", exact=True).click()),
        "column header": lambda: here.locator(".col-h b").click(),
    }
    for case, run in cases.items():
        run()
        try:
            reaches(page, first)
        except AssertionError as error:
            raise AssertionError(f"after {case}: {error}") from error
    # Leave the column as wide as it was.
    while width.inner_text() != opened:
        width.click()
    return {"cases": list(cases)}


def selects_in_place(page: Page, name: str) -> Any:
    """Selecting text in a column scrolled into view leaves the strip where it
    is; focusing the terminal's hidden input scrolled it back to the first
    column."""
    open_workspace(page, name)
    names = columns(page)
    assert len(names) >= 2, names
    at_prompt(page, names[0])
    prefix(page, "l")
    second = column(page, names[1])
    width = second.locator(".wbtn")
    opened = width.inner_text()
    # As wide as the strip, so showing it scrolls the first column away.
    while width.inner_text() != "1":
        width.click()
    strip = page.locator(".w-cols")
    expect(second).to_be_in_viewport(ratio=0.9)
    time.sleep(0.5)
    before = strip.evaluate("(s) => s.scrollLeft")
    assert before > 0, before
    strip.evaluate("(s) => { window.stripMoves = []; s.addEventListener('scroll', () => window.stripMoves.push(s.scrollLeft)); }")
    box = second.locator(".term-grid").bounding_box()
    assert box
    page.mouse.move(box["x"] + 40, box["y"] + 20)
    page.mouse.down()
    page.mouse.move(box["x"] + 200, box["y"] + 60, steps=8)
    page.mouse.up()
    time.sleep(0.5)
    moves = page.evaluate("window.stripMoves")
    while width.inner_text() != opened:
        width.click()
    assert moves == [], {"before": before, "moves": moves}
    return {"scrollLeft": before}


def dialogs_hold_focus(page: Page, name: str) -> Any:
    """A dialog keeps the keyboard while it's open: Tab never reaches the page
    behind it, Escape still closes it, and the terminal has the keyboard
    again after."""
    open_workspace(page, name)
    first = columns(page)[0]
    at_prompt(page, first)
    prefix(page, "Shift+Slash")
    sheet = page.get_by_role("dialog", name="Keyboard shortcuts")
    expect(sheet).to_be_visible()
    # Past its last control, Tab may go to the browser's own controls, which
    # leaves the page's body focused; never to the page behind the dialog.
    for _ in range(3):
        page.keyboard.press("Tab")
        inside = page.evaluate("document.activeElement === document.body || Boolean(document.activeElement?.closest('dialog'))")
        assert inside, page.evaluate("document.activeElement?.outerHTML.slice(0, 120)")
    page.keyboard.press("Escape")
    expect(sheet).to_have_count(0)
    reaches(page, first)
    return {"tabbed": 3}


def enter_presses_buttons(page: Page) -> Any:
    """On the overview, Enter on a focused button presses it, rather than
    opening the selected workspace."""
    page.goto(CONSOLE)
    button = page.get_by_role("button", name="New project", exact=True)
    button.focus()
    page.keyboard.press("Enter")
    dialog = page.get_by_role("dialog", name="New project")
    expect(dialog).to_be_visible()
    assert page.url.rstrip("/") == CONSOLE, page.url
    page.keyboard.press("Escape")
    expect(dialog).to_have_count(0)
    return {"opened": "New project"}


def prefix_cancels(page: Page, name: str) -> Any:
    """The prefix waits for the next key only: a click in between puts it
    away, so what's typed next goes where the click went."""
    open_workspace(page, name)
    at_prompt(page, columns(page)[0])
    page.keyboard.press("Control+Space")
    expect(page.get_by_role("status", name="iglu is waiting for a key")).to_be_visible()
    page.get_by_role("button", name="Search and commands").click()
    search = page.get_by_role("combobox", name="Search")
    expect(search).to_be_focused()
    page.keyboard.type("previews")
    expect(search).to_have_value("previews")
    assert page.url == f"{CONSOLE}/w/{name}", page.url
    page.keyboard.press("Escape")
    return {"typed": "previews"}


def names_column(page: Page, name: str) -> Any:
    """A column can be called what it's for: the name shows on it and its
    chip, stays after a reload, and an empty one gives back the session's.
    The keyboard comes back to the terminal after."""
    open_workspace(page, name)
    first = columns(page)[0]
    at_prompt(page, first)
    prefix(page, "Comma")
    field = page.get_by_role("textbox", name=f"Name of {first}")
    expect(field).to_be_focused()
    expect(field).to_have_value(first)
    field.fill("the tests")
    page.keyboard.press("Enter")
    header = column(page, first).locator(".col-h b")
    expect(header).to_have_text("the tests")
    expect(page.get_by_role("navigation", name="Columns").get_by_role("button", name="the tests")).to_be_visible()
    reaches(page, first)
    page.reload()
    expect(header).to_have_text("the tests")
    # Leaving the field keeps what's in it, as there's no Enter on a phone.
    header.dblclick()
    page.get_by_role("textbox", name="Name of the tests").fill("left behind")
    column(page, first).locator(".term-host").click()
    expect(header).to_have_text("left behind")
    header.dblclick()
    field = page.get_by_role("textbox", name="Name of left behind")
    expect(field).to_have_value("left behind")
    field.fill("")
    page.keyboard.press("Enter")
    expect(header).to_have_text(first)
    return {"column": first}


def zooms(page: Page, name: str) -> Any:
    """Zoom gives a column the page and puts it back as it was: the layout
    iglu keeps doesn't change, and the keyboard stays in the terminal."""
    open_workspace(page, name)
    first = columns(page)[0]
    at_prompt(page, first)
    width = column(page, first).get_by_role("button", name=re.compile("^Width of ")).inner_text()
    sidebar = page.get_by_role("navigation", name="Workspaces")
    prefix(page, "KeyZ")
    expect(column(page, first)).to_have_attribute("style", re.compile(r"--cw:\s*1\b"))
    expect(sidebar).to_be_hidden()
    expect(column(page, first).get_by_role("button", name=re.compile("^Unzoom "))).to_have_attribute("aria-pressed", "true")
    reaches(page, first)
    # The palette has the column's actions too.
    prefix(page, "Slash")
    page.get_by_role("combobox", name="Search").fill("unzoom")
    page.keyboard.press("Enter")
    expect(sidebar).to_be_visible()
    expect(column(page, first).get_by_role("button", name=re.compile("^Width of "))).to_have_text(width)
    reaches(page, first)
    return {"width": width}


def goes_back(page: Page, name: str, other: str | None = None) -> Any:
    """The prefix then ; goes back to the workspace you were in before, even
    by way of the overview; with none yet, it says so."""
    open_workspace(page, name)
    if other is None:
        prefix(page, "Semicolon")
        expect(page.get_by_text("There's no other workspace to go back to yet.")).to_be_visible()
        return {"back": None}
    page.get_by_role("link", name="iglu, overview").click()
    page.get_by_role("link", name=other, exact=True).first.click()
    page.wait_for_url(f"{CONSOLE}/w/{other}")
    prefix(page, "Semicolon")
    page.wait_for_url(f"{CONSOLE}/w/{name}")
    prefix(page, "Semicolon")
    page.wait_for_url(f"{CONSOLE}/w/{other}")
    return {"back": other}


def drags_column(page: Page, name: str) -> Any:
    """Dragging a column's chip onto another's left half puts it before that
    one, and the order is what iglu keeps."""
    open_workspace(page, name)
    before = columns(page)
    assert len(before) >= 2, before
    chips = page.get_by_role("navigation", name="Columns").get_by_role("button")

    def drag(source: int, target: int) -> None:
        box = chips.nth(target).bounding_box()
        assert box, target
        chips.nth(source).drag_to(chips.nth(target), target_position={"x": 3, "y": box["height"] / 2})

    drag(1, 0)
    swapped = [before[1], before[0], *before[2:]]
    expect(page.locator("section[data-column]").first).to_have_attribute("data-column", before[1])
    page.reload()
    expect(page.locator("section[data-column]").first).to_have_attribute("data-column", before[1])
    assert columns(page) == swapped, columns(page)
    drag(1, 0)
    expect(page.locator("section[data-column]").first).to_have_attribute("data-column", before[0])
    return {"swapped": swapped}


def phone_keys(page: Page, name: str) -> Any:
    """On a phone, the key row sends what the on-screen keyboard lacks, to
    the column with the keyboard and without taking it: ^C stops a command,
    and Ctrl goes with the next letter typed."""
    browser = page.context.browser
    assert browser
    phone = browser.new_context(
        storage_state=page.context.storage_state(), viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True
    )
    try:
        tap = phone.new_page()
        open_workspace(tap, name)
        first = columns(tap)[0]
        at_prompt(tap, first)
        keys = tap.get_by_role("toolbar", name="Terminal keys")
        tap.keyboard.type("sleep 300")
        tap.keyboard.press("Enter")
        time.sleep(1)
        keys.get_by_role("button", name="Interrupt (Ctrl+C)").tap()
        reaches(tap, first)
        # Ctrl then u clears what's typed so far, in bash and fish alike.
        mark = str(time.time_ns())
        tap.keyboard.type(f"echo gone{mark}")
        ctrl = keys.get_by_role("button", name="Ctrl, for the next key")
        ctrl.tap()
        expect(ctrl).to_have_attribute("aria-pressed", "true")
        tap.keyboard.type("u")
        expect(ctrl).to_have_attribute("aria-pressed", "false")
        tap.keyboard.type(f"echo kept{mark}")
        tap.keyboard.press("Enter")
        shows(tap, f"kept{mark}", 2)
        assert f"gone{mark}echo" not in screen(tap).replace("\n", ""), screen(tap)
        # Arrows go as the program asks, here application cursor keys, and
        # bring a terminal scrolled back into its history to the live screen.
        mark = str(time.time_ns())
        run_in_column(tap, first, "printf '\\e[?1h'; seq 1 300; " + RAW_READ.format(mark=mark) + "; printf '\\e[?1l'", f"ready-{mark}")
        term = column(tap, first).locator(".wterm")
        term.evaluate("(w) => (w.scrollTop = 0)")
        keys.get_by_role("button", name="Left").tap()
        shows(tap, f"read-{mark} $'\\EOD'")
        assert term.evaluate("(w) => w.scrollHeight - w.scrollTop - w.clientHeight < 5"), "still in the history"
        return {"column": first}
    finally:
        phone.close()


def acts_where_shown(page: Page, name: str) -> Any:
    """A column acted on, from its header or the palette, takes the keyboard:
    what's typed next goes to the column shown, and a question it asks has
    the keyboard until it's answered."""
    open_workspace(page, name)
    first, second = columns(page)[:2]
    at_prompt(page, first)
    column(page, second).locator(".col-h").hover()
    column(page, second).get_by_role("button", name=re.compile("^Zoom ")).click()
    reaches(page, second)
    column(page, second).get_by_role("button", name=re.compile("^Unzoom ")).click()
    search = page.get_by_role("combobox", name="Search")
    prefix(page, "Slash")
    search.fill(f"go to column {first}")
    page.keyboard.press("Enter")
    reaches(page, first)
    prefix(page, "Slash")
    search.fill(f"end {first}")
    page.keyboard.press("Enter")
    expect(page.get_by_role("button", name=f"End {first}", exact=True)).to_be_focused()
    page.keyboard.press("Escape")
    reaches(page, first)
    return {"columns": [first, second]}


def renames_follow(page: Page, name: str) -> Any:
    """Renaming keeps the same workspace open: its column keeps the keyboard,
    and another tab showing it follows to the new name, terminals and all."""
    other = page.context.new_page()
    open_workspace(other, name)
    open_workspace(page, name)
    names = columns(page)
    assert len(names) >= 2, names
    at_prompt(page, names[0])
    prefix(page, "l")
    expect(column(page, names[1])).to_have_class(re.compile(r"\bon\b"))
    renamed = f"{name}-renamed"

    def rename(old: str, new: str) -> None:
        page.get_by_role("button", name=old, exact=True).click()
        field = page.get_by_role("textbox", name="Workspace name")
        field.fill(new)
        field.press("Enter")
        page.wait_for_url(f"{CONSOLE}/w/{new}")

    rename(name, renamed)
    reaches(page, names[1])
    other.wait_for_url(f"{CONSOLE}/w/{renamed}")
    expect(other.locator("[data-column]")).to_have_count(len(names))
    rename(renamed, name)
    reaches(page, names[1])
    other.wait_for_url(f"{CONSOLE}/w/{name}")
    other.close()
    return {"renamed": renamed}


def drafts_survive(page: Page, project: str) -> Any:
    """An edit in progress on a project's settings survives a save made in
    another tab, which the form says, rather than vanishing; an untouched
    form takes the saved settings."""
    other = page.context.new_page()
    for p in (page, other):
        p.goto(f"{CONSOLE}/p/{project}")
    form = page.get_by_role("form", name=f"Settings for {project}")
    theirs = other.get_by_role("form", name=f"Settings for {project}")
    idle = theirs.get_by_label("When unused")
    before = idle.input_value()
    previews = form.get_by_label("Previews")
    previews.fill("18480")
    idle.select_option("never" if before != "never" else "default")
    theirs.get_by_role("button", name="Save").click()
    notice = form.get_by_role("status")
    expect(notice).to_contain_text("saved elsewhere")
    expect(previews).to_have_value("18480")
    notice.get_by_role("button", name="Show the saved settings").click()
    expect(form.get_by_label("When unused")).to_have_value(idle.input_value())
    # Put it back, from the tab that changed it; this one, untouched, follows.
    idle.select_option(before)
    theirs.get_by_role("button", name="Save").click()
    expect(form.get_by_label("When unused")).to_have_value(before)
    other.close()
    return {"kept": "18480"}


def adds_at_once(page: Page, name: str) -> Any:
    """Two columns asked for at once both open, under different names; they
    once picked the same free name, and one failed."""
    open_workspace(page, name)
    added = page.evaluate(
        """async (name) => {
          const me = await (await fetch('/v1/me')).json();
          const ws = (await (await fetch('/v1/workspaces')).json()).find((w) => w.name === name);
          const headers = { 'content-type': 'application/json', 'x-csrf-token': me.csrf_token };
          const add = () => fetch(`/v1/workspaces/${ws.id}/columns`, { method: 'POST', headers, body: JSON.stringify({ kind: { kind: 'shell' } }) });
          const answers = await Promise.all([add(), add()]);
          const made = await Promise.all(answers.map(async (a) => (a.ok ? (await a.json()).name : `${a.status} ${await a.text()}`)));
          for (const a of answers.keys()) if (answers[a].ok) await fetch(`/v1/workspaces/${ws.id}/columns/${made[a]}`, { method: 'DELETE', headers });
          return { statuses: answers.map((a) => a.status), made };
        }""",
        name,
    )
    assert added["statuses"] == [201, 201] and len(set(added["made"])) == 2, added
    return added


def questions_end(page: Page, name: str) -> Any:
    """A question ends with the visit that asked it: a Delete armed and left
    unanswered is gone on coming back, so no later click can answer it."""
    open_workspace(page, name)
    details = page.get_by_role("button", name="Details", exact=True)
    panel = page.get_by_role("complementary", name="Details")
    details.click()
    panel.get_by_role("button", name="Delete", exact=True).click()
    expect(panel.get_by_text(f"Delete {name}?")).to_be_visible()
    page.get_by_role("link", name="iglu, overview").click()
    page.wait_for_url(f"{CONSOLE}/")
    page.go_back()
    page.wait_for_url(f"{CONSOLE}/w/{name}")
    expect(page.locator('.wsv[data-phase="running"]')).to_be_visible()
    expect(panel).to_have_count(0)
    details.click()
    expect(panel.get_by_role("button", name="Delete", exact=True)).to_be_visible()
    expect(panel.get_by_text(f"Delete {name}?")).to_have_count(0)
    details.click()
    return {"asked": False}


def card(page: Page, name: str, expected: str) -> Any:
    """Waits for a workspace's overview card to show `expected`, without opening it."""
    page.goto(CONSOLE)
    entry = page.locator(f'.card[data-name="{name}"]')
    expect(entry).to_contain_text(expected, timeout=120_000)
    return {"text": entry.inner_text(), "class": entry.get_attribute("class")}


def publish(page: Page, name: str, port: str) -> Any:
    open_workspace(page, name)
    page.get_by_role("button", name="Publish a port").click()
    page.get_by_label("Port to publish").fill(port)
    page.get_by_role("button", name="Publish", exact=True).click()
    link = page.locator(".w-ports").get_by_role("link").filter(has_text=f":{port}")
    url = link.get_attribute("href")
    with page.context.expect_page() as opened:
        link.click()
    preview = opened.value
    preview.wait_for_load_state()
    expect(preview.locator("body")).to_have_text("hello", timeout=60_000)
    return {"url": url, "landed": preview.url}


def visit(page: Page, url: str) -> Any:
    """Opens a preview and waits for the app, through any 'Thawing…' page."""
    page.goto(url)
    expect(page.locator("body")).to_have_text("hello", timeout=120_000)
    return {"url": page.url}


def attack(page: Page, preview: str, workspace: str) -> Any:
    """What a hostile preview page can try against the console, with the
    person's cookies in the same browser."""
    page.goto(preview)
    expect(page.locator("body")).to_have_text("hello")
    return page.evaluate(
        """async ([console, workspace]) => {
            const result = {};
            try {
                const response = await fetch(`${console}/v1/workspaces`, { credentials: "include" });
                result.read = `status ${response.status}`;
            } catch {
                result.read = "blocked";
            }
            // A form-like request the browser sends without a preflight.
            await fetch(`${console}/v1/workspaces`, {
                method: "POST",
                mode: "no-cors",
                credentials: "include",
                headers: { "content-type": "text/plain" },
                body: JSON.stringify({ environment: "example", repo: "https://example.org/x.git", name: "forged" }),
            }).catch(() => {});
            const url = new URL(`/v1/workspaces/${workspace}/columns/forged/attach?cols=80&rows=24`, console);
            url.protocol = "wss:";
            result.socket = await new Promise((resolve) => {
                const socket = new WebSocket(url);
                socket.onopen = () => { resolve("open"); socket.close(); };
                socket.onerror = () => resolve("refused");
                setTimeout(() => resolve("timeout"), 15000);
            });
            return result;
        }""",
        [CONSOLE, workspace],
    )


STEPS: dict[str, Callable[..., Any]] = {
    "sign-in": sign_in,
    "signed-out": signed_out,
    "approve-cli": approve_cli,
    "create": create,
    "refused-environment": refused_environment,
    "project-agent": project_agent,
    "terminal": terminal,
    "new-column": new_column,
    "answers-queries": answers_queries,
    "draws-blocks": draws_blocks,
    "wheels-pager": wheels_pager,
    "copies-out": copies_out,
    "finds-output": finds_output,
    "inserts-text": inserts_text,
    "opens-links": opens_links,
    "copies-history": copies_history,
    "mac-keys": mac_keys,
    "selected-keys": selected_keys,
    "styles-terminal": styles_terminal,
    "palette-from-terminal": palette_from_terminal,
    "palette-ranks": palette_ranks,
    "recording-cancels": recording_cancels,
    "keys-stay": keys_stay,
    "prefix-moves": prefix_moves,
    "focus-returns": focus_returns,
    "selects-in-place": selects_in_place,
    "dialogs-hold-focus": dialogs_hold_focus,
    "enter-presses-buttons": enter_presses_buttons,
    "prefix-cancels": prefix_cancels,
    "names-column": names_column,
    "zooms": zooms,
    "acts-where-shown": acts_where_shown,
    "drags-column": drags_column,
    "phone-keys": phone_keys,
    "goes-back": goes_back,
    "renames-follow": renames_follow,
    "drafts-survive": drafts_survive,
    "adds-at-once": adds_at_once,
    "questions-end": questions_end,
    "card": card,
    "lands-on-waiting": lands_on_waiting,
    "publish": publish,
    "visit": visit,
    "attack": attack,
}


def main() -> None:
    step, args = sys.argv[1], sys.argv[2:]
    with sync_playwright() as playwright:
        rules = os.environ.get("BROWSER_RESOLVER_RULES")
        browser = playwright.chromium.launch(args=[f"--host-resolver-rules={rules}"] if rules else [])
        context = browser.new_context(storage_state=STATE if STATE.exists() else None)
        context.set_default_timeout(30_000)
        page = context.new_page()
        try:
            result = STEPS[step](page, *args)
        except Exception:
            SHOTS.mkdir(parents=True, exist_ok=True)
            for index, open_page in enumerate(context.pages):
                open_page.screenshot(path=SHOTS / f"{step}-{index}.png")
            raise
        finally:
            context.storage_state(path=STATE)
            browser.close()
    print(json.dumps(result))


if __name__ == "__main__":
    main()
