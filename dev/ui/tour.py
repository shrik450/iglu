"""Visits every view of the console and keeps what a reviewer needs.

    python3 dev/ui/tour.py [--label NAME] [--quick] [--only TEXT]

For each stop, in each variant (desktop and phone, dark and light, Chromium
and WebKit), it saves a screenshot. For the main variant it also saves the
accessibility tree and an axe audit, and for every variant it records console
errors and horizontal overflow. Everything lands in .dev/tour/<label>/, with an
index.html to look through and report.json to read. Seed the dev stack first
(`just dev-seed`); `dev/ui/compare.py` shows what changed between two tours.
"""

import argparse
import html
import json
import re
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

sys.path.insert(0, str(Path(__file__).parent))

from common import CONSOLE, ROOT, launch, sign_in  # noqa: E402
from playwright.sync_api import Page, sync_playwright  # noqa: E402

AXE = ROOT / "console/node_modules/axe-core/axe.min.js"


@dataclass(frozen=True)
class Variant:
    name: str
    engine: str
    width: int
    height: int
    scheme: str
    # The variant whose accessibility tree and audit are kept; the audit runs
    # in light too, since contrast differs by theme.
    main: bool = False
    audit: bool = False


VARIANTS = [
    Variant("desktop-dark", "chromium", 1440, 900, "dark", main=True, audit=True),
    Variant("desktop-light", "chromium", 1440, 900, "light", audit=True),
    Variant("phone-dark", "chromium", 390, 844, "dark"),
    Variant("phone-light", "chromium", 390, 844, "light"),
    Variant("webkit-desktop-dark", "webkit", 1440, 900, "dark"),
    Variant("webkit-phone-light", "webkit", 390, 844, "light"),
]


@dataclass(frozen=True)
class Stop:
    name: str
    path: str
    user: str = "alice"
    # Brings the page to the state worth seeing, after it loads.
    act: Callable[[Page], None] = lambda page: None
    notes: str = ""


def press_palette(page: Page) -> None:
    page.keyboard.press("ControlOrMeta+k")
    page.get_by_role("dialog").wait_for()


def click(role: str, name: str) -> Callable[[Page], None]:
    def act(page: Page) -> None:
        page.get_by_role(role, name=name, exact=True).first.click()
    return act


def delete_confirmation(page: Page) -> None:
    page.get_by_role("button", name="Details").click()
    page.get_by_role("button", name="Delete", exact=True).first.click()


def refused_environment(page: Page) -> None:
    form = page.get_by_role("form", name="Add an environment")
    form.get_by_label("Name").fill("broken")
    form.get_by_label("Flake").fill("github:you/env")
    form.get_by_role("button", name="Add").click()
    form.locator(".field-err").wait_for()


def stops() -> list[Stop]:
    workspaces = [
        ("fix-login-redirect", "an agent waiting for you"),
        ("refactor-billing-and-invoice-generation-pipeline", "a working agent and a long name"),
        ("docs-typos", "an agent that's done"),
        ("agent-crashed", "an agent that exited with an error"),
        ("unsaved-work", "uncommitted and unpushed work"),
        ("preview-server", "a server column and a published preview"),
        ("notes-frozen", "frozen"),
        ("notes-stopped", "stopped"),
        ("scratchpad", "no repository"),
        ("broken-clone", "a clone that fails"),
    ]
    return [
        Stop("overview", "/"),
        Stop("previews", "/previews"),
        Stop("settings", "/settings"),
        Stop("settings-refused", "/settings", act=refused_environment, notes="a refused environment"),
        Stop("project-app", "/p/app"),
        Stop("project-general", "/p/general"),
        Stop("palette", "/", act=press_palette),
        Stop("new-workspace", "/", act=click("button", "New workspace")),
        Stop("new-project", "/", act=click("button", "New project")),
        *[Stop(f"w-{name}", f"/w/{name}", notes=notes) for name, notes in workspaces],
        Stop("add-column", "/w/scratchpad", act=click("button", "Add a column")),
        Stop("details", "/w/unsaved-work", act=click("button", "Details")),
        Stop("delete-unsaved", "/w/unsaved-work", act=delete_confirmation, notes="what deleting would lose"),
        Stop("bob-overview", "/", user="bob", notes="another person's view"),
    ]


@dataclass
class Shot:
    stop: str
    variant: str
    image: str
    errors: list[str] = field(default_factory=list)
    overflow: int = 0
    notifications: list[str] = field(default_factory=list)
    failed: str = ""


# Records notifications the console shows, without showing them.
NOTIFICATIONS = """
(() => {
  const shown = [];
  window.__notifications = shown;
  class Recorded {
    static permission = "granted";
    static requestPermission() { return Promise.resolve("granted"); }
    constructor(title, options) { shown.push(title + (options && options.body ? `: ${options.body}` : "")); }
    close() {}
    addEventListener() {}
  }
  window.Notification = Recorded;
})();
"""


