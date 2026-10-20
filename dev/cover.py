"""Coverage of every tier the dev shell can run: Rust from the unit tests,
the local runtime's conformance suite and the dev stack under `just
dev-check`, the CLI included; the console from its unit tests and from the
browsers the terminal tier and dev-check drive.

    python3 dev/cover.py all         # just coverage: every tier, then report
    python3 dev/cover.py diff <ref>  # just coverage-diff: the lines changed since <ref>
    python3 dev/cover.py gate <ref>  # fail unless the changed lines meet the bar

    python3 dev/cover.py clean       # forget earlier runs
    python3 dev/cover.py unit        # the unit tests and conformance suite, measured
    python3 dev/cover.py terminal    # the terminal tier, in Chromium
    python3 dev/cover.py check       # dev-check, against `stack.py run --coverage`
    python3 dev/cover.py report      # lcov and a summary under .dev/coverage/

`all` and `report` take `--baseline <summary.json>` to show each tier's
change from an earlier run's summary. `all` runs a dev stack of its own, so
stop `just dev` first.

Rust is built with LLVM's source-based coverage into target/coverage. Its
programs write their counters to the profile as they change (continuous
mode), so processes the stack ends with a signal, and guest tools a
terminal's detaching kills, still count.

The console's browser coverage comes from V8, through the DevTools
protocol, mapped back to the TypeScript through each bundle's source map.
V8 forgets a document's counts with it, so they're taken before each
navigation the tools make and before pages close; a navigation a click
makes, such as through sign-in, loses what ran since the last one.
"""

import json
import os
import shlex
import shutil
import signal
import subprocess
import sys
from datetime import UTC, datetime
from functools import cache
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "target/coverage"
OUT = ROOT / ".dev/coverage"
# What the tools write, before `report` gathers it.
RAW = OUT / "raw"
BROWSER_RAW = RAW / "browser"
# Set for the dev tools' browsers to collect the console's coverage into.
BROWSER_ENV = "IGLU_COVERAGE_BROWSER"
# The sources each tier counts, by repository path.
CONSOLE_SOURCES = ("console/src/", "console/vendor/wterm-dom/src/")


@cache
def rust_env() -> dict[str, str]:
    """The environment that builds Rust with coverage into target/coverage,
    and runs it writing profiles there."""
    base = {**os.environ, "CARGO_TARGET_DIR": str(TARGET)}
    shown = subprocess.run(
        ["cargo", "llvm-cov", "show-env", "--sh"],
        cwd=ROOT, env=base, check=True, capture_output=True, text=True,
    ).stdout
    env = dict(base)
    for line in shown.splitlines():
        if line.startswith("export "):
            key, _, value = line.removeprefix("export ").partition("=")
            env[key] = shlex.split(value)[0] if value else ""
    # %c keeps the profile current as counters change, which Linux needs
    # relocatable counters for.
    env["RUSTFLAGS"] = "-Cllvm-args=-runtime-counter-relocation"
    env["LLVM_PROFILE_FILE"] = str(TARGET / "iglu-%p-%m%c.profraw")
    return env


def run(command: list[str], env: dict[str, str] | None = None, cwd: Path = ROOT) -> None:
    subprocess.run(command, cwd=cwd, env=env, check=True)


def clean() -> None:
    """Forgets earlier runs' profiles and reports, keeping what's built."""
    for profile in TARGET.glob("*.profraw"):
        profile.unlink()
    shutil.rmtree(OUT, ignore_errors=True)


def unit() -> None:
    """Runs the Rust tests, the local runtime's conformance suite included,
    and the console's unit tests, measuring each."""
    clean()
    env = rust_env()
    run(["cargo", "build", "-p", "iglu-guest"], env)
    # A failing test is the test jobs' to report; coverage counts what ran.
    tested = subprocess.run(
        ["cargo", "test", "--workspace", "--no-fail-fast"],
        cwd=ROOT, env={**env, "IGLU_GUEST_TOOLS": str(TARGET / "debug")},
    )
    if tested.returncode:
        print("cover: some Rust tests failed; their coverage still counts", file=sys.stderr)
    rust_lcov("rust-unit")
    import stack

    stack.console_dependencies()
    subprocess.run(
        [
            "node", "--test", "--experimental-test-coverage",
            "--test-coverage-exclude=**/*.test.ts", "--test-coverage-exclude=node_modules/**",
            "--test-reporter=dot", "--test-reporter-destination=stdout",
            "--test-reporter=lcov", f"--test-reporter-destination={RAW / 'console-unit.lcov'}",
            "src/**/*.test.ts", "vendor/wterm-dom/src/**/*.test.ts",
        ],
        cwd=ROOT / "console",
    )


