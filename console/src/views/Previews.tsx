// Every published port, across workspaces.

import { lookOf, WorkspaceLink } from "../components/bits.tsx";
import { CopyLink } from "../components/CopyLink.tsx";
import { groups } from "../state/store.ts";

export function PreviewsView() {
  const shown = groups.value.map((group) => ({ group, workspaces: group.workspaces.filter((ws) => ws.routes.length > 0) })).filter((g) => g.workspaces.length > 0);
  if (shown.length === 0) return <div class="empty-state"><p class="empty">Nothing is published.</p></div>;
  return (
    <div class="pv">
      <h1 class="sr-only">Previews</h1>
      <p class="pv-who">Only you can open your previews, signed in to iglu.</p>
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
                    → <b translate={false}>{ws.name}</b> :{route.port}
                    {state ? <span class="state"> {ws.phase}</span> : null}
                  </span>
                  <span class="pv-acts">
                    <CopyLink url={route.url} name={route.name} />
                    <WorkspaceLink ws={ws} class="btn" label={`Open ${ws.name}`}>
                      Open workspace
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