def settle(page: Page) -> None:
    page.wait_for_load_state("load")
    page.locator("#stage, main").first.wait_for()
    # Terminals draw after their sockets connect; snapshots arrive over SSE.
    time.sleep(1.5)


def visit(page: Page, stop: Stop, variant: Variant, out: Path, shot: Shot) -> None:
    page.goto(f"{CONSOLE}{stop.path}")
    settle(page)
    stop.act(page)
    time.sleep(0.6)
    shot.overflow = int(page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth"))
    page.screenshot(path=out / shot.image, full_page=True, animations="disabled", caret="hide")
    shot.notifications = page.evaluate("window.__notifications || []")
    if variant.main:
        (out / f"{stop.name}.aria.yaml").write_text(page.locator("body").aria_snapshot())
    if variant.audit:
        # Evaluated by the browser's tooling, which the console's Content
        # Security Policy rightly doesn't allow as a page script.
        page.evaluate(AXE.read_text())
        result = page.evaluate("axe.run(document, { resultTypes: ['violations'] })")
        (out / f"{stop.name}--{variant.name}.axe.json").write_text(json.dumps([
            {"id": v["id"], "impact": v["impact"], "help": v["help"], "nodes": [n["target"] for n in v["nodes"]]}
            for v in result["violations"]
        ], indent=1))


def index(out: Path, shots: list[Shot], chosen: list[Stop], variants: list[Variant]) -> None:
    by = {(s.stop, s.variant): s for s in shots}
    rows = []
    for stop in chosen:
        cells = []
        for variant in variants:
            shot = by.get((stop.name, variant.name))
            if not shot:
                continue
            flags = []
            if shot.failed:
                flags.append(f"<b class=bad>failed: {html.escape(shot.failed)}</b>")
            if shot.errors:
                flags.append(f"<span class=bad>{len(shot.errors)} console errors</span>")
            if shot.overflow > 0:
                flags.append(f"<span class=bad>overflows by {shot.overflow}px</span>")
            audit = out / f"{stop.name}--{variant.name}.axe.json"
            if audit.exists() and (found := json.loads(audit.read_text())):
                flags.append(f"<span class=bad>{len(found)} axe rules</span>")
            cells.append(
                f"<figure><a href='{shot.image}'><img loading=lazy src='{shot.image}'></a>"
                f"<figcaption>{variant.name} {' '.join(flags)}</figcaption></figure>"
            )
        rows.append(f"<section><h2>{stop.name} <small>{html.escape(stop.notes)}</small></h2><div class=row>{''.join(cells)}</div></section>")
    (out / "index.html").write_text(f"""<!doctype html><meta charset=utf-8><title>iglu tour {out.name}</title>
<style>body{{font:14px system-ui;margin:16px;background:#111;color:#ddd}}.row{{display:flex;gap:12px;overflow-x:auto}}
figure{{margin:0}}img{{height:320px;border:1px solid #333}}.bad{{color:#f77}}small{{color:#999;font-weight:normal}}</style>
<h1>iglu tour {out.name}</h1>{''.join(rows)}""")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--label", default=time.strftime("%Y%m%d-%H%M%S"))
    parser.add_argument("--quick", action="store_true", help="only the main variant")
    parser.add_argument("--only", help="only stops whose name contains this")
    args = parser.parse_args()
    out = ROOT / ".dev/tour" / args.label
    out.mkdir(parents=True, exist_ok=True)
    chosen = [s for s in stops() if not args.only or args.only in s.name]
    variants = [v for v in VARIANTS if v.main] if args.quick else VARIANTS
    shots: list[Shot] = []
    with sync_playwright() as playwright:
        for variant in variants:
            for user in sorted({s.user for s in chosen}):
                context = launch(
                    playwright, variant.engine,
                    viewport={"width": variant.width, "height": variant.height},
                    color_scheme=variant.scheme, reduced_motion="reduce",
                )
                context.add_init_script(NOTIFICATIONS)
                page = context.new_page()
                errors: list[str] = []
                page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
                page.on("pageerror", lambda e: errors.append(str(e)))
                sign_in(page, user)
                for stop in (s for s in chosen if s.user == user):
                    shot = Shot(stop.name, variant.name, f"{stop.name}--{variant.name}.png")
                    errors.clear()
                    try:
                        visit(page, stop, variant, out, shot)
                    except Exception as error:  # a stop that fails is a finding, not the end of the tour
                        shot.failed = re.sub(r"\s+", " ", str(error))[:300]
                        page.screenshot(path=out / shot.image, full_page=True)
                    shot.errors = list(errors)
                    shots.append(shot)
                    print(f"{variant.name:22} {stop.name:30} {'FAILED' if shot.failed else 'ok'}", flush=True)
                context.browser.close()
    (out / "report.json").write_text(json.dumps([s.__dict__ for s in shots], indent=1))
    index(out, shots, chosen, variants)
    print(f"{out / 'index.html'}")


if __name__ == "__main__":
    main()
