// A workspace's published ports, as links.

import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { m } from "../messages.tsx";

export function Previews({ ws }: { ws: WorkspaceView }) {
  const asleep = ws.phase !== "running";
  return (
    <>
      {ws.routes.map((route) => (
        <a
          key={route.id}
          class={`chip route${asleep ? " asleep" : ""}`}
          href={route.url}
          target="_blank"
          rel="noopener"
          translate={false}
          title={m.preview.title(route.url, asleep ? m.phase(ws.phase) : null)}
        >
          ↗ <b>{route.name}</b> :{route.port}
        </a>
      ))}
    </>
  );
}
