// What deleting a workspace would lose, in words. Pure.

import type { Unsaved } from "../generated/Unsaved.ts";

/** For closed sets: a new variant makes the call site a type error. */
export function unreachable(value: never): never {
  throw new Error(`unexpected value: ${JSON.stringify(value)}`);
}

/** One of a closed set's keys, from a form's raw string: options are
 * rendered from the same record, so a stale key can't be chosen. */
export function oneOf<K extends string>(labels: Readonly<Record<K, string>>, value: string): K | undefined {
  return (Object.keys(labels) as K[]).find((key) => key === value);
}

export function unsavedText(unsaved: Unsaved): string {
  const files = unsaved.uncommitted === 0 ? null : unsaved.uncommitted === 1 ? "1 uncommitted file" : `${unsaved.uncommitted} uncommitted files`;
  const commits = (() => {
    const unpushed = unsaved.unpushed;
    switch (unpushed.kind) {
      case "none":
        return null;
      case "commits":
        return unpushed.count === 1 ? "1 unpushed commit" : `${unpushed.count} unpushed commits`;
      case "no_upstream":
        return "a branch that was never pushed";
      default:
        return unreachable(unpushed);
    }
  })();
  return [files, commits].filter((part) => part !== null).join(" and ");
}
