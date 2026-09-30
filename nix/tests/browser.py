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

from playwright.sync_api import Page, expect, sync_playwright

CONSOLE = "https://iglu.example.test"
STATE = Path("/root/browser-state.json")
SHOTS = Path("/tmp/browser")


def open_workspace(page: Page, name: str) -> None:
    page.goto(CONSOLE)
    page.locator(".row", has=page.locator(".name", has_text=name)).click()
    expect(page.locator("#workspace-header .phase")).to_have_text("running", timeout=600_000)


def screen(page: Page) -> str:
    return str(page.evaluate("globalThis.iglu.screen()"))


def run_in_terminal(page: Page, command: str, expected: str) -> str:
    """Types into the active terminal, as a person would, and waits for output."""
    deadline = time.monotonic() + 60
    while "$" not in screen(page):  # the shell's prompt
        assert time.monotonic() < deadline, f"no prompt: {screen(page)!r}"
        time.sleep(0.5)
    page.locator("#terminal").click()
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
    expect(page.locator("#sidebar .who")).to_have_text("alice@example.org")
    return {"url": page.url}


def approve_cli(page: Page, url: str) -> Any:
    page.goto(url)
    page.get_by_role("button", name="Sign in the CLI").click()
    page.wait_for_url("http://127.0.0.1:*/**")
    expect(page.locator("body")).to_contain_text("Signed in.")
    return {}


def create(page: Page, environment: str, repo: str, name: str) -> Any:
    page.goto(CONSOLE)
    page.get_by_role("button", name="+ New workspace").click()
    dialog = page.locator("dialog#create")
    dialog.locator("select[name=environment]").select_option(environment)
    dialog.locator("input[name=repo]").fill(repo)
    dialog.locator("input[name=name]").fill(name)
    dialog.get_by_role("button", name="Create").click()
    expect(page.locator("#workspace-header h1")).to_have_text(name)
    expect(page.locator("#workspace-header .phase")).to_have_text("running", timeout=600_000)
    return {"id": page.url.rsplit("/", 1)[1]}


def terminal(page: Page, name: str, command: str, expected: str) -> Any:
    open_workspace(page, name)
    return {"screen": run_in_terminal(page, command, expected)}


def new_tab(page: Page, name: str, command: str, expected: str) -> Any:
    open_workspace(page, name)
    tabs = page.locator("#tabs .tab:not(.add)")
    expect(tabs).not_to_have_count(0)
    before = tabs.count()
    page.locator("#tabs .tab.add").click()
    expect(tabs).to_have_count(before + 1)
    output = run_in_terminal(page, command, expected)
    return {"tabs": tabs.all_inner_texts(), "screen": output}


def row(page: Page, name: str, expected: str) -> Any:
    """Waits for a workspace's sidebar row to show `expected`, without selecting it."""
    page.goto(CONSOLE)
    entry = page.locator(".row", has=page.locator(".name", has_text=name))
    expect(entry).to_contain_text(expected, timeout=120_000)
    return {"text": entry.inner_text(), "class": entry.get_attribute("class")}


def publish(page: Page, name: str, port: str) -> Any:
    open_workspace(page, name)
    page.locator("#ports input").fill(port)
    page.get_by_role("button", name="Publish").click()
    link = page.locator("#ports a", has_text=f":{port}")
    url = link.get_attribute("href")
    with page.context.expect_page() as opened:
        link.click()
    preview = opened.value
    preview.wait_for_load_state()
    expect(preview.locator("body")).to_have_text("hello", timeout=60_000)
    return {"url": url, "landed": preview.url}


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
            const url = new URL(`/v1/workspaces/${workspace}/terminals/forged/attach?cols=80&rows=24`, console);
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
    "new-tab": new_tab,
    "row": row,
    "publish": publish,
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
