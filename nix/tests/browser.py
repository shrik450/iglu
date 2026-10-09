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

import json
import os
import re
import sys
import time
from pathlib import Path
from typing import Any, Callable

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
    return page.get_by_role("region", name=f"Column {name}", exact=True)


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
# fish does after every command, and prints the answers without their ESC
# after a mark, so an earlier run's answers on screen don't count.
QUERIES = (
    "bash -c 'printf \"\\e[c\" >/dev/tty; IFS= read -rs -t 5 -d c d </dev/tty; "
    "printf \"\\e]11;?\\a\" >/dev/tty; IFS= read -rs -t 5 -d \"$(printf \"\\a\")\" o </dev/tty; "
    "printf \"answers-%s %s %s\\n\" {mark} \"${{d#?}}\" \"${{o#?}}\"'"
)


def answers_queries(page: Page, name: str) -> Any:
    """The terminal answers the queries shells wait on; fish held every
    keystroke for ten seconds after each command when it didn't."""
    open_workspace(page, name)
    first = columns(page)[0]
    mark = str(time.time_ns())
    shown = run_in_column(page, first, QUERIES.format(mark=mark), f"answers-{mark} [?62;22 ]11;rgb:")
    answers = re.search(rf"answers-{mark} (\S+ \]11;rgb:[0-9a-f/]+)", shown.replace("\n", ""))
    return {"column": first, "answers": answers and answers[1]}


def lands_on_waiting(page: Page, name: str, waiting: str, other: str) -> Any:
    """Coming back to a workspace where an agent waits lands on that agent's
    column, whichever column you were in when you left."""
    open_workspace(page, name)
    column(page, other).locator(".term-host").click()
    expect(column(page, other)).to_have_class(re.compile(r"\bon\b"))
    page.get_by_role("button", name="Overview").click()
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
    box = second.locator("canvas").first.bounding_box()
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


def questions_end(page: Page, name: str) -> Any:
    """A question ends with the visit that asked it: a Delete armed and left
    unanswered is gone on coming back, so no later click can answer it."""
    open_workspace(page, name)
    details = page.get_by_role("button", name="Details", exact=True)
    panel = page.get_by_role("complementary", name="Details")
    details.click()
    panel.get_by_role("button", name="Delete", exact=True).click()
    expect(panel.get_by_text(f"Delete {name}?")).to_be_visible()
    page.get_by_role("group", name="View").get_by_role("button", name="Overview").click()
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
    "palette-from-terminal": palette_from_terminal,
    "keys-stay": keys_stay,
    "prefix-moves": prefix_moves,
    "focus-returns": focus_returns,
    "selects-in-place": selects_in_place,
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
