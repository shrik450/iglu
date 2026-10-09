// A workspace's published ports, as links.

import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { ONLY_YOU } from "./CopyLink.tsx";

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
          title={`${route.url}${asleep ? ` (the workspace is ${ws.phase})` : ""}. ${ONLY_YOU}`}
        >
          ↗ <b>{route.name}</b> :{route.port}
        </a>
      ))}
    </>
  );
}
