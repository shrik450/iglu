"""Drives Chromium through the console the way a person would.

Each run performs one step and prints its result as JSON. Cookies, including
session cookies, carry over between runs in a state file, so steps build on
each other like one long browser session. A failing step leaves a screenshot.

    browser <step> [args...]
"""

import json
import os
import sys
import time
from pathlib import Path
from typing import Any, Callable

from playwright.sync_api import Locator, Page, expect, sync_playwright

CONSOLE = "https://iglu.example.test"
STATE = Path("/root/browser-state.json")
SHOTS = Path("/tmp/browser")


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


def run_in_column(page: Page, name: str, command: str, expected: str) -> str:
    """Types into a column's terminal, as a person would, and waits for output."""
    page.wait_for_selector("[data-column]")
    column(page, name).locator(".term-host").click()
    deadline = time.monotonic() + 60
    while "$" not in screen(page):  # the shell's prompt
        assert time.monotonic() < deadline, f"no prompt: {screen(page)!r}"
        time.sleep(0.5)
    page.keyboard.type(command)
    page.keyboard.press("Enter")
    while expected not in screen(page):
        assert time.monotonic() < deadline, f"{expected!r} never appeared: {screen(page)!r}"
        time.sleep(0.5)
    return screen(page)


def sign_in(page: Page) -> Any:
    page.goto(CONSOLE)
    page.locator("#username-textfield").fill("alice")
    page.locator("#password-textfield").fill("password")
    page.locator("#sign-in-button").click()
    page.wait_for_url(f"{CONSOLE}/**")
    expect(page.get_by_role("link", name="Settings, signed in as alice@example.org")).to_be_visible()
    return {"url": page.url}


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
    expect(form.get_by_label("Project").locator("option:checked")).to_have_text("app")
    form.get_by_label("Name").fill(name)
    form.get_by_role("button", name="Create").click()
    page.wait_for_url(f"{CONSOLE}/w/{name}")
    expect(page.get_by_role("heading", name=name)).to_be_visible()
    expect(page.locator('.wsv[data-phase="running"]')).to_be_visible(timeout=600_000)
    return {"name": name, "columns": columns(page)}


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
    "approve-cli": approve_cli,
    "create": create,
    "terminal": terminal,
    "new-column": new_column,
    "card": card,
    "publish": publish,
    "visit": visit,
    "attack": attack,
}


def main() -> None:
    step, args = sys.argv[1], sys.argv[2:]
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(
            args=[f"--host-resolver-rules={os.environ['BROWSER_RESOLVER_RULES']}"]
        )
        context = browser.new_context(storage_state=STATE if STATE.exists() else None)
        context.set_default_timeout(30_000)
        page = context.new_page()
        try:
            result = STEPS[step](page, *args)
        except Exception:
            SHOTS.mkdir(exist_ok=True)
            for index, open_page in enumerate(context.pages):
                open_page.screenshot(path=SHOTS / f"{step}-{index}.png")
            raise
        finally:
            context.storage_state(path=STATE)
            browser.close()
    print(json.dumps(result))


if __name__ == "__main__":
    main()
