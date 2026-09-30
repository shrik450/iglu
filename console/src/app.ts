// The iglu console: a sidebar of workspaces sorted by what needs you, and
// the selected workspace's terminals and ports.

import { Ghostty } from "ghostty-web";

import { api, ApiError, type Condition, type CreateWorkspace, type EnvironmentView, type Me, type WorkspaceView, watchWorkspaces } from "./api";
import { byAttention, glyph, needsYou } from "./attention";
import { h, replace } from "./dom";
import { TerminalPane } from "./terminal";

interface State {
  me: Me | null;
  workspaces: WorkspaceView[];
  selected: string | null;
  terminals: string[];
  activeTerminal: string | null;
  notice: string;
}

const state: State = { me: null, workspaces: [], selected: null, terminals: [], activeTerminal: null, notice: "" };

const sidebar = document.querySelector<HTMLElement>("#sidebar");
const header = document.querySelector<HTMLElement>("#workspace-header");
const tabs = document.querySelector<HTMLElement>("#tabs");
const terminalBox = document.querySelector<HTMLElement>("#terminal");
const ports = document.querySelector<HTMLElement>("#ports");
const statusLine = document.querySelector<HTMLElement>("#status");
const dialog = document.querySelector<HTMLDialogElement>("#create");
if (!sidebar || !header || !tabs || !terminalBox || !ports || !statusLine || !dialog) {
  throw new Error("the console page is missing elements");
}

let pane: TerminalPane | null = null;

function selected(): WorkspaceView | undefined {
  return state.workspaces.find((ws) => ws.id === state.selected);
}

function sorted(): WorkspaceView[] {
  return [...state.workspaces].sort(byAttention);
}

function conditionText(condition: Condition): string {
  switch (condition.kind) {
    case "error":
      return `${condition.message} (${condition.code}); retrying`;
    case "capacity":
      return "waiting for the host to have room";
    case "runtime_failed":
      return "the runtime reports this workspace as broken; stop or delete it";
    case "host_offline":
      return "the host isn't answering";
  }
}

function notify(message: string): void {
  state.notice = message;
  renderStatus();
  if (message) setTimeout(() => {
    if (state.notice === message) {
      state.notice = "";
      renderStatus();
    }
  }, 6000);
}

async function act(action: () => Promise<unknown>): Promise<void> {
  try {
    await action();
  } catch (error) {
    notify(error instanceof ApiError ? error.message : String(error));
  }
}

function renderSidebar(): void {
  const rows = sorted().map((ws) => {
    const summary = ws.condition ? conditionText(ws.condition) : ws.attention?.summary || ws.attention?.state || ws.phase;
    return h(
      "button",
      {
        class: `row${ws.id === state.selected ? " selected" : ""}${needsYou(ws) ? " attention" : ""}`,
        onclick: () => void select(ws.id),
        title: `${ws.repo} · ${ws.branch}`,
      },
      h("span", { class: "glyph" }, glyph(ws)),
      h("span", { class: "name" }, ws.name),
      h("span", { class: "summary" }, summary),
    );
  });
  replace(
    sidebar!,
    h("div", { class: "brand" }, h("strong", {}, "iglu"), h("span", { class: "who" }, state.me?.email ?? state.me?.name ?? "")),
    h("button", { class: "new", onclick: () => void openCreate() }, "+ New workspace"),
    rows.length ? h("nav", { class: "rows" }, ...rows) : h("p", { class: "empty" }, "No workspaces yet."),
    h("p", { class: "keys" }, "alt+j/k move · alt+n next needing you"),
  );
}

