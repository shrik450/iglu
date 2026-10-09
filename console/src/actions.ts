// Everything the person can do, in one place, so keys, the palette and
// buttons share it.

import { effect, untracked } from "@preact/signals";

import { api, failure } from "./api/client.ts";
import type { ColumnKind } from "./generated/ColumnKind.ts";
import type { ColumnSpec } from "./generated/ColumnSpec.ts";
import type { CreateWorkspace } from "./generated/CreateWorkspace.ts";
import type { DesiredState } from "./generated/DesiredState.ts";
import type { RouteView } from "./generated/RouteView.ts";
import type { WorkspaceView } from "./generated/WorkspaceView.ts";
import { layoutOf, moved, type Shown, shown, stepIndex, widened } from "./state/layout.ts";
import {
  activeColumn,
  columnStates,
  current,
  cursor,
  details,
  inside,
  listed,
  navigate,
  overlay,
  projects,
  question,
  route,
  say,
  settle,
  waiting,
  workspaces,
} from "./state/store.ts";
import { panes } from "./terminal.ts";

/** Runs a request, showing its error instead of throwing. */
export async function attempt<T>(request: () => Promise<T>): Promise<T | undefined> {
  try {
    return await request();
  } catch (error) {
    say(failure(error));
    return undefined;
  }
}

export function open(ws: WorkspaceView): void {
  cursor.value = ws.id;
  navigate({ view: "workspace", name: ws.name });
}

export function back(): void {
  const ws = current.value;
  if (overlay.value) overlay.value = null;
  else if (ws && question.value) settle(ws);
  else if (details.value) details.value = false;
  else if (route.value.view !== "overview") navigate({ view: "overview" });
}

/** Moves the cursor through the list, opening the next workspace when one is open. */
export function step(direction: -1 | 1): void {
  const list = listed.value;
  if (list.length === 0) return;
  const at = list.findIndex((ws) => ws.id === (current.value?.id ?? cursor.value));
  const next = list[(at + direction + list.length) % list.length];
  if (!next) return;
  if (current.value) open(next);
  else cursor.value = next.id;
}

export function nextWaiting(): void {
  const list = waiting.value;
  const at = list.findIndex((ws) => ws.id === current.value?.id);
  const next = list[(at + 1) % Math.max(1, list.length)];
  if (next) open(next);
  else say("Nothing needs you.");
}

export async function setState(ws: WorkspaceView, state: DesiredState): Promise<void> {
  await attempt(() => api.setState(ws, state));
}

/** Deletes a workspace once the person confirmed, leaving it for the
 * overview only if iglu agreed; otherwise they stay, with the reason shown. */
export async function remove(ws: WorkspaceView): Promise<void> {
  const deleted = await attempt(() => api.setState(ws, "deleted"));
  settle(ws, "delete");
  if (deleted && current.value?.id === ws.id) navigate({ view: "overview" });
}

export async function toggleFreeze(ws: WorkspaceView): Promise<void> {
  if (ws.phase === "running") await setState(ws, "frozen");
  else if (ws.phase === "frozen") await setState(ws, "running");
}

/** Renames a workspace; throws what iglu refused, for the form to show. Its
 * new name shows at once, and the URL follows it, before the next snapshot. */
export async function rename(ws: WorkspaceView, name: string): Promise<void> {
  const renamed = await api.rename(ws.id, name);
  workspaces.value = workspaces.value.map((w) => (w.id === renamed.id ? renamed : w));
  settle(ws, "rename");
}

/** Creates a workspace and opens it; throws what iglu refused, for the form to show. */
export async function create(body: CreateWorkspace): Promise<void> {
  const ws = await api.create(body);
  overlay.value = null;
  open(ws);
}

/** Publishes a port; throws what iglu refused, for a form to show. */
export async function publish(ws: WorkspaceView, port: number): Promise<void> {
  const route = await api.publish(ws.id, port);
  say(`Published :${route.port} at ${route.url}`);
  settle(ws, "port");
}

export async function unpublish(ws: WorkspaceView, route: RouteView): Promise<void> {
  await attempt(() => api.unpublish(ws.id, route.id));
}

/** Makes the workspace's columns how new workspaces in its project open. */
export async function saveOpening(ws: WorkspaceView): Promise<void> {
  const project = projects.value.find((p) => p.id === ws.project);
  if (!project) return;
  const opening = ws.columns.map(({ name, kind, width }) => ({ name, kind, width }));
  const saved = await attempt(() =>
    api.changeProject(project.id, {
      expected_revision: project.revision,
      name: project.name,
      repo: project.repo,
      environment: project.environment,
      opening,
      agent: project.agent,
      ports: project.ports,
      idle: project.idle,
    }),
  );
  if (saved) say(`New workspaces in ${saved.name} open like this.`);
}

/** Asks the host what's listening and where the checkout stands. Polled:
 * a failure keeps the last answer and the next poll tries again. */
