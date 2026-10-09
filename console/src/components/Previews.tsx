// A workspace's published ports, as links.

import type { WorkspaceView } from "../generated/WorkspaceView.ts";

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
          title={asleep ? `${route.url} (the workspace is ${ws.phase})` : route.url}
        >
          ↗ <b>{route.name}</b> :{route.port}
        </a>
      ))}
    </>
  );
}
