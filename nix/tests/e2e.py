# The e2e test's driver script. e2e.nix defines the constants it uses
# (PKI, SELF, GIT_ADDRESS, HOST_IP, CONTROL_IP, IMAGE, HOSTD_CONFIG, PYTHON
# and BUSYBOX) before it.

import json
import re
import shlex
import time
import urllib.parse
from typing import Any

CONSOLE = "https://iglu.example.test"
# The test driver's shell has no HOME; the CLI keeps its token under it.
IGLU = "HOME=/root iglu"


def found(pattern: str, text: str) -> str:
    match = re.search(pattern, text)
    assert match, f"{pattern} not in {text}"
    return match.group(1)


def diagnose() -> None:
    for machine, units in [
        (control, "iglud caddy authelia-main nginx fcgiwrap-git"),
        (host, "iglu-hostd incus incus-preseed"),
    ]:
        flags = " ".join(f"-u {unit}" for unit in units.split())
        _, logs = machine.execute(f"journalctl --no-pager -n 150 {flags}")
        print(f"===== {machine.name}: {units} =====\n{logs}")
    # What iglud thinks of each workspace, and what happened to it.
    _, listing = control.execute(f"{IGLU} ls; for w in $({IGLU} --json ls | jq -r '.[].name'); do echo \"== $w\"; {IGLU} log $w; done")
    print(f"===== workspaces =====\n{listing}")
    # Each workspace's browser display, as the guest started it. Bounded,
    # so a stuck Incus can't hold the run for the driver's 15 minutes, as it
    # once did.
    _, displays = host.execute(
        "for i in $(timeout 30 incus list -c n -f csv); do echo \"== $i\"; "
        "timeout 30 incus exec -T $i -- sh -c 'tail -n 30 /run/user/*/iglu/display/*.log' </dev/null; done"
    )
    print(f"===== browser displays =====\n{displays}")
    if control.execute("test -d /tmp/browser")[0] == 0:
        control.copy_from_machine("/tmp/browser", "")


def browser(step: str, *args: str) -> Any:
    command = " ".join(shlex.quote(a) for a in (step, *args))
    return json.loads(control.succeed(f"browser {command}"))


def iglu(args: str) -> Any:
    return json.loads(control.succeed(f"{IGLU} --json {args}"))


def guest_of(instance: str):
    def run(command: str) -> str:
        return host.succeed(
            f"incus exec {instance} --user 1000 --group 100 --env HOME=/home/dev -- bash -lc {shlex.quote(command)}"
        )

    return run


def api_refusal(method: str, path: str, *curl: str) -> tuple[int, Any, str]:
    """Calls the API as the CLI's user, expecting a refusal: its status,
    its JSON body, and its headers."""
    token = control.succeed("jq -r .token /root/.config/iglu/credentials.json").strip()
    flags = " ".join(shlex.quote(c) for c in curl)
    status = control.succeed(
        f"curl -sS -D /tmp/refused-headers -o /tmp/refused-body -w '%{{http_code}}' -X {method} "
        f"-H 'Authorization: Bearer {token}' {flags} {shlex.quote(CONSOLE + path)}"
    )
    headers = control.succeed("cat /tmp/refused-headers").lower()
    assert "content-type: application/json" in headers, headers
    return int(status), json.loads(control.succeed("cat /tmp/refused-body")), headers


def phase_is(name: str, phase: str) -> None:
    control.wait_until_succeeds(
        f"{IGLU} --json show {name} | jq -e '.phase == \"{phase}\"'", timeout=600
    )


