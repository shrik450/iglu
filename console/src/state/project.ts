// Reading a project's settings form, and what a project starts with. Pure.

import type { ColumnKind } from "../generated/ColumnKind.ts";
import type { ProjectView } from "../generated/ProjectView.ts";
import { unreachable } from "./unsaved.ts";

/** "3000, 5173" as numbers, or the part that isn't one. Whether each is a
 * port is iglu's to say. */
export function parsePorts(text: string): { ports: number[] } | { error: string } {
  const parts = text.split(/[\s,]+/).filter(Boolean);
  const ports: number[] = [];
  for (const part of parts) {
    const port = Number(part);
    if (!/^\d+$/.test(part)) return { error: `${part} isn't a port number.` };
    if (!ports.includes(port)) ports.push(port);
  }
  return { ports: ports.sort((a, b) => a - b) };
}

/** What a kind of column is called before it has a name. */
export function kindText(kind: ColumnKind): string {
  switch (kind.kind) {
    case "shell":
      return "Shell";
    case "browser":
      return "Browser";
    default:
      return unreachable(kind);
  }
}

/** The columns a new workspace in the project opens with. */
export function opensWith(project: Pick<ProjectView, "opening">): string[] {
  return project.opening.map((t) => t.name ?? kindText(t.kind));
}
