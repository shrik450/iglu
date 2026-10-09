import assert from "node:assert/strict";
import { test } from "node:test";

import type { ProjectView } from "../generated/ProjectView.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { groupByProject, repoLabel } from "./groups.ts";

const project = (id: string, name: string) => ({ id, name }) as ProjectView;
const ws = (name: string, projectId: string) => ({ name, project: projectId }) as WorkspaceView;

test("repo labels are owner/name whatever the URL form", () => {
  assert.equal(repoLabel("https://github.com/acme/app.git"), "acme/app");
  assert.equal(repoLabel("git@github.com:acme/app.git"), "acme/app");
  assert.equal(repoLabel("ssh://git@10.0.0.2/srv/git/app.git"), "git/app");
});

test("groups follow the projects and keep the server's order inside", () => {
  const groups = groupByProject(
    [project("p1", "app"), project("p2", "general"), project("p3", "zed")],
    [ws("waiting", "p3"), ws("first", "p1"), ws("second", "p3"), ws("third", "p1")],
  );
  assert.deepEqual(groups.map((g) => g.label), ["app", "general", "zed"]);
  assert.deepEqual(groups.map((g) => g.workspaces.map((w) => w.name)), [["first", "third"], [], ["waiting", "second"]]);
});