function renderHeader(): void {
  const ws = selected();
  if (!ws) {
    replace(header!, h("p", { class: "empty" }, "Select or create a workspace."));
    return;
  }
  const button = (label: string, target: "running" | "frozen" | "stopped" | "deleted", enabled: boolean) =>
    h(
      "button",
      {
        disabled: !enabled,
        onclick: () => {
          if (target === "deleted" && !confirm(`Delete ${ws.name}? Everything in it is lost; push your work first.`)) return;
          void act(() => api.setState(ws, target));
        },
      },
      label,
    );
  const live = ws.phase !== "deleting" && ws.phase !== "deleted";
  replace(
    header!,
    h(
      "div",
      { class: "title" },
      h("h1", {}, ws.name),
      h("span", { class: `phase ${ws.phase}` }, ws.phase),
      h("span", { class: "meta" }, `${ws.branch} · ${ws.repo} · ${ws.environment}`),
    ),
    ws.condition ? h("p", { class: "condition" }, conditionText(ws.condition)) : null,
    h(
      "div",
      { class: "actions" },
      button("Start", "running", live && (ws.phase === "stopped" || ws.phase === "frozen")),
      button("Freeze", "frozen", ws.phase === "running"),
      button("Stop", "stopped", live && ws.phase !== "stopped"),
      button("Delete", "deleted", live),
    ),
  );
}

function renderTabs(): void {
  const ws = selected();
  if (!ws || ws.phase !== "running") {
    replace(tabs!, ws ? h("span", { class: "hint" }, "Terminals open once the workspace is running.") : null);
    return;
  }
  const statusBySession = new Map(ws.sessions.map((s) => [s.session, s]));
  replace(
    tabs!,
    ...state.terminals.map((name) => {
      const status = statusBySession.get(name);
      return h(
        "span",
        { class: `tab${name === state.activeTerminal ? " active" : ""}` },
        h("button", { class: "tab-name", onclick: () => openTerminal(name) }, `${name}${status ? ` · ${status.state}` : ""}`),
        h(
          "button",
          {
            class: "close",
            title: "Close this terminal and end its processes",
            onclick: () => {
              if (confirm(`Close ${name}? Its processes end.`)) void act(() => closeTerminal(name));
            },
          },
          "×",
        ),
      );
    }),
    h("button", { class: "tab add", onclick: () => void act(newTerminal) }, "+"),
  );
}

function renderPorts(): void {
  const ws = selected();
  if (!ws) {
    replace(ports!);
    return;
  }
  const input = h("input", { type: "number", min: "1", max: "65535", placeholder: "port", "aria-label": "Port to publish" });
  replace(
    ports!,
    ...ws.routes.map((route) =>
      h(
        "span",
        { class: "route" },
        h("a", { href: route.url, target: "_blank", rel: "noopener" }, `:${route.port} ${route.name}`),
        h("button", { class: "close", title: "Unpublish", onclick: () => void act(() => api.unpublish(ws.id, route.id)) }, "×"),
      ),
    ),
    h(
      "form",
      {
        class: "publish",
        onsubmit: (event: Event) => {
          event.preventDefault();
          const port = Number(input.value);
          if (Number.isInteger(port) && port > 0 && port < 65536) void act(() => api.publish(ws.id, port));
          input.value = "";
        },
      },
      input,
      h("button", { type: "submit" }, "Publish"),
    ),
  );
}

function renderStatus(): void {
  statusLine!.textContent = state.notice;
}

function render(): void {
  renderSidebar();
  renderHeader();
  renderTabs();
  renderPorts();
  renderStatus();
}

let ghostty: Ghostty | null = null;

function openTerminal(name: string): void {
  const ws = selected();
  if (!ws || !ghostty) return;
  if (!pane) pane = new TerminalPane(terminalBox!, ghostty, shortcut, notify);
  state.activeTerminal = name;
  pane.attach(ws.id, name);
  renderTabs();
}

async function loadTerminals(): Promise<void> {
  const ws = selected();
  pane?.detach();
  state.terminals = [];
  state.activeTerminal = null;
  if (!ws || ws.phase !== "running") {
    renderTabs();
    return;
  }
  const list = await api.terminals(ws.id);
  state.terminals = list.map((t) => t.name).sort();
  renderTabs();
  const first = state.terminals[0];
  if (first) openTerminal(first);
  else await newTerminal();
}

async function newTerminal(): Promise<void> {
  const ws = selected();
  if (!ws) return;
  const { name } = await api.newTerminal(ws.id);
  if (!state.terminals.includes(name)) state.terminals = [...state.terminals, name].sort();
  openTerminal(name);
}