def terminal() -> None:
    """Runs the terminal tier in Chromium, whose V8 counts what ran, with
    its browser collecting the console's coverage."""
    tested = subprocess.run(
        [sys.executable, str(ROOT / "console/test/terminal/run.py"), "--browser", "chromium"],
        cwd=ROOT, env={**os.environ, BROWSER_ENV: str(BROWSER_RAW)},
    )
    # As with the unit tests, a failure is the test jobs' to report.
    if tested.returncode:
        print("cover: some terminal cases failed; their coverage still counts", file=sys.stderr)


def check() -> None:
    """Runs dev-check with the browsers collecting the console's coverage.
    The stack must be running with `stack.py run --coverage` for Rust's."""
    # The CLI dev-check runs is the stack's measured build, writing where it does.
    env = {
        **os.environ,
        BROWSER_ENV: str(BROWSER_RAW),
        "IGLU_DEV_BIN": str(TARGET / "debug"),
        "LLVM_PROFILE_FILE": rust_env()["LLVM_PROFILE_FILE"],
    }
    run([sys.executable, str(ROOT / "dev/ui/check.py")], env)
    # The stack's profiles are current as it runs, so they're read now.
    rust_lcov("rust")


def rust_lcov(name: str) -> None:
    """Rust's coverage so far, from every profile in target/coverage."""
    RAW.mkdir(parents=True, exist_ok=True)
    run(["cargo", "llvm-cov", "report", "--lcov", "--output-path", str(RAW / f"{name}.lcov")], rust_env())


# lcov, as {repository path: {line: count}}.
Lines = dict[str, dict[int, int]]


def read_lcov(path: Path, base: Path = ROOT) -> Lines:
    """An lcov file's lines, by repository path; relative paths in it are
    relative to `base`."""
    files: Lines = {}
    current: dict[int, int] | None = None
    for line in path.read_text().splitlines():
        if line.startswith("SF:"):
            source = (base / line[3:]).resolve()
            current = files.setdefault(str(source.relative_to(ROOT)), {}) if source.is_relative_to(ROOT) else None
        elif line.startswith("DA:") and current is not None:
            number, count = line[3:].split(",")[:2]
            current[int(number)] = current.get(int(number), 0) + int(count)
        elif line == "end_of_record":
            current = None
    return files


def write_lcov(path: Path, files: Lines) -> None:
    with path.open("w") as out:
        for name, lines in sorted(files.items()):
            out.write(f"SF:{name}\n")
            for number, count in sorted(lines.items()):
                out.write(f"DA:{number},{count}\n")
            out.write(f"LF:{len(lines)}\nLH:{sum(1 for c in lines.values() if c)}\nend_of_record\n")


def merge(*tiers: Lines) -> Lines:
    merged: Lines = {}
    for tier in tiers:
        for name, lines in tier.items():
            into = merged.setdefault(name, {})
            for number, count in lines.items():
                into[number] = into.get(number, 0) + count
    return merged


B64 = {c: i for i, c in enumerate("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/")}


def vlq(segment: str) -> list[int]:
    values, value, shift = [], 0, 0
    for char in segment:
        digit = B64[char]
        value += (digit & 31) << shift
        if digit & 32:
            shift += 5
        else:
            values.append(-(value >> 1) if value & 1 else value >> 1)
            value, shift = 0, 0
    return values


@cache
def mapped(bundle: Path) -> list[tuple[int, str, int]]:
    """Each source-map segment of a bundle the console's sources made: its
    offset in the bundle, in UTF-16 units as V8 counts, with the repository
    path and line it came from."""
    source_map = json.loads(bundle.with_name(bundle.name + ".map").read_text())
    sources = []
    # watch_browsers made each source absolute as it kept the map.
    for source in source_map["sources"]:
        path = Path(source)
        name = str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else ""
        sources.append(name if name.startswith(CONSOLE_SOURCES) else "")
    starts, offset = [], 0
    for line in bundle.read_text().split("\n"):
        starts.append(offset)
        offset += len(line.encode("utf-16-le")) // 2 + 1
    segments = []
    source = source_line = 0
    for number, line in enumerate(source_map["mappings"].split(";")):
        column = 0
        for segment in filter(None, line.split(",")):
            fields = vlq(segment)
            column += fields[0]
            if len(fields) >= 4:
                source += fields[1]
                source_line += fields[2]
                if sources[source]:
                    segments.append((starts[number] + column, sources[source], source_line + 1))
    return segments


