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
    page.keyboard.type(f"echo here{mark}")
    page.keyboard.press("Enter")
    # Once in the command, once printed.
    shows(page, f"here{mark}", 2)


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
# rows, and a box cross in green, written as octal UTF-8 so typing them is
# plain ASCII in any shell.
BLOCKS = (
    "clear; printf '\\e[38;2;255;0;0m\\342\\226\\227\\342\\226\\226\\n\\342\\226\\235\\342\\226\\230 "
    "\\e[38;2;0;255;0m\\342\\225\\266\\342\\224\\200\\342\\224\\274\\342\\224\\200\\342\\225\\264\\e[0m\\nblocks-%s\\n' {mark}"
)


def draws_blocks(page: Page, name: str) -> Any:
    """Block glyphs that meet across cells meet without a seam, and box
    strokes are whole pixels: wterm split quadrants at 8.5px, so Claude
    Code's logo showed a faint line under each eye and arm, and drew 1px
    strokes across two pixels at half strength."""
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
    Ctrl+K kills the rest of the line, where iglu once took both."""
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
