// The console's state, as signals. Views read them; actions in actions.ts write them.

import { computed, effect, signal, untracked } from "@preact/signals";

import type { ColumnStatus } from "../generated/ColumnStatus.ts";
import type { LiveView } from "../generated/LiveView.ts";
import type { EnvironmentView } from "../generated/EnvironmentView.ts";
import type { Me } from "../generated/Me.ts";
import type { ProjectId } from "../generated/ProjectId.ts";
import type { ProjectView } from "../generated/ProjectView.ts";
import type { WorkspaceId } from "../generated/WorkspaceId.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { groupByProject } from "./groups.ts";
import { type Look, loadCollapsed, loadLook, saveCollapsed, saveLook, watch } from "./prefs.ts";
import { formatRoute, parseRoute, type Route } from "./route.ts";

export const me = signal<Me | null>(null);
/** In the core's standing order, most pressing first. */
export const workspaces = signal<WorkspaceView[]>([]);
export const projects = signal<ProjectView[]>([]);
export const environments = signal<EnvironmentView[]>([]);
export const live = signal(false);
/** iglu restarted, likely upgraded, since this page loaded: its code may no
 * longer match what iglu sends, so it asks to be reloaded. */
export const outdated = signal(false);

export const route = signal<Route>(parseRoute(location.pathname));
addEventListener("popstate", () => {
  route.value = parseRoute(location.pathname);
});

export function navigate(next: Route, mode: "push" | "replace" = "push"): void {
  const path = formatRoute(next);
  if (path !== location.pathname) history[mode === "push" ? "pushState" : "replaceState"](null, "", path);
  route.value = next;
}

export const groups = computed(() => groupByProject(projects.value, workspaces.value));
/** The order j and k walk: groups as shown, then each group's order. */
export const listed = computed(() => groups.value.flatMap((group) => group.workspaces));
export const waiting = computed(() => workspaces.value.filter((ws) => ws.needs_you));

/** The workspace last shown, by the name it had then. */
let shown: { id: WorkspaceId; name: string } | null = null;

/** The workspace the URL names. A URL still naming the workspace shown by the
 * name it had then follows it to its new name, so a rename, in this tab or
 * any other, keeps the same workspace open rather than losing it. */
export const current = computed(() => {
  const r = route.value;
  if (r.view !== "workspace") return null;
  const list = workspaces.value;
  const named = list.find((ws) => ws.name === r.name);
  if (named) return named;
  return shown?.name === r.name ? (list.find((ws) => ws.id === shown?.id) ?? null) : null;
});
effect(() => {
  const ws = current.value;
  if (!ws) return;
  shown = { id: ws.id, name: ws.name };
  const r = route.peek();
  if (r.view === "workspace" && r.name !== ws.name) navigate({ view: "workspace", name: ws.name }, "replace");
});

/** The overview's cursor, by ID so renames don't lose it. */
export const cursor = signal<WorkspaceId | null>(null);

export const look = signal<Look>(loadLook());
effect(() => {
  const value = look.value;
  if (value === "auto") delete document.documentElement.dataset["look"];
  else document.documentElement.dataset["look"] = value;
  saveLook(value);
});
watch("look", () => (look.value = loadLook()));

export const collapsed = signal<ReadonlySet<string>>(loadCollapsed());
effect(() => saveCollapsed(collapsed.value));
watch("collapsed", () => (collapsed.value = loadCollapsed()));

/** Each workspace's columns as its host last reported them. */
export const columnStates = signal<Record<WorkspaceId, ColumnStatus[]>>({});
/** What's going on inside each workspace, as its host last said. */
export const inside = signal<Record<WorkspaceId, LiveView>>({});
/** The focused column per workspace, for this visit. */
export const activeColumn = signal<Record<WorkspaceId, string>>({});

export const overlay = signal<null | "palette" | "new" | "project" | "keys">(null);
/** The project the new-workspace form starts on. */
export const newIn = signal<ProjectId | null>(null);
export const details = signal(false);
/** The column zoomed to fill the page, for this visit: the strip shows it
 * whole and the list of workspaces folds away until it's put back. */
export const zoomed = signal<{ ws: WorkspaceId; column: string } | null>(null);
/** The workspace visited before the open one, to go back to. */
export const previous = signal<WorkspaceId | null>(null);

/** Something a workspace is asking the person: its new name, whether to
 * delete it or end a column, a column's name, what to find in a column's
 * output, a port to publish, or a column to add. */
export type Question =
  | { kind: "rename" }
  | { kind: "delete" }
  | { kind: "end"; column: string }
  | { kind: "label"; column: string }
  | { kind: "find"; column: string }
  | { kind: "port" }
  /** A column to add; `server` opens straight at the server's command. */
  | { kind: "add-column"; server?: true };

/** One question at a time, held with the workspace that asked it, so it never
 * carries over to another: an armed delete stays with its own workspace. */
const asked = signal<{ ws: WorkspaceId; question: Question } | null>(null);

/** The open workspace's question, if it has one. */
export const question = computed(() => {
  const a = asked.value;
  return a && a.ws === current.value?.id ? a.question : null;
});

export function ask(ws: WorkspaceView, next: Question): void {
  asked.value = { ws: ws.id, question: next };
}

/** Puts a workspace's question away: any of them, or only one kind, so a
 * request that finishes late can't close a question asked since. */
export function settle(ws: WorkspaceView, kind?: Question["kind"]): void {
  const a = asked.value;
  if (a?.ws === ws.id && (kind === undefined || a.question.kind === kind)) asked.value = null;
}

export const isAsking = (kind: Question["kind"]) => question.value?.kind === kind;

// A visit starts fresh: what the last one asked or showed stays with it.
// Passing through the overview between two workspaces still goes from one
// to the other.
let visiting: WorkspaceId | null = null;
let visited: WorkspaceId | null = null;
effect(() => {
  const id = current.value?.id ?? null;
  if (id === visiting) return;
  visiting = id;
  untracked(() => {
    asked.value = null;
    details.value = false;
    zoomed.value = null;
    if (id === null) return;
    if (visited !== null && visited !== id) previous.value = visited;
    visited = id;
  });
});

export interface Flash {
  message: string;
  undo?: () => void;
}

export const flash = signal<Flash | null>(null);
let flashTimer = 0;

export function say(message: string, undo?: () => void): void {
  flash.value = undo ? { message, undo } : { message };
  clearTimeout(flashTimer);
  flashTimer = window.setTimeout(() => (flash.value = null), undo ? 7000 : 4500);
}

export interface Toast {
  id: number;
  workspace: string;
  title: string;
  detail: string;
}

export const toasts = signal<Toast[]>([]);
let toastId = 0;

export function toast(workspace: string, title: string, detail: string): void {
  const id = ++toastId;
  toasts.value = [{ id, workspace, title, detail }, ...toasts.value].slice(0, 3);
  window.setTimeout(() => (toasts.value = toasts.value.filter((t) => t.id !== id)), 6000);
}