def browser_lines() -> Lines:
    """The console's lines the browsers ran, from V8's block counts: each
    point counts as the innermost range around it."""
    total: Lines = {}
    for snapshot in sorted(BROWSER_RAW.glob("snap-*.json")):
        for script in json.loads(snapshot.read_text()):
            bundle = BROWSER_RAW / "assets" / Path(urlparse(script["url"]).path).name
            ranges = sorted(
                ((r["startOffset"], r["endOffset"], r["count"]) for f in script["functions"] for r in f["ranges"]),
                key=lambda r: (r[0], -r[1]),
            )
            seen: Lines = {}
            stack: list[tuple[int, int, int]] = []
            index = 0
            for offset, name, line in mapped(bundle):
                while index < len(ranges) and ranges[index][0] <= offset:
                    while stack and stack[-1][1] <= ranges[index][0]:
                        stack.pop()
                    stack.append(ranges[index])
                    index += 1
                while stack and stack[-1][1] <= offset:
                    stack.pop()
                lines = seen.setdefault(name, {})
                lines[line] = max(lines.get(line, 0), stack[-1][2] if stack else 0)
            total = merge(total, seen)
    return total


def area(name: str) -> str:
    """What a summary groups a file under: its crate, or a part of the console."""
    parts = name.split("/")
    if parts[0] == "crates":
        return f"crates/{parts[1]}"
    if name.startswith("console/vendor/wterm-dom/"):
        return "console/vendor/wterm-dom"
    if name.startswith("console/src/") and len(parts) > 3:
        return f"console/src/{parts[2]}"
    return "/".join(parts[:2])


def totals(files: Lines) -> dict[str, Any]:
    areas: dict[str, list[int]] = {}
    for name, lines in files.items():
        counted = areas.setdefault(area(name), [0, 0])
        counted[0] += len(lines)
        counted[1] += sum(1 for c in lines.values() if c)
    found = sum(a[0] for a in areas.values())
    hit = sum(a[1] for a in areas.values())
    return {"found": found, "hit": hit, "areas": {k: {"found": v[0], "hit": v[1]} for k, v in sorted(areas.items())}}


def percent(hit: int, found: int) -> str:
    return f"{100 * hit / found:.1f}%" if found else "-"


# What each tier is, in the order the summary shows them.
TIERS = {
    "rust-unit": "Rust: unit tests and the conformance suite",
    "rust": "Rust: those and dev-check",
    # Node counts only the files the tests load.
    "console-unit": "Console: unit tests, in the files they load",
    "console-browser": "Console: the terminal tier's and dev-check's browsers",
    "console": "Console: both",
}


