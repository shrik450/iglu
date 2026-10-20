// Every workspace, grouped, with what needs you first.

import { AddEnvironment } from "../components/AddEnvironment.tsx";
import { Card } from "../components/Card.tsx";
import { attentionGlyph, Glyph, Icon, Igloo, WorkspaceLink } from "../components/bits.tsx";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { m } from "../messages.tsx";
import { situation } from "../state/situation.ts";
import { repoLabel } from "../state/groups.ts";
import { collapsed, groups, newIn, overlay, projects, waiting, workspaces } from "../state/store.ts";
import { called } from "../state/naming.ts";
import { unreachable } from "../state/unsaved.ts";

function toggle(key: string): void {
  const next = new Set(collapsed.value);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  collapsed.value = next;
}

/** What's wrong with a workspace in trouble, in a few words. */
function troubleText(ws: WorkspaceView): string {
  const now = situation(ws);
  switch (now.kind) {
    case "stuck":
    case "broken":
    case "held":
      return m.situation.trouble(now).title;
    case "building":
    case "frozen":
    case "stopped":
    case "leaving":
    case "running":
      return "";
    default:
      return unreachable(now);
  }
}

/** What someone new sees: what iglu is, and the first step, right here. */
function Welcome() {
  return (
    <div class="welcome">
      <Igloo />
      <h1>{m.welcome.title}</h1>
      <p class="lede">{m.welcome.lede}</p>
      <ol class="steps">
        <li class="now">
          <b>{m.welcome.environment}</b>
          <span>{m.welcome.environmentAbout}</span>
          <AddEnvironment first />
        </li>
        <li>
          <b>{m.welcome.project}</b>
          <span>{m.welcome.projectAbout}</span>
        </li>
        <li>
          <b>{m.welcome.workspace}</b>
          <span>{m.welcome.workspaceAbout}</span>
        </li>
      </ol>
    </div>
  );
}

export function Overview() {
  if (projects.value.length === 0) return <Welcome />;
  return (
    <div class="overview">
      <h1 class="sr-only">{m.overview.heading}</h1>
      {waiting.value.length ? (
        <ul class="needs-strip" aria-label={m.overview.waiting}>
          {waiting.value.map((ws) => (
            <li key={ws.id}>
              <WorkspaceLink ws={ws} class="nchip">
                <Glyph kind={ws.condition ? "trouble" : attentionGlyph(ws.attention)} />
                <span class="w" translate={false}>
                  {called(ws)}
                </span>
                <span class="t">{ws.condition ? troubleText(ws) : (ws.attention?.summary ?? "")}</span>
              </WorkspaceLink>
            </li>
          ))}
        </ul>
      ) : null}
      {workspaces.value.length === 0 ? (
        <div class="first-ws">
          <p>{m.overview.ready}</p>
          <button type="button" class="btn primary" onClick={() => (overlay.value = "new")}>
            {m.overview.newWorkspace}
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
                <button type="button" class="chev" aria-expanded={!closed} aria-label={closed ? m.overview.expand(group.label) : m.overview.collapse(group.label)} onClick={() => toggle(group.key)}>
                  <Icon name="chevron" size={12} />
                </button>
                <h2 translate={false}>
                  <a
                    href={`/p/${group.label}`}
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
                  aria-label={m.overview.newIn(group.label)}
                  title={m.overview.newIn(group.label)}
                  onClick={() => {
                    newIn.value = group.project.id;
                    overlay.value = "new";
                  }}
                >
                  <Icon name="plus" size={14} />
                </button>
                {closed ? (
                  <span class="sum">
                    {m.overview.count(group.workspaces.length)}
                    {need ? (
                      <>
                        {" · "}
                        <b>{m.overview.needYou(need)}</b>
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
        {m.overview.newProject}
      </button>
    </div>
  );
}
