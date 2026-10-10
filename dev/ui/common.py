"""What the dev stack's UI tools share: signing in, the API, and terminals.

The tools drive the local dev stack (`just dev`) as a person would, through
real browsers from the dev shell, and its API with that person's session.
"""

import json
import os
import subprocess
import time
import uuid
from pathlib import Path
from typing import Any

from playwright.sync_api import BrowserContext, Page, Playwright

CONSOLE = "https://iglu.localhost"
PASSWORD = "password"
ROOT = Path(__file__).resolve().parents[2]


def shared() -> Path:
    """The dev stack's shared directory: the dev CA and fixture repositories,
    in the main checkout, as the justfile's `dev_shared`."""
    common = subprocess.run(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout.strip()
    return Path(common).parent / ".dev/shared"


# Browsers trust the dev CA through the Keychain; Playwright's API requests
# go through Node, which needs to be told.
os.environ.setdefault("NODE_EXTRA_CA_CERTS", str(shared() / "ca/ca.crt"))


def launch(playwright: Playwright, engine: str = "chromium", **context: Any) -> BrowserContext:
    """A fresh browser and context. Close the context's browser when done."""
    browser = getattr(playwright, engine).launch()
    made = browser.new_context(**context)
    made.set_default_timeout(30_000)
    return made


def sign_in(page: Page, user: str) -> None:
    """Signs in through Authelia's form, as a person would."""
    page.goto(CONSOLE)
    # The console's page loads before it learns there's no session, then
    # sends the browser to sign in; wait for whichever it settles on.
    form = page.locator("#username-textfield")
    signed_in = page.get_by_role("link", name=f"Settings, signed in as {user}@example.org")
    form.or_(signed_in).wait_for()
    if signed_in.is_visible():
        return
    form.fill(user)
    page.locator("#password-textfield").fill(PASSWORD)
    page.locator("#sign-in-button").click()
    page.wait_for_url(f"{CONSOLE}/**")


class Api:
    """iglud's API with a signed-in person's session, sending what the
    console sends with every mutating request."""

    def __init__(self, page: Page) -> None:
        self.request = page.context.request
        me = self.get("/v1/me")
        self.me = me
        self.headers = {
            "origin": CONSOLE,
            "sec-fetch-site": "same-origin",
            "x-csrf-token": me["csrf_token"],
        }

    def _check(self, method: str, path: str, response: Any) -> Any:
        if not response.ok:
            raise RuntimeError(f"{method} {path}: {response.status} {response.text()}")
        return response.json() if response.status != 204 else None

    def get(self, path: str) -> Any:
        return self._check("GET", path, self.request.get(f"{CONSOLE}{path}"))

    def send(self, method: str, path: str, body: Any = None, **headers: str) -> Any:
        response = self.request.fetch(
            f"{CONSOLE}{path}",
            method=method,
            headers={**self.headers, "content-type": "application/json", **headers},
            data=json.dumps(body) if body is not None else None,
        )
        return self._check(method, path, response)

    def workspace(self, name: str) -> dict[str, Any]:
        return next(ws for ws in self.get("/v1/workspaces") if ws["name"] == name)

    def create(self, **request: Any) -> dict[str, Any]:
        return self.send("POST", "/v1/workspaces", request, **{"idempotency-key": str(uuid.uuid4())})

    def environment(self, name: str, source: str, timeout: float = 120) -> None:
        """Adds an environment unless it exists, and waits until it's built."""
        if not any(env["name"] == name for env in self.get("/v1/environments")):
            self.send("POST", "/v1/environments", {"name": name, "source": source})
        deadline = time.monotonic() + timeout
        while True:
            env = next(env for env in self.get("/v1/environments") if env["name"] == name)
            status = (env.get("latest") or {}).get("status")
            if status == "ready":
                return
            if status == "failed" or time.monotonic() > deadline:
                raise RuntimeError(f"environment {name} is {status}")
            time.sleep(1)

    def wait(self, name: str, phase: str, timeout: float = 180) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        while True:
            ws = self.workspace(name)
            if ws["phase"] == phase:
                return ws
            if time.monotonic() > deadline:
                raise TimeoutError(f"{name} is {ws['phase']}, not {phase}")
            time.sleep(1)


def screen(page: Page) -> str:
    """The focused terminal's visible text, as the core holds it."""
    return str(page.evaluate("globalThis.iglu.screen()"))


def type_in(page: Page, column: str, text: str, expected: str, timeout: float = 60) -> str:
    """Types a line into a column's terminal and waits for `expected`."""
    page.get_by_role("region", name=f"Column {column}", exact=True).locator(".term-host").click()
    deadline = time.monotonic() + timeout
    shown = ""
    while not shown.strip() or shown != screen(page):
        if time.monotonic() > deadline:
            raise TimeoutError(f"{column} never settled: {screen(page)!r}")
        shown = screen(page)
        time.sleep(1)
    page.keyboard.type(text)
    page.keyboard.press("Enter")
    while expected not in screen(page):
        if time.monotonic() > deadline:
            raise TimeoutError(f"{expected!r} never appeared in {column}: {screen(page)!r}")
        time.sleep(0.5)
    return screen(page)
