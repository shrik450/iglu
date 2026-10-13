// Why iglud ended a terminal's or the browser's connection on purpose, read
// from the close frame's reason. Pure.

import type { Closed } from "../generated/Closed.ts";

/** Every reason iglud gives; a new one is a type error here until it's added. */
const REASONS: Readonly<Record<Closed, true>> = {
  NotRunning: true,
  AccessEnded: true,
  HostFailed: true,
  BrowserNotRunning: true,
  BrowserUnreachable: true,
};

/** The reason iglud gave, or null for none, or for one that isn't iglud's. */
export function closedFor(reason: string): Closed | null {
  return Object.hasOwn(REASONS, reason) ? (reason as Closed) : null;
}
