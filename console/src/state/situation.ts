// What a workspace is doing, in words, so its card and its view tell the same
// story. Pure.

import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { unreachable } from "./unsaved.ts";

export type Situation =
  /** Being made or started, and going fine. */
  | { kind: "building"; label: string }
  /** A step keeps failing; iglu retries it. */
  | { kind: "stuck"; title: string; detail: string }
  /** The runtime gave up on it; only stopping or deleting helps. */
  | { kind: "broken"; title: string; detail: string }
  /** Held up by something outside it, which iglu waits out. */
  | { kind: "held"; title: string; detail: string }
  | { kind: "frozen"; label: string }
  | { kind: "stopped"; label: string }
  /** Stopping or being deleted. */
  | { kind: "leaving"; label: string }
  | { kind: "running" };

/** Bytes as people read memory sizes: "1.5 GB". */
export function gigabytes(bytes: number): string {
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

/** The first letter up, for a message that starts mid-sentence. */
function sentence(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function failing(phase: WorkspaceView["phase"]): string {
  switch (phase) {
    case "creating":
      return "Can't create it yet";
    case "starting":
      return "Can't start it yet";
    case "freezing":
      return "Can't freeze it yet";
    case "stopping":
      return "Can't stop it yet";
    case "deleting":
    case "deleted":
      return "Can't delete it yet";
    case "running":
    case "frozen":
    case "stopped":
      return "Something keeps failing";
    default:
      return unreachable(phase);
  }
}

export function situation(ws: Pick<WorkspaceView, "phase" | "condition">): Situation {
  const condition = ws.condition;
  if (condition) {
    switch (condition.kind) {
      case "error":
        return { kind: "stuck", title: failing(ws.phase), detail: `${sentence(condition.message)}. iglu keeps trying.` };
      case "runtime_failed":
        return { kind: "broken", title: "It's broken", detail: "The host can't run it any more. Stop it to try again, or delete it." };
      case "host_offline":
        return { kind: "held", title: "The host isn't answering", detail: "It carries on when the host is back." };
      case "capacity":
        return {
          kind: "held",
          title: "Waiting for room",
          detail: `It needs ${gigabytes(condition.needed)} and the host has ${gigabytes(condition.available)} free. It starts when another workspace frees some.`,
        };
      default:
        return unreachable(condition);
    }
  }
  switch (ws.phase) {
    case "creating":
      return { kind: "building", label: "Creating…" };
    case "starting":
      return { kind: "building", label: "Starting…" };
    case "freezing":
      return { kind: "frozen", label: "Freezing…" };
    case "frozen":
      return { kind: "frozen", label: "Frozen" };
    case "stopping":
      return { kind: "leaving", label: "Stopping…" };
    case "stopped":
      return { kind: "stopped", label: "Stopped. Its files are kept; its processes ended." };
    case "deleting":
    case "deleted":
      return { kind: "leaving", label: "Deleting…" };
    case "running":
      return { kind: "running" };
    default:
      return unreachable(ws.phase);
  }
}

/** What a workspace without agent threads runs, for its card: "shell · server". */
export function runs(columns: readonly { name: string }[]): string {
  return columns.map((c) => c.name).join(" · ");
}
