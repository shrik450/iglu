// How much a workspace wants the user. Mirrors the server's urgency order so
// the sidebar and the server agree on what "needs you" means.

import type { Attention, Workspace } from "./api";

export function urgency(attention: Attention | null): number {
  if (attention === null) return 0;
  switch (attention.state) {
    case "waiting":
      return 6;
    case "done":
      return attention.seen === "unseen" ? 5 : 3;
    case "working":
      return 4;
    case "idle":
      return 2;
    case "exited":
      return 1;
  }
}

export function needsYou(ws: Workspace): boolean {
  return urgency(ws.attention) >= 5 || ws.condition?.kind === "error" || ws.condition?.kind === "runtime_failed";
}

export function byAttention(a: Workspace, b: Workspace): number {
  const difference = urgency(b.attention) - urgency(a.attention);
  if (difference !== 0) return difference;
  return (b.attention?.updated_at ?? b.created_at) - (a.attention?.updated_at ?? a.created_at);
}

export function glyph(ws: Workspace): string {
  if (ws.condition?.kind === "error" || ws.condition?.kind === "runtime_failed") return "!";
  switch (ws.phase) {
    case "creating":
    case "starting":
    case "freezing":
    case "stopping":
    case "deleting":
      return "…";
    case "frozen":
      return "❄";
    case "stopped":
    case "deleted":
      return "○";
    case "running":
      break;
  }
  switch (ws.attention?.state) {
    case "waiting":
      return "?";
    case "done":
      return ws.attention.seen === "unseen" ? "✓" : "·";
    case "working":
      return "●";
    case "idle":
    case "exited":
    case undefined:
      return "·";
  }
}
