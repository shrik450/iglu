// Grouping the server-ordered workspace list by project for display. Groups
// follow the server's project order, alphabetical, so they don't jump around
// as urgency changes; within a group the server's standing order is kept.

import type { ProjectView } from "../generated/ProjectView.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";

export interface Group {
  key: string;
  label: string;
  project: ProjectView;
  workspaces: WorkspaceView[];
}

/** `https://github.com/acme/app.git` and `git@github.com:acme/app.git` both read `acme/app`. */
export function repoLabel(repo: string): string {
  const path = repo.replace(/^[a-z]+:\/\/[^/]+\//, "").replace(/^[^@/]+@[^:]+:/, "").replace(/\.git$/, "").replace(/\/+$/, "");
  const parts = path.split("/").filter(Boolean);
  return parts.slice(-2).join("/") || repo;
}

/**
 * Every project, empty ones too so there's somewhere to start, with its
 * workspaces. A snapshot always carries a workspace's project with it.
 */
export function groupByProject(projects: readonly ProjectView[], workspaces: readonly WorkspaceView[]): Group[] {
  return projects.map((project) => ({
    key: project.id,
    label: project.name,
    project,
    workspaces: workspaces.filter((ws) => ws.project === project.id),
  }));
}
