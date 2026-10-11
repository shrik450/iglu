// What a workspace is called, and the name iglu makes from what a person
// types for a new one.

import type { WorkspaceView } from "../generated/WorkspaceView.ts";

/** What to show for a workspace: its label, or else its name. */
export function called(ws: Pick<WorkspaceView, "name" | "label">): string {
  return ws.label ?? ws.name;
}

/** The longest name made from text, as iglu makes it. */
const MADE_MAX = 40;

/** The name iglu makes from a label, as `names::from_label` does: ASCII
 * letters and digits, lowercased, dashes between, from the first letter.
 * `null` when the text has no letter. iglu adds a number if it's taken. */
export function nameFrom(text: string): string | null {
  let stem = "";
  for (const c of text) {
    if (/^[A-Za-z0-9]$/.test(c)) {
      if (stem === "" && !/^[A-Za-z]$/.test(c)) continue;
      stem += c.toLowerCase();
    } else if (stem !== "" && !stem.endsWith("-")) {
      stem += "-";
    }
  }
  stem = stem.slice(0, MADE_MAX).replace(/-+$/, "");
  return stem === "" ? null : stem;
}

/** The name iglu makes from a branch: from its last part. */
export function nameFromBranch(branch: string): string | null {
  return nameFrom(branch.slice(branch.lastIndexOf("/") + 1));
}
