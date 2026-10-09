// Every workspace, grouped, with what needs you first.

import { AddEnvironment } from "../components/AddEnvironment.tsx";
import { Card } from "../components/Card.tsx";
import { Glyph, Icon, Igloo, WorkspaceLink, attentionGlyph } from "../components/bits.tsx";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { situation } from "../state/situation.ts";
import { repoLabel } from "../state/groups.ts";
import { collapsed, groups, navigate, newIn, overlay, projects, waiting, workspaces } from "../state/store.ts";

function toggle(key: string): void {
  const next = new Set(collapsed.value);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  collapsed.value = next;
}

/** What's wrong with a workspace in trouble, in a few words. */
function troubleText(ws: WorkspaceView): string {
  const now = situation(ws);
  return "title" in now ? now.title : "";
}

/** What someone new sees: what iglu is, and the first step, right here. */
function Welcome() {
  return (
    <div class="welcome">
      <Igloo />
      <h1>Welcome to iglu</h1>
      <p class="lede">Each piece of work gets its own machine: a NixOS workspace whose terminals, agents and previews live in your browser.</p>
      <ol class="steps">
        <li class="now">
          <b>Add an environment</b>
          <span>The NixOS system your workspaces run, from a flake that uses iglu's workspace module.</span>
          <AddEnvironment first />
        </li>
        <li>
          <b>Add a project</b>
          <span>A repository to work on, and how its workspaces start. You can skip this: general has no repository.</span>
        </li>
        <li>
          <b>Start a workspace</b>
          <span>Say what to work on, and an agent starts on it.</span>
        </li>
      </ol>
    </div>
  );
}

export function Overview() {
  if (projects.value.length === 0) return <Welcome />;
  return (
    <div class="overview">
      <h1 class="sr-only">Workspaces</h1>
      {waiting.value.length ? (
        <ul class="needs-strip" aria-label="Waiting on you">
          {waiting.value.map((ws) => (
            <li key={ws.id}>
              <WorkspaceLink ws={ws} class="nchip">
                <Glyph kind={ws.condition ? "trouble" : attentionGlyph(ws.attention)} />
                <span class="w" translate={false}>
                  {ws.name}
                </span>
                <span class="t">{ws.condition ? troubleText(ws) : (ws.attention?.summary ?? "")}</span>
              </WorkspaceLink>
            </li>
          ))}
        </ul>
      ) : null}
      {workspaces.value.length === 0 ? (
        <div class="first-ws">
          <p>
            <b>Ready.</b> Start a workspace: name what to work on, and an agent starts on it.
          </p>
          <button type="button" class="btn primary" onClick={() => (overlay.value = "new")}>
            New workspace
          </button>
        </div>
      ) : null}
      <div class="groups">
        {groups.value.map((group) => {
          const closed = collapsed.value.has(group.key);
          const need = group.workspaces.filter((ws) => ws.needs_you).length;
          return (
            <section key={group.key} aria-label={group.label} class={group.workspaces.length === 0 ? "g-empty" : undefined}>
              <header class="g-head">
                <button type="button" class="chev" aria-expanded={!closed} aria-label={`${closed ? "Expand" : "Collapse"} ${group.label}`} onClick={() => toggle(group.key)}>
                  <Icon name="chevron" size={12} />
                </button>
                <h2 translate={false}>
                  <a
                    href={`/p/${group.label}`}
                    onClick={(e) => {
                      if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
                      e.preventDefault();
                      navigate({ view: "project", name: group.label });
                    }}
                  >
                    {group.label}
                  </a>
                </h2>
                {group.project.repo ? (
                  <span class="repo" translate={false}>
                    {repoLabel(group.project.repo)}
                  </span>
                ) : null}
                <button
                  type="button"
                  class="g-add"
                  aria-label={`New workspace in ${group.label}`}
                  title={`New workspace in ${group.label}`}
                  onClick={() => {
                    newIn.value = group.project.id;
                    overlay.value = "new";
                  }}
                >
                  <Icon name="plus" size={14} />
                </button>
                {closed ? (
                  <span class="sum">
                    {group.workspaces.length}
                    {need ? (
                      <>
                        {" · "}
                        <b>{need} waiting</b>
                      </>
                    ) : null}
                  </span>
                ) : null}
              </header>
              {closed || group.workspaces.length === 0 ? null : (
                <div class="grid">
                  {group.workspaces.map((ws) => (
                    <Card key={ws.id} ws={ws} />
                  ))}
                </div>
              )}
            </section>
          );
        })}
      </div>
      <button type="button" class="btn new-project" onClick={() => (overlay.value = "project")}>
        New project
      </button>
    </div>
  );
}