try:
    start_all()

    with subtest("the Git server has a private repository, over SSH and HTTPS"):
        control.wait_for_unit("sshd.service")
        control.succeed(
            "install -d -o git -m 700 /home/git/.ssh",
            f"install -o git -m 600 {PKI}/deploy.pub /home/git/.ssh/authorized_keys",
            "install -d -o git -m 755 /srv/git",
            "su git -c 'set -e; git init -q --bare -b main /srv/git/app.git; "
            "git init -q -b main /tmp/seed; cd /tmp/seed; echo hello > README; git add README; "
            "git -c user.name=t -c user.email=t@t commit -q -m init; git push -q /srv/git/app.git main'",
        )
        control.wait_for_unit("nginx.service")
        control.succeed(f"git -c http.sslCAInfo={PKI}/ca.crt ls-remote https://alice:pass@{GIT_ADDRESS}/app.git")
        control.fail(f"GIT_TERMINAL_PROMPT=0 git -c http.sslCAInfo={PKI}/ca.crt ls-remote https://{GIT_ADDRESS}/app.git")

    with subtest("services come up"):
        control.wait_for_unit("authelia-main.service")
        control.wait_for_open_port(9091)
        host.wait_for_unit("iglu-hostd.service")
        host.wait_for_open_port(7443)
        control.wait_for_unit("iglud.service")
        control.wait_for_open_port(443, CONTROL_IP)

    with subtest("hostd put the egress policy on the bridge"):
        acl = json.loads(host.succeed("incus query /1.0/network-acls/iglu-egress"))
        assert any(rule["action"] == "allow" for rule in acl["egress"]), acl
        bridge = host.succeed("incus network get iglubr0 security.acls").strip()
        assert bridge == "iglu-egress", bridge

    with subtest("a person signs in to the console in a browser"):
        control.succeed(
            "mkdir -p /root/.pki/nssdb",
            "certutil -d sql:/root/.pki/nssdb -N --empty-password",
            f"certutil -d sql:/root/.pki/nssdb -A -t C,, -n iglu-e2e -i {PKI}/ca.crt",
        )
        signed_in = browser("sign-in")
        assert signed_in["url"].startswith(CONSOLE), signed_in

    with subtest("a console page opened without a session goes straight to sign in, and back"):
        browser("signed-out", "/settings")

    with subtest("cross-origin mutations are refused"):
        cookie = next(
            c for c in json.loads(control.succeed("cat /root/browser-state.json"))["cookies"] if c["name"] == "__Host-iglu"
        )
        status = control.succeed(
            f"curl -sS -H 'Cookie: __Host-iglu={cookie['value']}' -o /dev/null -w '%{{http_code}}' -X POST "
            "-H 'Sec-Fetch-Site: cross-site' -H 'Content-Type: application/json' -d '{}' "
            f"{CONSOLE}/v1/workspaces"
        )
        assert status == "403", status

    with subtest("browsers check the console's page on every load and keep its assets, named for their contents"):
        page = control.succeed(f"curl -sS -i -H 'Cookie: __Host-iglu={cookie['value']}' {CONSOLE}/")
        assert "cache-control: no-cache" in page.lower(), page
        script = found(r'<script type="module" src="(/assets/app-\w+\.js)"', page)
        asset = control.succeed(f"curl -sS -D - -o /dev/null {CONSOLE}{script}").lower()
        assert "cache-control: public, max-age=31536000, immutable" in asset, asset
        missing = control.succeed(f"curl -sS -D - -o /dev/null -w '%{{http_code}}' {CONSOLE}/assets/app-missing.js").lower()
        assert missing.endswith("404") and "immutable" not in missing, missing

    with subtest("a sign-in finishes only in the browser that started it"):
        authorize = control.succeed(f"curl -sS -o /dev/null -w '%{{redirect_url}}' {CONSOLE}/auth/login")
        state = found(r"[?&]state=([^&]+)", authorize)
        # Another browser, without the sign-in cookie, follows the callback.
        page = control.succeed(f"curl -sS -i '{CONSOLE}/auth/callback?code=stolen&state={state}'")
        assert "browser that started the sign-in" in page, page
        # A person sees a page, not the API's JSON.
        assert "content-type: text/html" in page.lower(), page

    with subtest("the CLI signs in through the browser"):
        control.succeed(
            "systemd-run --unit=cli-login --setenv=HOME=/root "
            "-p StandardOutput=file:/tmp/login.log -p StandardError=file:/tmp/login.log "
            f"/run/current-system/sw/bin/iglu login {CONSOLE}"
        )
        control.wait_until_succeeds("grep -q 'Opening ' /tmp/login.log")
        browser("approve-cli", found(r"Opening (\S+)", control.succeed("cat /tmp/login.log")))
        control.wait_until_succeeds("grep -q 'signed in' /tmp/login.log")

    with subtest("an environment builds on the host"):
        iglu(f"env add example 'path:{SELF}#example'")
        control.wait_until_succeeds(
            f"{IGLU} --json env ls | jq -e '.[] | select(.name == \"example\") | .latest.status == \"Ready\"'",
            timeout=900,
        )

    with subtest("the guest tools are the host's, not the image's"):
        host.fail(f"nix-store --query --requisites $(readlink {IMAGE}/system) | grep -e iglu-guest -e '-zmx-'")
        host.succeed("test -x /var/lib/iglu-hostd/guest-tools/bin/iglu-guest")

    with subtest("hostd answers only iglud's client certificate"):
        # The control box trusts the host's certificate, but has no client
        # certificate of its own outside iglud's credentials.
        refused = control.fail("curl -sS https://host:7443/v1/inventory 2>&1")
        assert "alert" in refused or "certificate" in refused, refused

    with subtest("Incus passes the runtime conformance suite and its isolation checks"):
        # Its own workspaces, beside iglud's; iglud leaves instances it never made alone.
        print(host.succeed(f"iglu-hostd-conformance --config {HOSTD_CONFIG} --source 'path:{SELF}#example' --next 'path:{SELF}#example-next' 2>&1"))

    with subtest("a private environment can't be fetched without its owner's Git credential"):
        iglu(f"env add private 'github:alice/env?host={GIT_ADDRESS}#private'")
        control.wait_until_succeeds(
            f"{IGLU} --json env ls | jq -e '.[] | select(.name == \"private\") | .latest.status == \"Failed\"'",
            timeout=300,
        )
        private = next(e for e in iglu("env ls") if e["name"] == "private")
        assert "404" in private["latest"]["log_tail"], private

    with subtest("secrets are stored"):
        control.succeed(f"{IGLU} secret set deploy-key --file .ssh/id_ed25519 < {PKI}/deploy")
        control.succeed(f"printf hunter2 | {IGLU} secret set token --env TEST_TOKEN")
        control.succeed(f"printf pass | {IGLU} secret set git-https --git {GIT_ADDRESS} --username alice")
        control.succeed(f"{IGLU} secret set git-ca --file .config/git/ca.crt < {PKI}/ca.crt")
        control.succeed(
            f"printf '[http \"https://{GIT_ADDRESS}/\"]\\n\\tsslCAInfo = /home/dev/.config/git/ca.crt\\n' "
            f"| {IGLU} secret set git-config --file .config/git/config"
        )
        names = {s["name"] for s in iglu("secret ls")}
        assert names == {"deploy-key", "token", "git-https", "git-ca", "git-config"}, names
        assert "hunter2" not in control.succeed(f"{IGLU} --json secret ls")
        # A second secret for a destination one already has is refused.
        control.fail(f"printf other | {IGLU} secret set clash --env TEST_TOKEN")
        control.fail(f"printf other | {IGLU} secret set clash --file .config/git/config/extra")

    with subtest("every refused request says why, as JSON naming the input"):
        json_body = ("-H", "Content-Type: application/json")
        status, body, _ = api_refusal("POST", "/v1/environments", *json_body, "-d", '{"name":"broken","source":"github:a/b"}')
        assert status == 400 and body["reason"] == {"kind": "Invalid"} and body["field"] == "source", body
        # Why is data, for whoever shows it to say; iglud sends no prose.
        assert set(body) == {"reason", "field"}, body
        # A path parameter's own parser doesn't know its name; the route's only one is it.
        status, body, _ = api_refusal(
            "PUT", "/v1/secrets/Bad%20Name", *json_body, "-d", '{"target":{"kind":"env","name":"X"},"value":"x"}'
        )
        assert status == 400 and body["field"] == "name", body
        status, body, _ = api_refusal("GET", "/v1/workspaces/not-a-uuid")
        assert status == 400 and body["field"] == "id", body
        # Not the console's page, which every other path gets.
        status, body, _ = api_refusal("GET", "/v1/no-such-thing")
        assert status == 404 and body["reason"]["kind"] == "NotFound", body
        status, body, headers = api_refusal("DELETE", "/v1/me")
        assert status == 405 and body["reason"]["kind"] == "MethodNotAllowed", body
        assert "allow: get" in headers, headers
        status, body, _ = api_refusal("POST", "/v1/environments", "-d", "name=broken")
        assert status == 415 and body["reason"]["kind"] == "JsonExpected", body
        status, body, _ = api_refusal("POST", "/v1/environments", *json_body, "-d", '{"name":')
        assert status == 400 and body["field"] is None and body["reason"]["kind"] == "NotJson", body
        # The CLI says the server's reason, and the input it's about.
        code, said = control.execute(f"{IGLU} env add example 'path:{SELF}#example' 2>&1")
        assert code != 0 and "name: an environment with that name exists" in said, said

    with subtest("the console shows a refusal by the input it's about"):
        shown = browser("refused-environment")
        assert "flake#name" in shown["error"], shown

    with subtest("the same Git credential lets the host fetch the private environment"):
        iglu("env build private")
        control.wait_until_succeeds(
            f"{IGLU} --json env ls | jq -e '.[] | select(.name == \"private\") | .latest.status == \"Ready\"'",
            timeout=600,
        )
        # Only in the build's environment: not in its arguments, on disk, or in the logs.
        host.fail("grep -rqs 'access-tokens' /etc/nix /var/lib/iglu-build")
        host.fail("journalctl -u iglu-hostd --no-pager | grep -q '=pass'")

    with subtest("a workspace created in the browser starts from the private repository"):
        created = browser("create", "example", f"ssh://git@{GIT_ADDRESS}/srv/git/app.git", "demo")
        assert created["columns"] == ["shell"], created
        projects = {p["name"]: p for p in iglu("project ls")}
        assert projects.keys() == {"app", "general"}, projects
        assert projects["general"]["origin"] == "builtin" and projects["general"]["repo"] is None, projects
        ws = iglu("show demo")
        assert ws["phase"] == "running", ws
        instance = "iglu-" + ws["id"].replace("-", "")
        guest = guest_of(instance)

    with subtest("the checkout and secrets are in place"):
        assert "init" in guest("git -C ~/app log --oneline")
        assert guest("git -C ~/app branch --show-current").strip() == "demo"
        guest("test -L ~/.ssh/id_ed25519")

    with subtest("workspaces reach the Internet but not private networks"):
        guest(f"nc -z -w 5 {GIT_ADDRESS} 22")
        guest(f"! nc -z -w 5 {HOST_IP} 7443")
        # The host's own address in public space is still the host.
        guest("! nc -z -w 5 11.0.0.1 7443")
        guest(f"! nc -z -w 5 {CONTROL_IP} 443")

    # The console's own behaviour is dev-check's (dev/ui/check.py); these
    # steps are the ones that depend on a real guest.
    with subtest("the browser's terminal runs in the workspace with secrets in its environment, keeps its history, and takes pasted files"):
        browser("terminal", "demo", "echo token=$TEST_TOKEN", "token=hunter2")
        browser("keeps-history", "demo")
        browser("pastes-files", "demo")

    with subtest("a browser column shows the workspace's browser, which its agents drive too"):
        # The console shows the browser's display, not its pages, so what
        # happens on them is read as agents read it, at $IGLU_BROWSER.
        # A browser that's only just shown may not answer yet.
        def pages() -> list[Any]:
            listed = json.loads(guest("curl -sf $IGLU_BROWSER/json/list || echo []"))
            return [t for t in listed if t["type"] == "page"]

        def page_titled(pattern: str) -> str:
            for _ in range(60):
                found = [t["title"] for t in pages() if re.fullmatch(pattern, t["title"])]
                if found:
                    return found[0]
                time.sleep(1)
            raise AssertionError(f"no page titled {pattern}: {pages()}")

        browser("opens-browser", "demo")
        # Its first page's load clears the address bar, so typing waits for it.
        page_titled("New Tab")
        browser("browses", "demo")
        page_titled("Blank")
        browser("browser-answers", "demo")
        page_titled("confirmed hello")
        browser("browser-keeps", "demo")
        page_titled("typed pasted")
        # Its VNC server ending, as the display ends it for a size it
        # refuses, leaves the browser as it was: shown again, its page open.
        guest("pkill -x wayvnc")
        browser("browser-hides", "demo")
        page_titled("typed pasted")
        # Its port is iglu's own, and never published.
        refused = control.fail(f"{IGLU} port demo 9222 2>&1")
        assert "DevTools" in refused, refused
        control.succeed(f"{IGLU} column close browser -w demo")

    with subtest("programs copy and open through the console, as on a desktop"):
        # Programs that look for a browser without a display find iglu's.
        assert guest("echo $BROWSER").strip() == "/var/lib/iglu/tools/bin/xdg-open"
        browser("copies-from-programs", "demo")
        browser("opens-from-programs", "demo")
        opened = browser("opens-in-workspace", "demo")
        for _ in range(60):
            if any(t["url"] == opened["local"] for t in pages()):
                break
            time.sleep(1)
        else:
            raise AssertionError(f"{opened['local']} isn't open: {pages()}")
        control.succeed(f"{IGLU} column close browser -w demo")

    token = json.loads(control.succeed("cat /root/.config/iglu/credentials.json"))["token"]
    api = f"curl -sS --fail-with-body -H 'Authorization: Bearer {token}'"

    with subtest("a download is saved only from the workspace's Downloads, never through a link"):
        downloads = f"{CONSOLE}/v1/workspaces/{ws['id']}/downloads"
        saved = control.succeed(f"{api} -D /tmp/download-headers '{downloads}/note.txt'")
        assert saved == "saved here", saved
        assert "attachment" in control.succeed("cat /tmp/download-headers").lower()
        guest("ln -sf /etc/passwd ~/Downloads/passwd")
        status = f"curl -sS -o /dev/null -w '%{{http_code}}' -H 'Authorization: Bearer {token}'"
        assert control.succeed(f"{status} '{downloads}/passwd'") == "404"
        # As sent: curl would otherwise fold the dots away itself.
        assert control.succeed(f"{status} --path-as-is '{downloads}/%2E%2E'") == "400"

    with subtest("API tokens attach to columns too"):
        terminal = json.loads(
            control.succeed(
                f"{api} -X POST -H 'Content-Type: application/json' "
                f"-d '{{\"kind\": {{\"kind\": \"Shell\"}}}}' {CONSOLE}/v1/workspaces/{ws['id']}/columns"
            )
        )["name"]
        attach = f"wss://iglu.example.test/v1/workspaces/{ws['id']}/columns/{terminal}/attach?cols=80&rows=24"
        control.succeed(
            f"(sleep 2; printf 'echo token=$TEST_TOKEN\\r'; sleep 3) "
            f"| websocat {shlex.quote(attach)} -b -H 'Authorization: Bearer {token}' > /tmp/terminal.out || true"
        )
        output = control.succeed("cat /tmp/terminal.out")
        assert "token=hunter2" in output, f"the terminal should see the secret: {output!r}"
        columns = json.loads(control.succeed(f"{api} {CONSOLE}/v1/workspaces/{ws['id']}/columns"))
        assert [(c["name"], c["state"]) for c in columns] == [
            ("shell", "Open"),
            (terminal, "Open"),
        ], columns
        git_state = json.loads(control.succeed(f"{api} {CONSOLE}/v1/workspaces/{ws['id']}/live"))["git"]
        assert git_state["branch"] == "demo" and git_state["conflicted"] == 0, git_state

    with subtest("the CLI attaches to a column and detaches with Ctrl-]"):
        control.succeed(
            "(sleep 3; printf 'echo via-cli-$((6*7))\\r'; sleep 3; printf '\\035') "
            f"| script -qfec '{IGLU} attach demo shell' /tmp/attach.out"
        )
        output = control.succeed("cat /tmp/attach.out")
        assert "via-cli-42" in output and "detached from demo shell" in output, output

    with subtest("Claude Code's hooks report attention"):
        settings = json.loads(guest("cat /etc/claude-code/managed-settings.d/50-iglu.json"))
        hook = settings["hooks"]["Notification"][0]["hooks"][0]["command"]
        # Payloads as Claude Code sends them; see crates/iglu-guest/src/claude.rs.
        base = {"session_id": "s1", "transcript_path": "/home/dev/.claude/projects/p/s1.jsonl", "cwd": "/home/dev/app"}
        events = {
            "tool": {
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": "echo probe", "description": "Print the word probe"},
                "tool_use_id": "t1",
            },
            "permission": {
                "hook_event_name": "Notification",
                "message": "Claude needs your permission to use Bash",
                "notification_type": "permission_prompt",
            },
            "idle": {
                "hook_event_name": "Notification",
                "message": "Claude is waiting for your input",
                "notification_type": "idle_prompt",
            },
        }
        for event, payload in events.items():
            guest(f"cat > /tmp/{event}.json <<'EOF'\n{json.dumps(base | payload)}\nEOF")
        # From the browser's first terminal, where Claude Code would run.
        browser("terminal", "demo", f"{hook} < /tmp/tool.json && echo tool-$((40+2))", "tool-42")
        control.wait_until_succeeds(
            f"{IGLU} --json show demo | jq -e '.threads[] | select(.session == \"shell\" and .thread == \"s1\") | "
            ".state == \"working\" and .summary == \"Bash: Print the word probe\"'",
            timeout=60,
        )
        browser("terminal", "demo", f"{hook} < /tmp/permission.json && echo permission-$((40+2))", "permission-42")
        # An idle prompt follows every stop; it mustn't clear the question.
        browser("terminal", "demo", f"{hook} < /tmp/idle.json && echo idle-$((40+2))", "idle-42")
        control.wait_until_succeeds(
            f"{IGLU} --json show demo | jq -e '.attention.state == \"waiting\"'", timeout=60
        )
        waiting = browser("card", "demo", "Claude needs your permission to use Bash")
        assert "needs" in (waiting["class"] or ""), waiting

    with subtest("a port published in the browser is served behind preview sign-in"):
        # An app that takes connections at once, as real ones do: iglu checks
        # the port is reachable while browsers load it. A loop around `nc -l`
        # had no listener between connections, and one in three runs failed.
        host.succeed(f"incus file push {BUSYBOX} {instance}/tmp/busybox --mode 0755")
        guest("mkdir -p ~/site && printf hello > ~/site/index.html")
        # httpd puts itself in the background.
        guest("/tmp/busybox httpd -p 3000 -h ~/site")
        control.wait_until_succeeds(
            f"{api} {CONSOLE}/v1/workspaces/{ws['id']}/live | "
            "jq -e '.listeners[] | select(.port == 3000) | .reachable and .route == null'",
            timeout=60,
        )
        label_host = urllib.parse.urlsplit(iglu("port demo 3000")["url"]).hostname
        resolve = f"--resolve {label_host}:443:{CONTROL_IP} --resolve auth.dev.example.test:443:{CONTROL_IP}"
        anonymous = control.succeed(f"curl -sS {resolve} -o /dev/null -w '%{{http_code}} %{{redirect_url}}' https://{label_host}/")
        assert anonymous.startswith("30") and "auth.dev.example.test" in anonymous, anonymous
        published = browser("publish", "demo", "3000")
        assert urllib.parse.urlsplit(published["url"]).hostname == label_host, published
        assert published["landed"].startswith(published["url"]), published

    with subtest("a preview page can't act on the console with the person's cookies"):
        result = browser("attack", published["url"], ws["id"])
        assert result == {"read": "blocked", "socket": "refused"}, result
        assert "forged" not in {w["name"] for w in iglu("ls")}
        columns = json.loads(control.succeed(f"{api} {CONSOLE}/v1/workspaces/{ws['id']}/columns"))
        assert "forged" not in {c["name"] for c in columns}, columns

    with subtest("opening a frozen workspace's preview thaws it"):
        iglu("freeze demo --wait")
        browser("visit", published["url"])
        phase_is("demo", "running")
        assert "ThawedOnOpen" in [entry["kind"] for entry in iglu("log demo")]

    with subtest("a workspace clones over HTTPS with the Git credential secret"):
        project = iglu(f"project add app-https --repo https://{GIT_ADDRESS}/app.git --env example")
        # Its workspaces publish port 3000 as they're created.
        change = {key: project[key] for key in ("name", "repo", "environment", "opening", "idle")}
        change |= {"expected_revision": project["revision"], "ports": [3000]}
        control.succeed(
            f"{api} -X PUT -H 'Content-Type: application/json' -d {shlex.quote(json.dumps(change))} "
            f"{CONSOLE}/v1/projects/{project['id']}"
        )
        https = iglu("new app-https --name over-https --wait")
        assert [route["port"] for route in https["routes"]] == [3000], https
        assert https["phase"] == "running", https
        https_instance = "iglu-" + https["id"].replace("-", "")
        assert "init" in guest_of(https_instance)("git -C ~/app log --oneline")
        # Unpublishing it is in its history, as publishing it is.
        route = https["routes"][0]
        control.succeed(f"{api} -X DELETE {CONSOLE}/v1/workspaces/{https['id']}/routes/{route['id']}")
        history = [(entry["kind"], entry.get("port"), entry.get("route")) for entry in iglu("log over-https")]
        assert ("Published", 3000, route["name"]) in history, history
        assert ("Unpublished", 3000, route["name"]) in history, history

    with subtest("a workspace without a repository opens a shell, named for what it's called"):
        scratch = iglu("new general --label 'Fix the login bug' --wait")
        assert scratch["name"] == "fix-the-login-bug", scratch
        assert scratch["phase"] == "running" and scratch["checkout"] is None, scratch
        guest_of("iglu-" + scratch["id"].replace("-", ""))("test ! -e ~/app")
        columns = json.loads(control.succeed(f"{api} {CONSOLE}/v1/workspaces/{scratch['id']}/columns"))
        assert [(c["name"], c["state"]) for c in columns] == [("shell", "Open")], columns
        assert json.loads(control.succeed(f"{api} {CONSOLE}/v1/workspaces/{scratch['id']}/live"))["git"] is None
        control.fail(f"{IGLU} new general --name branchy --branch main")
        control.fail(f"{IGLU} project rm general")
        iglu(f"rm {scratch['name']} --wait")

    with subtest("a workspace upgrades in place to its environment's newest build, keeping its files"):
        host.succeed(f"cp -r {CHANGING_ENV} /var/lib/changing-env && chmod -R a+rX,u+w /var/lib/changing-env")
        iglu("env add changing 'path:/var/lib/changing-env#changing'")
        ready = "jq -e '.[] | select(.name == \"changing\") | .latest.status == \"Ready\"'"
        control.wait_until_succeeds(f"{IGLU} --json env ls | {ready}", timeout=600)
        iglu("project add changing --env changing")
        updating = iglu("new changing --name updating --wait")
        assert not updating["newer_build"], updating
        updating_instance = "iglu-" + updating["id"].replace("-", "")
        inside = guest_of(updating_instance)
        inside("echo kept > ~/mark && echo also > /var/tmp/mark")
        assert inside("cat /etc/iglu-example 2>&1 || true").strip() != "next"
        before = json.loads(host.succeed(f"incus query /1.0/instances/{updating_instance}"))["config"]
        # Its environment is built again, with a change.
        host.succeed("touch /var/lib/changing-env/next")
        iglu("env build changing")
        control.wait_until_succeeds(f"{IGLU} --json env ls | {ready}", timeout=600)
        control.wait_until_succeeds(f"{IGLU} --json show updating | jq -e .newer_build", timeout=30)
        iglu("update updating --wait")
        updated = iglu("show updating")
        assert updated["phase"] == "running" and not updated["newer_build"], updated
        after = json.loads(host.succeed(f"incus query /1.0/instances/{updating_instance}"))["config"]
        assert after["user.iglu.image"] != before["user.iglu.image"], (before, after)
        assert after["user.iglu.system"] != before["user.iglu.system"], (before, after)
        inside = guest_of(updating_instance)
        assert inside("cat /etc/iglu-example").strip() == "next"
        assert inside("cat ~/mark").strip() == "kept"
        assert inside("cat /var/tmp/mark").strip() == "also"
        assert inside("readlink /run/current-system").strip() == after["user.iglu.system"]
        # The old system is gone from its store.
        inside(f"test ! -e {before['user.iglu.system']}")
        history = [entry["kind"] for entry in iglu("log updating")]
        assert "UpdateAsked" in history and "Updated" in history, history
        iglu("rm updating --wait --force")
        iglu("project rm changing")

    with subtest("workspaces can't reach each other"):
        neighbour = guest_of(https_instance)
        neighbour("setsid nc -lk 0.0.0.0 4444 >/dev/null 2>&1 < /dev/null &")
        state = json.loads(host.succeed(f"incus query /1.0/instances/{https_instance}/state"))
        address = next(a["address"] for a in state["network"]["eth0"]["addresses"] if a["family"] == "inet")
        # The listener answers on that address, so the refusal below is the
        # policy's, not a listener that isn't there. Only the neighbour
        # itself can show it: the egress policy keeps guests from answering
        # the host's private bridge address too.
        neighbour(f"for i in $(seq 30); do nc -z -w 5 {address} 4444 && exit 0; sleep 1; done; exit 1")
        guest(f"! nc -z -w 5 {address} 4444")

    with subtest("a workspace drives its own columns with iglu, and another only as far as it's granted"):
        # Inside, `iglu` is the workspace, through its channel; the network
        # checks above still hold.
        assert guest("iglu whoami").strip() == "the workspace demo"
        guest("iglu column new --name probe")
        guest("iglu column send probe 'printf done-%s 42' --enter")
        guest("for i in $(seq 30); do iglu column output probe | grep -q done-42 && exit 0; sleep 1; done; exit 1")
        guest("iglu column close probe")
        # Another workspace of the same owner is out of sight until granted.
        guest("! iglu column ls -w over-https")
        guest("iglu secret ls 2>&1 | grep -q \"its owner can\"")
        iglu("access set demo --on over-https --allow view,read_output")
        assert "shell" in guest("iglu column ls -w over-https")
        guest("iglu column send shell -w over-https hi 2>&1 | grep -q \"hasn't been granted\"")
        # The owner's own token, sent through the channel, is still the
        # workspace. Without -N: Incus's proxy ends a connection whose client
        # stops writing, before the answer.
        refused = guest(
            "printf 'GET /v1/secrets HTTP/1.1\\r\\nHost: iglu\\r\\n"
            f"Authorization: Bearer {token}\\r\\nConnection: close\\r\\n\\r\\n' "
            "| nc -U $IGLU_SOCKET"
        )
        assert refused.startswith("HTTP/1.1 403"), refused
        iglu("access set demo --on over-https --allow")
        guest("! iglu column ls -w over-https")
        history = [(entry["kind"], entry["via"]) for entry in iglu("log demo")]
        assert ("ColumnAdded", "demo") in history, history

    with subtest("freezing reclaims memory and thawing resumes"):

        def resident() -> int:
            return int(host.succeed(f"cat /sys/fs/cgroup/lxc.payload.{instance}/memory.current"))

        before = resident()
        assert iglu("freeze demo --wait")["phase"] == "frozen"
        after = resident()
        assert after < before // 2, f"freezing should reclaim memory: {before} -> {after} bytes"
        assert iglu("start demo --wait")["phase"] == "running"
        assert "init" in guest("git -C ~/app log --oneline")

    with subtest("after a host restart, running workspaces come back and frozen ones stop"):
        assert iglu("freeze over-https --wait")["phase"] == "frozen"
        boot = guest("cat /proc/sys/kernel/random/boot_id").strip()
        host.shutdown()
        host.start()
        host.wait_for_unit("iglu-hostd.service")
        # iglud keeps showing the last observation while the host is away, so
        # wait for one from after the restart before trusting a phase.
        phase_is("over-https", "stopped")
        phase_is("demo", "running")
        assert guest("cat /proc/sys/kernel/random/boot_id").strip() != boot
        assert iglu("show over-https")["desired"] == "stopped"
        history = [entry["kind"] for entry in iglu("log over-https")]
        assert "Adopted" in history, history
        assert "init" in guest("git -C ~/app log --oneline")
        # hostd listens for each workspace's channel again.
        guest("for i in $(seq 30); do iglu whoami && exit 0; sleep 1; done; exit 1")
        # Reapplying the preseed at boot must not trip over the existing pool.
        print("pool source:", host.succeed("incus storage get iglu source"))
        assert host.succeed("systemctl show -p Result --value incus-preseed").strip() == "success"

    with subtest("a workspace waits for memory, then starts"):
        available = int(found(r"MemAvailable:\s+(\d+) kB", host.succeed("cat /proc/meminfo"))) * 1024
        hog = available - 200 * 1024 * 1024
        host.succeed(
            "systemd-run --unit=hog -p MemorySwapMax=0 "
            f"{PYTHON} -c 'import time; b = b\"x\" * {hog}; time.sleep(3600)'"
        )
        host.wait_until_succeeds(
            f"test $(awk '/MemAvailable/ {{ print $2 * 1024 }}' /proc/meminfo) -lt {512 * 1024 * 1024}"
        )
        iglu("new app-https --name waits")
        control.wait_until_succeeds(
            f"{IGLU} --json show waits | jq -e '.condition.kind == \"Capacity\"'", timeout=120
        )
        browser("card", "waits", "Waiting for room")
        host.succeed("systemctl stop hog")
        phase_is("waits", "running")
        # Its branch was never pushed, so deleting it would lose it.
        refused = control.fail(f"{IGLU} rm waits 2>&1")
        assert "never pushed" in refused, refused
        iglu("rm waits --force --wait")

    with subtest("an interrupted delete is finished, and unknown instances are left alone"):
        stranger = "0123456789abcdef0123456789abcdef"
        stranger_id = f"{stranger[:8]}-{stranger[8:12]}-{stranger[12:16]}-{stranger[16:20]}-{stranger[20:]}"
        host.succeed(
            f"incus create --empty iglu-{stranger} -p iglu "
            f"-c user.iglu.workspace={stranger_id} -c user.iglu.owner={stranger_id} "
            "-c 'user.iglu.guest-user={\"name\":\"dev\",\"uid\":1000,\"gid\":100,\"home\":\"/home/dev\"}'"
        )
        # As if iglud had recorded the delete and then crashed before the host acted.
        control.succeed(
            "systemctl stop iglud",
            "sqlite3 /var/lib/iglud/iglu.db \"UPDATE workspace SET desired = 'deleted', deleted_at = 1 WHERE name = 'over-https'\"",
            "systemctl start iglud",
        )
        host.wait_until_succeeds(f"! incus info {https_instance}", timeout=120)
        control.wait_until_succeeds(
            f"journalctl -u iglud | grep -q 'never seen.*{stranger_id}\\|{stranger_id}.*never seen'", timeout=60
        )
        host.succeed(f"incus info iglu-{stranger}")
        host.succeed(f"incus delete iglu-{stranger}")

    with subtest("iglud copies its database at every start, keeping the newest two"):

        def copies() -> list[str]:
            names = control.succeed("ls /var/lib/iglud/backups").split()
            assert all(re.fullmatch(r"iglu-\d+\.db", name) for name in names), names
            return sorted(names, key=lambda name: int(name[len("iglu-") : -len(".db")]))

        # The first start had no database to copy; the restart above made one.
        assert len(copies()) == 1, copies()
        for expected in (2, 2):
            control.succeed("systemctl restart iglud")
            control.wait_for_open_port(7080)
            assert len(copies()) == expected, copies()
        newest = f"/var/lib/iglud/backups/{copies()[-1]}"
        assert control.succeed(f"sqlite3 {newest} 'PRAGMA integrity_check'").strip() == "ok"
        live = control.succeed(f"sqlite3 {newest} 'SELECT name FROM workspace WHERE deleted_at IS NULL'").split()
        assert live == ["demo"], live

    with subtest("stopping and deleting clean up the instance"):
        assert iglu("stop demo --wait")["phase"] == "stopped"
        control.fail(f"{IGLU} rm demo")
        iglu("rm demo --force --wait")
        host.wait_until_succeeds(f"! incus info {instance}", timeout=120)
except Exception:
    diagnose()
    raise