async function closeTerminal(name: string): Promise<void> {
  const ws = selected();
  if (!ws) return;
  if (state.activeTerminal === name) pane?.detach();
  await api.closeTerminal(ws.id, name);
  state.terminals = state.terminals.filter((t) => t !== name);
  state.activeTerminal = null;
  const next = state.terminals[0];
  if (next) openTerminal(next);
  renderTabs();
}

async function select(id: string): Promise<void> {
  if (state.selected === id) return;
  state.selected = id;
  history.replaceState(null, "", `/w/${id}`);
  render();
  await act(() => api.seen(id));
  await act(loadTerminals);
}

function move(step: number): void {
  const list = sorted();
  if (!list.length) return;
  const index = list.findIndex((ws) => ws.id === state.selected);
  const next = list[(index + step + list.length) % list.length];
  if (next) void select(next.id);
}

function nextNeedingYou(): void {
  const next = sorted().find((ws) => needsYou(ws) && ws.id !== state.selected);
  if (next) void select(next.id);
}

/** Console shortcuts. Returns true when the key was ours, so the terminal ignores it. */
function shortcut(event: KeyboardEvent): boolean {
  if (event.type !== "keydown" || !event.altKey || event.ctrlKey || event.metaKey) return false;
  switch (event.code) {
    case "KeyJ":
      move(1);
      return true;
    case "KeyK":
      move(-1);
      return true;
    case "KeyN":
      nextNeedingYou();
      return true;
    default:
      return false;
  }
}

async function openCreate(): Promise<void> {
  const environments: EnvironmentView[] = await api.environments().catch(() => []);
  const ready = environments.filter((e) => e.latest?.status === "ready");
  const form = dialog!.querySelector("form");
  const select = dialog!.querySelector<HTMLSelectElement>("select[name=environment]");
  if (!form || !select) return;
  replace(select, ...ready.map((e) => h("option", { value: e.name }, e.name)));
  if (!ready.length) {
    notify("No environment is built yet. Run `iglu env add default <flake>#<attribute>` first.");
    return;
  }
  dialog!.showModal();
}

function setupCreate(): void {
  const form = dialog!.querySelector("form");
  if (!form) return;
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const data = new FormData(form);
    const text = (key: string) => {
      const value = String(data.get(key) ?? "").trim();
      return value === "" ? undefined : value;
    };
    const environment = text("environment");
    const repo = text("repo");
    if (!environment || !repo) return;
    const body: CreateWorkspace = { environment, repo };
    const branch = text("branch");
    const base = text("base");
    const name = text("name");
    if (branch) body.branch = branch;
    if (base) body.base = base;
    if (name) body.name = name;
    void act(async () => {
      const ws = await api.create(body);
      dialog!.close();
      form.reset();
      state.workspaces = [...state.workspaces.filter((w) => w.id !== ws.id), ws];
      await select(ws.id);
    });
  });
  dialog!.querySelector("button[value=cancel]")?.addEventListener("click", (event) => {
    event.preventDefault();
    dialog!.close();
  });
}

function onWorkspaces(workspaces: WorkspaceView[]): void {
  const before = selected();
  state.workspaces = workspaces;
  const after = selected();
  if (state.selected && !after) {
    state.selected = null;
    pane?.detach();
    state.terminals = [];
  }
  if (after && before?.phase !== after.phase && (after.phase === "running" || before?.phase === "running")) {
    void act(loadTerminals);
  }
  if (after && after.attention?.seen === "unseen" && document.visibilityState === "visible") {
    void act(() => api.seen(after.id));
  }
  render();
}

async function main(): Promise<void> {
  document.addEventListener("keydown", (event) => {
    if (shortcut(event)) event.preventDefault();
  });
  setupCreate();
  state.me = await api.me();
  ghostty = await Ghostty.load("/ghostty-vt.wasm");
  state.workspaces = await api.workspaces();
  const fromUrl = location.pathname.match(/^\/w\/([0-9a-f-]{36})$/)?.[1];
  render();
  if (fromUrl && state.workspaces.some((ws) => ws.id === fromUrl)) await select(fromUrl);
  watchWorkspaces(onWorkspaces);
}

main().catch((error: unknown) => {
  if (!(error instanceof ApiError && error.status === 401)) notify(`The console failed to start: ${String(error)}`);
});