export async function loadLive(ws: WorkspaceView): Promise<void> {
  if (ws.phase !== "running") return;
  try {
    const now = await api.live(ws.id);
    inside.value = { ...inside.value, [ws.id]: now };
  } catch {
    // Polled: the next poll tries again, and the header keeps what it had.
  }
}

// ---- columns ----

export function columnsOf(ws: WorkspaceView): Shown[] {
  return shown(ws.columns, columnStates.value[ws.id]);
}

export function activeOf(ws: WorkspaceView): string | null {
  const columns = columnsOf(ws);
  const active = activeColumn.value[ws.id];
  return columns.some((c) => c.name === active) ? (active ?? null) : (columns[0]?.name ?? null);
}

/** Arriving at a workspace lands on the column of an agent that waits, or
 * else on the first column, scrolled to the start. It's decided here, as the
 * route changes and before the columns render. Only on arrival: a thread that
 * starts waiting while you're there mustn't pull focus from what you're doing. */
let arrived: WorkspaceView["id"] | null = null;
effect(() => {
  const ws = current.value;
  if (ws?.id === arrived) return;
  arrived = ws?.id ?? null;
  if (!ws) return;
  const waits = ws.attention?.state === "waiting" ? ws.attention.session : null;
  const landing = waits && ws.columns.some((c) => c.name === waits) ? waits : ws.columns[0]?.name;
  if (landing) untracked(() => markActive(ws, landing));
});

/** In a workspace, the keyboard belongs to the active column whenever
 * nothing else asked for it. So when a dialog, menu, form or confirmation
 * closes, the terminal gets it back, rather than the page, where typing
 * would go nowhere. */
let asking = false;
effect(() => {
  const now = Boolean(overlay.value) || question.value !== null || details.value;
  const closed = asking && !now;
  asking = now;
  if (!closed) return;
  window.setTimeout(() => {
    const ws = current.peek();
    const focus = document.activeElement;
    const free = !focus || focus === document.body || (Boolean(focus.closest(".main")) && !focus.closest("input, textarea, select, .info"));
    if (ws && free) {
      const name = activeOf(ws);
      if (name) panes.get(`${ws.id}/${name}`)?.focus();
    }
  });
});

/** Records which column has focus, without moving focus. */
export function markActive(ws: WorkspaceView, name: string): void {
  if (activeColumn.value[ws.id] !== name) activeColumn.value = { ...activeColumn.value, [ws.id]: name };
}

export function focusColumn(ws: WorkspaceView, name: string): void {
  markActive(ws, name);
  panes.get(`${ws.id}/${name}`)?.focus();
}

export function stepColumn(ws: WorkspaceView, direction: -1 | 1): void {
  const columns = columnsOf(ws);
  const active = activeOf(ws);
  const next = columns[stepIndex(columns.length, columns.findIndex((c) => c.name === active), direction)];
  if (next) focusColumn(ws, next.name);
}

/** Shows the new arrangement at once and saves it; the next snapshot settles any disagreement. */
async function arrange(ws: WorkspaceView, columns: ColumnSpec[]): Promise<void> {
  workspaces.value = workspaces.value.map((w) => (w.id === ws.id ? { ...w, columns } : w));
  await attempt(() => api.putLayout(ws.id, { columns: layoutOf(columns) }));
}

export async function moveColumn(ws: WorkspaceView, direction: -1 | 1): Promise<void> {
  const active = activeOf(ws);
  const next = active ? moved(ws.columns, active, direction) : null;
  if (next) await arrange(ws, next);
}

export async function cycleWidth(ws: WorkspaceView, name?: string): Promise<void> {
  const target = name ?? activeOf(ws);
  const next = target ? widened(ws.columns, target) : null;
  if (next) await arrange(ws, next);
}

export async function loadColumns(ws: WorkspaceView): Promise<void> {
  if (ws.phase !== "running") return;
  const list = await attempt(() => api.columns(ws.id));
  if (list) columnStates.value = { ...columnStates.value, [ws.id]: list };
}

/** Opens a column after the focused one and focuses it; throws what iglu
 * refused, for a form to show. */
export async function openColumn(ws: WorkspaceView, kind: ColumnKind): Promise<void> {
  if (ws.phase !== "running") return;
  const after = activeOf(ws);
  const created = await api.addColumn(ws.id, after ? { kind, after } : { kind });
  settle(ws, "add-column");
  markActive(ws, created.name);
  await loadColumns(ws);
}

/** openColumn for buttons and keys. */
export async function addColumn(ws: WorkspaceView, kind: ColumnKind): Promise<void> {
  settle(ws, "add-column");
  await attempt(() => openColumn(ws, kind));
}

export async function restartColumn(ws: WorkspaceView, name: string): Promise<void> {
  await attempt(() => api.restartColumn(ws.id, name));
  await loadColumns(ws);
}

/** Ends a column's session and its processes, and drops the column. Callers confirm first. */
export async function closeColumn(ws: WorkspaceView, name: string): Promise<void> {
  settle(ws, "end");
  await attempt(() => api.closeColumn(ws.id, name));
  await loadColumns(ws);
}
