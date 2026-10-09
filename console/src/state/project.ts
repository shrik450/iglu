// Reading a project's settings form, and what a project starts with. Pure.

import type { ProjectView } from "../generated/ProjectView.ts";

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

/** The agent a new workspace starts with a prompt when none is picked, as
 * iglu chooses it: the project's, else the first agent its opening has, else
 * the environment's first. Null when the environment has none of them. */
export function startingAgent(project: Pick<ProjectView, "agent" | "opening">, agents: readonly string[]): string | null {
  const opening = project.opening.flatMap((t) => (t.kind.kind === "agent" ? [t.kind.agent] : []));
  for (const candidate of [project.agent, ...opening, agents[0]]) {
    if (candidate && agents.includes(candidate)) return candidate;
  }
  return null;
}
