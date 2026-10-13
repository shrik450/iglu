// What a workspace is doing, so its card and its view tell the same story.
// messages.tsx says it in words. Pure.

import type { Condition } from "../generated/Condition.ts";
import type { Effect } from "../generated/Effect.ts";
import type { Failure } from "../generated/Failure.ts";
import type { Phase } from "../generated/Phase.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { unreachable } from "./unsaved.ts";

export type Situation =
  /** Being made or started, and going fine. */
  | { kind: "building"; phase: "creating" | "starting" }
  /** A step keeps failing; iglu retries it. */
  | { kind: "stuck"; phase: Phase; step: Effect; failure: Failure }
  /** The runtime gave up on it; only stopping or deleting helps. */
  | { kind: "broken" }
  /** Held up by something outside it, which iglu waits out. */
  | { kind: "held"; condition: Extract<Condition, { kind: "HostOffline" | "Capacity" }> }
  | { kind: "frozen"; freezing: boolean }
  | { kind: "stopped" }
  /** Stopping or being deleted. */
  | { kind: "leaving"; deleting: boolean }
  | { kind: "running" };

export function situation(ws: Pick<WorkspaceView, "phase" | "condition">): Situation {
  const condition = ws.condition;
  if (condition) {
    switch (condition.kind) {
      case "Error":
        return { kind: "stuck", phase: ws.phase, step: condition.step, failure: condition.failure };
      case "RuntimeFailed":
        return { kind: "broken" };
      case "HostOffline":
      case "Capacity":
        return { kind: "held", condition };
      default:
        return unreachable(condition);
    }
  }
  switch (ws.phase) {
    case "creating":
    case "starting":
      return { kind: "building", phase: ws.phase };
    case "freezing":
      return { kind: "frozen", freezing: true };
    case "frozen":
      return { kind: "frozen", freezing: false };
    case "stopping":
      return { kind: "leaving", deleting: false };
    case "stopped":
      return { kind: "stopped" };
    case "deleting":
    case "deleted":
      return { kind: "leaving", deleting: true };
    case "running":
      return { kind: "running" };
    default:
      return unreachable(ws.phase);
  }
}