def report(baseline: Path | None = None) -> None:
    """Writes lcov for each tier, summary.json, summary.md and Rust's HTML
    report under .dev/coverage, and prints the summary. With a baseline
    summary.json, the summary shows each tier's change from it."""
    if any(TARGET.glob("*.profraw")):
        run(["cargo", "llvm-cov", "report", "--html", "--output-dir", str(OUT / "rust-html")], rust_env())
    tiers: dict[str, Lines] = {}
    for name in ("rust-unit", "rust"):
        if (RAW / f"{name}.lcov").exists():
            tiers[name] = read_lcov(RAW / f"{name}.lcov")
    if (RAW / "console-unit.lcov").exists():
        # Node writes paths relative to where the tests ran.
        unit = read_lcov(RAW / "console-unit.lcov", ROOT / "console")
        tiers["console-unit"] = {k: v for k, v in unit.items() if k.startswith(CONSOLE_SOURCES)}
    if BROWSER_RAW.exists():
        tiers["console-browser"] = browser_lines()
    if "console-unit" in tiers or "console-browser" in tiers:
        tiers["console"] = merge(tiers.get("console-unit", {}), tiers.get("console-browser", {}))
    for name, files in tiers.items():
        write_lcov(OUT / f"{name}.lcov", files)
    summary = {
        "commit": os.environ.get("GITHUB_SHA") or git("rev-parse", "HEAD"),
        "date": datetime.now(UTC).isoformat(timespec="seconds"),
        "tiers": {name: totals(files) for name, files in tiers.items()},
    }
    (OUT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    before = json.loads(baseline.read_text())["tiers"] if baseline and baseline.exists() else {}
    markdown = summary_markdown(summary["tiers"], before)
    (OUT / "summary.md").write_text(markdown)
    print(markdown)


def summary_markdown(tiers: dict[str, Any], before: dict[str, Any]) -> str:
    def change(name: str, now: dict[str, int]) -> str:
        was = before.get(name)
        if not was or not was["found"] or not now["found"]:
            return ""
        delta = 100 * (now["hit"] / now["found"] - was["hit"] / was["found"])
        return f" ({delta:+.1f})"

    out = ["## Coverage", "", "Lines covered by each tier" + (", with the change from main" if before else "") + ".", ""]
    out += ["| Tier | Lines | Covered |", "| --- | ---: | ---: |"]
    for name, label in TIERS.items():
        if name in tiers:
            t = tiers[name]
            out.append(f"| {label} | {t['found']} | {percent(t['hit'], t['found'])}{change(name, t)} |")
    # Each area by the most complete tier that counts it.
    for name in ("rust", "rust-unit", "console"):
        if name not in tiers or (name == "rust-unit" and "rust" in tiers):
            continue
        tier = tiers[name]
        out += ["", f"<details><summary>{TIERS[name]}, by area</summary>", "", "| Area | Lines | Covered |", "| --- | ---: | ---: |"]
        for key, t in tier["areas"].items():
            out.append(f"| `{key}` | {t['found']} | {percent(t['hit'], t['found'])} |")
        out += ["", "</details>"]
    return "\n".join(out) + "\n"


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def diff(ref: str) -> None:
    """Coverage of the lines changed since `ref`, for Rust across every
    tier and for the console across both of its, as diff_cover reports it."""
    reports = [OUT / f"{name}.lcov" for name in ("rust", "console") if (OUT / f"{name}.lcov").exists()]
    if not reports:
        sys.exit("no coverage yet; run `python3 dev/cover.py report` first")
    subprocess.run(
        [
            "diff-cover", *map(str, reports), f"--compare-branch={ref}",
            f"--format=markdown:{OUT / 'diff.md'},json:{OUT / 'diff.json'}",
        ],
        cwd=ROOT, check=True,
    )


# The bar for a pull request's changed lines: at least this share covered,
# or no more than this many uncovered. The allowance keeps a small change
# from failing on a line or two that no test can reach in the dev shell.
GATE_SHARE = 0.9
GATE_ALLOWANCE = 3


def gated(name: str) -> bool:
    """Whether a file's changed lines count toward the bar. Test harnesses
    are code under test's tests, not code; and what only a real host runs,
    Incus and hostd's own binaries, is the VM test's, which isn't measured."""
    if name.startswith("crates/") and "/tests/" in name:
        return False
    if name.endswith(("/sim.rs", "/conformance.rs")):
        return False
    if name.startswith("crates/iglu-hostd/src/incus/"):
        return name == "crates/iglu-hostd/src/incus/observe.rs"
    return name != "crates/iglu-hostd/src/main.rs"


def gate(ref: str) -> None:
    """Fails unless the lines changed since `ref` meet the bar, saying which
    are uncovered."""
    diff(ref)
    files = json.loads((OUT / "diff.json").read_text())["src_stats"]
    counted = {name: stats for name, stats in files.items() if gated(name)}
    covered = sum(len(stats["covered_lines"]) for stats in counted.values())
    missed = {name: stats["violation_lines"] for name, stats in counted.items() if stats["violation_lines"]}
    uncovered = sum(len(lines) for lines in missed.values())
    total = covered + uncovered
    verdict = f"{covered} of {total} changed lines covered ({percent(covered, total)})"
    if uncovered <= GATE_ALLOWANCE or covered >= GATE_SHARE * total:
        print(f"cover: {verdict}; the bar is {GATE_SHARE:.0%}, or at most {GATE_ALLOWANCE} uncovered")
        return
    for name, lines in sorted(missed.items()):
        print(f"{name}: {', '.join(map(str, lines))}", file=sys.stderr)
    sys.exit(
        f"cover: {verdict}, under the bar of {GATE_SHARE:.0%}, or at most {GATE_ALLOWANCE} uncovered. "
        "Test the lines above, in the cheapest tier that reaches them."
    )


# Collecting the console's coverage in the dev tools' browsers.

_watching = False


def watch_browsers(into: Path) -> None:
    """Makes every Chromium page the tools open collect the console's
    coverage into `into`: started as a page opens, taken before it
    navigates or closes and before its context or browser closes."""
    global _watching
    if _watching:
        return
    _watching = True
    from playwright.sync_api import Browser, BrowserContext, Error, Page

    (into / "assets").mkdir(parents=True, exist_ok=True)
    sessions: dict[Page, Any] = {}
    taken = [0]

    def profile(session: Any) -> None:
        session.send("Profiler.enable")
        session.send("Profiler.startPreciseCoverage", {"callCount": True, "detailed": True})

    def start(page: Page) -> None:
        try:
            session = page.context.new_cdp_session(page)
            profile(session)
        except Error:
            return
        sessions[page] = session

        # A navigation to another site moves the page to another process,
        # which starts without the profiler.
        def navigated(frame: Any) -> None:
            if frame == page.main_frame:
                try:
                    profile(session)
                except Error:
                    pass

        page.on("framenavigated", navigated)

    def take(page: Page) -> None:
        session = sessions.get(page)
        if session is None:
            return
        try:
            scripts = session.send("Profiler.takePreciseCoverage")["result"]
        except Error:
            return
        bundles = [s for s in scripts if served(s["url"])]
        for script in bundles:
            keep(served(script["url"]))
        if bundles:
            taken[0] += 1
            (into / f"snap-{os.getpid()}-{taken[0]:05}.json").write_text(json.dumps(bundles))

    def served(url: str) -> Path | None:
        """The bundle a script came from, when it's one of the console's,
        built with a source map: the dev stack's or the terminal tier's."""
        if not url.startswith(("http://", "https://")) or not url.endswith(".js"):
            return None
        name = Path(urlparse(url).path).name
        for built in (ROOT / "console/dist/assets", ROOT / ".dev/terminal"):
            if (built / f"{name}.map").exists():
                return built / name
        return None

    def keep(bundle: Path | None) -> None:
        """Copies a bundle, with its map's sources made absolute, as a later
        build may replace it."""
        assert bundle
        kept = into / "assets" / bundle.name
        if kept.exists():
            return
        source_map = json.loads(bundle.with_name(bundle.name + ".map").read_text())
        base = bundle.parent / source_map.pop("sourceRoot", "")
        source_map["sources"] = [str((base / source).resolve()) for source in source_map["sources"]]
        kept.with_name(kept.name + ".map").write_text(json.dumps(source_map))
        shutil.copy(bundle, kept)

    def watch(context: BrowserContext) -> None:
        if context.browser is None or context.browser.browser_type.name != "chromium":
            return
        context.on("page", start)
        for page in context.pages:
            start(page)

    def before(cls: type, method: str, pages: Any) -> None:
        original = getattr(cls, method)

        def wrapped(self: Any, *args: Any, **kwargs: Any) -> Any:
            for page in pages(self):
                take(page)
            return original(self, *args, **kwargs)

        setattr(cls, method, wrapped)

    new_context = Browser.new_context

    def watched(self: Browser, *args: Any, **kwargs: Any) -> BrowserContext:
        context = new_context(self, *args, **kwargs)
        watch(context)
        return context

    Browser.new_context = watched  # type: ignore[method-assign]
    for method in ("goto", "reload", "close"):
        before(Page, method, lambda page: [page])
    before(BrowserContext, "close", lambda context: list(context.pages))
    before(Browser, "close", lambda browser: [p for c in browser.contexts for p in c.pages])


def everything(baseline: Path | None) -> None:
    """Every tier: the unit tests, then dev-check against a dev stack of its
    own, measuring, and the report."""
    import socket

    import stack

    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", stack.PORT)) == 0:
            sys.exit(f"something serves port {stack.PORT}; stop `just dev` first, as this runs its own")
    unit()
    terminal()
    # Built first, so the stack starts well within the time it's waited for.
    run(["cargo", "build", "-p", "iglud", "-p", "iglu-devhost", "-p", "iglu-guest", "-p", "iglu-cli"], rust_env())
    with (OUT / "stack.log").open("wb") as log:
        running = subprocess.Popen(
            [sys.executable, str(ROOT / "dev/stack.py"), "run", "--coverage"], cwd=ROOT, stdout=log, stderr=log,
        )
        try:
            stack.wait()
            check()
        finally:
            # As Ctrl-C does: the stack stops its children.
            running.send_signal(signal.SIGINT)
            running.wait(timeout=60)
    report(baseline)


def main() -> None:
    match sys.argv[1:]:
        case ["all"]:
            everything(None)
        case ["all", "--baseline", baseline]:
            everything(Path(baseline))
        case ["clean"]:
            clean()
        case ["unit"]:
            unit()
        case ["terminal"]:
            terminal()
        case ["check"]:
            check()
        case ["report"]:
            report()
        case ["report", "--baseline", baseline]:
            report(Path(baseline))
        case ["diff", ref]:
            diff(ref)
        case ["gate", ref]:
            gate(ref)
        case _:
            sys.exit(__doc__)


if __name__ == "__main__":
    main()
