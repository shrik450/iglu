"""Runs the VM test's browser steps against the local dev stack.

    python3 dev/ui/check.py

The steps are nix/tests/browser.py's, so a fix to the console gets one test
that runs here in about a minute and in the VM test (`just e2e`) against a
real host. It signs in as alice, adds a project for a fixture repository,
and drives a workspace through terminals, attention, a published preview and
a hostile preview page. It deletes what it made, so it can run again; run it
with `just dev` up.
"""

import os
import sys
import time
from pathlib import Path

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))

from common import CONSOLE, ROOT, Api, launch  # noqa: E402

os.environ["IGLU_CONSOLE"] = CONSOLE
sys.path.insert(0, str(ROOT / "nix/tests"))

import browser  # noqa: E402
from playwright.sync_api import sync_playwright  # noqa: E402
from seed import fixture  # noqa: E402

# The project is named after its repository, check; the workspace's name
# differs, so links to each are unambiguous.
PROJECT = "check"
NAME = "checking"
PORT = 18432


def step(name: str, run, *args):
    started = time.monotonic()
    result = run(*args)
    print(f"ok {name} ({time.monotonic() - started:.1f}s)", flush=True)
    return result


def main() -> None:
    repo = fixture(PROJECT, [{"index.html": "hello\n", "README.md": "# check\n"}])
    with sync_playwright() as playwright:
        context = launch(playwright, viewport={"width": 1440, "height": 900})
        page = context.new_page()
        try:
            step("sign-in", browser.sign_in, page)
            api = Api(page)
            api.environment("default", "github:you/iglu-env#workspace")
            # What an earlier run left: the project and its workspaces.
            for project in api.get("/v1/projects"):
                if project["name"] != PROJECT:
                    continue
                inside = lambda: [ws for ws in api.get("/v1/workspaces") if ws["project"] == project["id"]]  # noqa: E731
                for ws in inside():
                    api.send("PUT", f"/v1/workspaces/{ws['id']}/desired-state", {"state": "deleted", "expected_revision": ws["revision"]})
                deadline = time.monotonic() + 120
                while inside():
                    if time.monotonic() > deadline:
                        raise TimeoutError(f"an earlier run's workspaces in {PROJECT} weren't deleted")
                    time.sleep(1)
                api.send("DELETE", f"/v1/projects/{project['id']}")

            step("signed-out", browser.signed_out, page, "/settings")
            step("refused-environment", browser.refused_environment, page)
            step("create", browser.create, page, "default", repo, NAME)
            step("project-agent", browser.project_agent, page, PROJECT, "scripted")
            step("terminal", browser.terminal, page, NAME, "printf 'one-%s\\n' 42", "one-42")
            step("answers-queries", browser.answers_queries, page, NAME)
            step("draws-blocks", browser.draws_blocks, page, NAME)
            step("wheels-pager", browser.wheels_pager, page, NAME)
            step("copies-out", browser.copies_out, page, NAME)
            step("finds-output", browser.finds_output, page, NAME)
            step("inserts-text", browser.inserts_text, page, NAME)
            step("opens-links", browser.opens_links, page, NAME)
            step("copies-history", browser.copies_history, page, NAME)
            step("mac-keys", browser.mac_keys, page, NAME)
            step("selected-keys", browser.selected_keys, page, NAME)
            step("styles-terminal", browser.styles_terminal, page, NAME)
            added = step("new-column", browser.new_column, page, NAME, "printf 'two-%s\\n' 42", "two-42")["added"]
            step("attention", browser.terminal, page, NAME, "iglu-status set waiting Check needs you; printf 'set-%s\\n' 42", "set-42")
            step("palette-from-terminal", browser.palette_from_terminal, page, NAME)
            step("palette-ranks", browser.palette_ranks, page, NAME)
            step("recording-cancels", browser.recording_cancels, page)
            step("keys-stay", browser.keys_stay, page, NAME)
            step("prefix-moves", browser.prefix_moves, page, NAME)
            step("focus-returns", browser.focus_returns, page, NAME)
            step("selects-in-place", browser.selects_in_place, page, NAME)
            step("dialogs-hold-focus", browser.dialogs_hold_focus, page, NAME)
            step("enter-presses-buttons", browser.enter_presses_buttons, page)
            step("prefix-cancels", browser.prefix_cancels, page, NAME)
            step("names-column", browser.names_column, page, NAME)
            step("zooms", browser.zooms, page, NAME)
            step("acts-where-shown", browser.acts_where_shown, page, NAME)
            step("drags-column", browser.drags_column, page, NAME)
            step("phone-keys", browser.phone_keys, page, NAME)
            step("goes-back", browser.goes_back, page, NAME)
            step("renames-follow", browser.renames_follow, page, NAME)
            step("drafts-survive", browser.drafts_survive, page, PROJECT)
            step("adds-at-once", browser.adds_at_once, page, NAME)
            step("questions-end", browser.questions_end, page, NAME)
            step("card", browser.card, page, NAME, "Check needs you")
            step("lands-on-waiting", browser.lands_on_waiting, page, NAME, "shell", added)
            step("server", browser.terminal, page, NAME, f"python3 -m http.server {PORT} --bind 127.0.0.1 &", f"port {PORT}")
            published = step("publish", browser.publish, page, NAME, str(PORT))
            ws = api.workspace(NAME)
            result = step("attack", browser.attack, page, published["url"], ws["id"])
            assert result == {"read": "blocked", "socket": "refused"}, result
            step("visit", browser.visit, page, published["url"])
            # Leave nothing behind for a tour to show.
            api.send("PUT", f"/v1/workspaces/{ws['id']}/desired-state", {"state": "deleted", "expected_revision": api.workspace(NAME)["revision"]})
            project = next(p for p in api.get("/v1/projects") if p["name"] == PROJECT)
            deadline = time.monotonic() + 120
            while any(w["project"] == project["id"] for w in api.get("/v1/workspaces")):
                if time.monotonic() > deadline:
                    raise TimeoutError(f"{NAME} wasn't deleted")
                time.sleep(1)
            api.send("DELETE", f"/v1/projects/{project['id']}")
        except Exception:
            shots = ROOT / ".dev/check"
            shots.mkdir(parents=True, exist_ok=True)
            for index, open_page in enumerate(context.pages):
                open_page.screenshot(path=shots / f"failed-{index}.png")
            print(f"failed; screenshots in {shots}", file=sys.stderr)
            raise
        finally:
            context.browser.close()
    print("all steps passed")


if __name__ == "__main__":
    main()
