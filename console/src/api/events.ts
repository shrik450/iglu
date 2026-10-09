// The snapshot stream. iglud sends everything the console shows, whole, on
// every change; EventSource reconnects on its own.

import { batch } from "@preact/signals";

import type { Snapshot } from "../generated/Snapshot.ts";
import { environments, live, projects, workspaces } from "../state/store.ts";
import { noticeChanges } from "../notify.ts";

export function watch(): EventSource {
  const source = new EventSource("/v1/events");
  source.onopen = () => (live.value = true);
  source.onerror = () => (live.value = false);
  source.onmessage = (event: MessageEvent<string>) => {
    const snapshot = JSON.parse(event.data) as Snapshot;
    noticeChanges(workspaces.value, snapshot.workspaces);
    batch(() => {
      workspaces.value = snapshot.workspaces;
      projects.value = snapshot.projects;
      environments.value = snapshot.environments;
    });
  };
  return source;
}
