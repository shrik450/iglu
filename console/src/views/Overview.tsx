// Every workspace, grouped, with what needs you first.

import { Card } from "../components/Card.tsx";
import { Glyph, WorkspaceLink, attentionGlyph } from "../components/bits.tsx";
import { repoLabel } from "../state/groups.ts";
import { collapsed, groups, navigate, newIn, overlay, projects, waiting } from "../state/store.ts";

function toggle(key: string): void {
  const next = new Set(collapsed.value);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  collapsed.value = next;
}

export function Overview() {
  if (projects.value.length === 0) {
    return (
      <div class="empty-state">
        <p class="empty">Add an environment to begin.</p>
        <a class="btn primary" href="/settings" onClick={(e) => {
          if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
          e.preventDefault();
          navigate({ view: "settings" });
        }}>
          Settings
        </a>
      </div>
    );
  }
  return (
    <>
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
                <span class="t">{ws.attention?.summary ?? ""}</span>
              </WorkspaceLink>
            </li>
          ))}
        </ul>
      ) : null}
      <div class="groups">
        {groups.value.map((group) => {
          const closed = collapsed.value.has(group.key);
          const need = group.workspaces.filter((ws) => ws.needs_you).length;
          return (
            <section key={group.key} aria-label={group.label} class={group.workspaces.length === 0 ? "g-empty" : undefined}>
              <header class="g-head">
                <button type="button" class="chev" aria-expanded={!closed} aria-label={`${closed ? "Expand" : "Collapse"} ${group.label}`} onClick={() => toggle(group.key)}>
                  {closed ? "▸" : "▾"}
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
                  +
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
    </>
  );
}
