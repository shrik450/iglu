"""Fills the dev stack with workspaces in every state worth looking at.

    python3 dev/ui/seed.py

Run it against a fresh dev stack (`just dev-reset`, then `just dev`). It
makes fixture repositories that Caddy serves at https://git.localhost, then
signs in as alice and bob and uses the console's API and terminals, as a
person would, to reach each state. The `scripted` agent stands in for a real
one, reporting whichever attention state its prompt names.
"""

import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from common import Api, CONSOLE, launch, shared, sign_in, type_in  # noqa: E402
from playwright.sync_api import sync_playwright  # noqa: E402

GIT = "https://git.localhost"
# A server column's port. Local workspaces share this machine's loopback, so
# it's one nothing else is likely to use.
PREVIEW_PORT = 18431


def fixture(name: str, commits: list[dict[str, str]]) -> str:
    """A bare repository served at git.localhost, made from `commits`, each
    a map of path to contents. Fixed dates keep its hashes the same."""
    target = shared() / "git" / f"{name}.git"
    if target.exists():
        return f"{GIT}/{name}.git"
    env = {
        **os.environ,
        "GIT_AUTHOR_NAME": "Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.org",
        "GIT_COMMITTER_NAME": "Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.org",
        "GIT_AUTHOR_DATE": "2026-01-01T00:00:00Z", "GIT_COMMITTER_DATE": "2026-01-01T00:00:00Z",
    }
    with tempfile.TemporaryDirectory(dir=shared()) as work:
        git = lambda *args: subprocess.run(["git", *args], cwd=work, env=env, check=True, capture_output=True)  # noqa: E731
        git("init", "-q", "-b", "main")
        for index, files in enumerate(commits):
            for path, contents in files.items():
                (Path(work) / path).parent.mkdir(parents=True, exist_ok=True)
                (Path(work) / path).write_text(contents)
            git("add", "-A")
            git("commit", "-q", "-m", f"Commit {index + 1}")
        git("clone", "-q", "--bare", work, str(target))
    # Git's dumb HTTP protocol needs this index of refs.
    subprocess.run(["git", "update-server-info"], cwd=target, check=True)
    return f"{GIT}/{name}.git"


def main() -> None:
    app = fixture("app", [
        {"README.md": "# app\n\nA tiny web app for trying iglu.\n", "index.html": "hello\n"},
        {"src/main.py": "print('hello')\n"},
        {"README.md": "# app\n\nA tiny web app for trying iglu.\n\nRun `python3 -m http.server`.\n"},
    ])
    notes = fixture("notes", [{"README.md": "# notes\n"}])

    with sync_playwright() as playwright:
        context = launch(playwright, viewport={"width": 1440, "height": 900})
        page = context.new_page()
        sign_in(page, "alice")
        api = Api(page)

        api.environment("default", "github:you/iglu-env#workspace")
        projects = {p["name"]: p for p in api.get("/v1/projects")}
        for repo in (app, notes, f"{GIT}/missing.git"):
            name = repo.rsplit("/", 1)[1].removesuffix(".git")
            if name not in projects:
                projects[name] = api.send("POST", "/v1/projects", {"repo": repo, "environment": "default"})
        # app's workspaces open with the stand-in agent and a shell.
        project = api.get(f"/v1/projects/{projects['app']['id']}")
        api.send("PUT", f"/v1/projects/{project['id']}", {
            "expected_revision": project["revision"],
            "name": project["name"],
            "repo": project["repo"],
            "environment": project["environment"],
            "opening": [
                {"kind": {"kind": "agent", "agent": "scripted"}, "width": "two-thirds"},
                {"kind": {"kind": "shell"}, "width": "third"},
            ],
            "agent": "scripted",
            "ports": [],
            "idle": {"kind": "never"},
        })

        existing = {ws["name"] for ws in api.get("/v1/workspaces")}
        wanted = [
            # name, project, prompt for the scripted agent
            ("fix-login-redirect", "app", "wait Approve the database migration before I run it"),
            ("refactor-billing-and-invoice-generation-pipeline", "app", "work Rewriting how invoice totals are rounded"),
            ("docs-typos", "app", "done Fixed 12 typos across the docs"),
            ("agent-crashed", "app", "fail Ran out of memory"),
            ("unsaved-work", "app", None),
            ("preview-server", "app", None),
            ("notes-frozen", "notes", None),
            ("notes-stopped", "notes", None),
            ("scratchpad", "general", None),
            ("broken-clone", "missing", None),
        ]
        created = []
        for name, project_name, prompt in wanted:
            if name in existing:
                continue
            request = {"project": projects[project_name]["id"], "name": name}
            if prompt:
                request["prompt"] = prompt
            api.create(**request)
            created.append((name, project_name))
        for name, project_name in created:
            if project_name != "missing":
                api.wait(name, "running")
        time.sleep(2)

        # Work that isn't saved: a commit that isn't pushed, and an edit that
        # isn't committed.
        page.goto(f"{CONSOLE}/w/unsaved-work")
        type_in(page, "shell", "git commit -q --allow-empty -m 'Local only' && echo >> README.md && printf 'saved-%s\\n' ok", "saved-ok")

        # A server, published at its own preview address.
        ws = api.workspace("preview-server")
        if not any(r["port"] == PREVIEW_PORT for r in ws.get("routes", [])):
            api.send("POST", f"/v1/workspaces/{ws['id']}/columns", {
                "kind": {"kind": "server", "command": ["python3", "-m", "http.server", str(PREVIEW_PORT), "--bind", "127.0.0.1"]},
                "name": "server",
            })
            time.sleep(2)
            api.send("POST", f"/v1/workspaces/{ws['id']}/routes", {"port": PREVIEW_PORT})

        for name, state, phase in (("notes-frozen", "frozen", "frozen"), ("notes-stopped", "stopped", "stopped")):
            ws = api.workspace(name)
            if ws["phase"] != phase:
                api.send("PUT", f"/v1/workspaces/{ws['id']}/desired-state", {"state": state, "expected_revision": ws["revision"]})
                api.wait(name, phase)

        if not any(s["name"] == "github-token" for s in api.get("/v1/secrets")):
            api.send("PUT", "/v1/secrets/github-token", {"target": {"kind": "env", "name": "GITHUB_TOKEN"}, "value": "not-a-real-token"})
        context.browser.close()

        # Someone else's workspace, which alice mustn't see.
        context = launch(playwright)
        page = context.new_page()
        sign_in(page, "bob")
        bob = Api(page)
        bob.environment("default", "github:you/iglu-env#workspace")
        if not bob.get("/v1/workspaces"):
            general = next(p for p in bob.get("/v1/projects") if p["origin"] == "builtin")
            bob.create(project=general["id"], name="bobs-secret-work")
        context.browser.close()

    print("seeded https://iglu.localhost: sign in as alice or bob, password 'password'")


if __name__ == "__main__":
    main()
