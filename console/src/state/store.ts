// The console's state, as signals. Views read them; actions in actions.ts write them.

import { computed, effect, signal } from "@preact/signals";

import type { ColumnStatus } from "../generated/ColumnStatus.ts";
import type { LiveView } from "../generated/LiveView.ts";
import type { EnvironmentView } from "../generated/EnvironmentView.ts";
import type { Me } from "../generated/Me.ts";
import type { ProjectId } from "../generated/ProjectId.ts";
import type { ProjectView } from "../generated/ProjectView.ts";
import type { WorkspaceId } from "../generated/WorkspaceId.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { groupByProject } from "./groups.ts";
import { type Look, loadCollapsed, loadLook, saveCollapsed, saveLook } from "./prefs.ts";
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

/** The workspace the URL names, if it exists. */
export const current = computed(() => {
  const r = route.value;
  return r.view === "workspace" ? (workspaces.value.find((ws) => ws.name === r.name) ?? null) : null;
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

export const collapsed = signal<ReadonlySet<string>>(loadCollapsed());
effect(() => saveCollapsed(collapsed.value));

/** Each workspace's columns as its host last reported them. */
export const columnStates = signal<Record<WorkspaceId, ColumnStatus[]>>({});
/** What's going on inside each workspace, as its host last said. */
export const inside = signal<Record<WorkspaceId, LiveView>>({});
/** The focused column per workspace, for this visit. */
export const activeColumn = signal<Record<WorkspaceId, string>>({});

export const overlay = signal<null | "palette" | "new" | "project">(null);
/** The project the new-workspace form starts on. */
export const newIn = signal<ProjectId | null>(null);
export const details = signal(false);
export const renaming = signal(false);
export const confirming = signal<null | "delete">(null);
/** The column whose session is about to be ended, awaiting confirmation. */
export const closing = signal<string | null>(null);
export const addingPort = signal(false);
/** The add-column menu. */
export const addingColumn = signal(false);
export const filter = signal("");

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
