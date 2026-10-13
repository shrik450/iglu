// Every published port, across workspaces.

import { lookOf, WorkspaceLink } from "../components/bits.tsx";
import { CopyLink } from "../components/CopyLink.tsx";
import { m } from "../messages.tsx";
import { groups } from "../state/store.ts";
import { called } from "../state/naming.ts";

export function PreviewsView() {
  const shown = groups.value.map((group) => ({ group, workspaces: group.workspaces.filter((ws) => ws.routes.length > 0) })).filter((g) => g.workspaces.length > 0);
  if (shown.length === 0) return <div class="empty-state"><p class="empty">{m.previews.none}</p></div>;
  return (
    <div class="pv">
      <h1 class="sr-only">{m.previews.heading}</h1>
      <p class="pv-who">{m.previews.who}</p>
      {shown.map(({ group, workspaces }) => (
        <section key={group.key} aria-label={group.label}>
          <h2 translate={false}>{group.label}</h2>
          {workspaces.flatMap((ws) =>
            ws.routes.map((route) => {
              const look = lookOf(ws.phase);
              const state = look === "awake" ? "" : look === "frozen" ? "asleep" : "off";
              return (
                <div class="pv-row" key={route.id}>
                  <span class={`dot ${state}`} aria-hidden="true" />
                  <a class="url" href={route.url} target="_blank" rel="noopener" translate={false}>
                    {route.url.replace(/^https:\/\//, "")}
                  </a>
                  <span class="to">
                    → <b translate={false}>{called(ws)}</b> :{route.port}
                    {state ? <span class="state"> {m.phase(ws.phase)}</span> : null}
                  </span>
                  <span class="pv-acts">
                    <CopyLink url={route.url} name={route.name} />
                    <WorkspaceLink ws={ws} class="btn" label={m.previews.openLabel(called(ws))}>
                      {m.previews.open}
                    </WorkspaceLink>
                  </span>
                </div>
              );
            }),
          )}
        </section>
      ))}
    </div>
  );
}
