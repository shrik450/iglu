// The snapshot stream. iglud sends everything the console shows, whole, on
// every change; EventSource reconnects on its own after a drop.

import { batch } from "@preact/signals";

import type { Snapshot } from "../generated/Snapshot.ts";
import { environments, live, me, outdated, projects, workspaces } from "../state/store.ts";
import { noticeChanges } from "../notify.ts";
import { api } from "./client.ts";

export function watch(): void {
  const source = new EventSource("/v1/events");
  source.onopen = () => (live.value = true);
  source.onerror = () => {
    live.value = false;
    // A refused reconnect, such as an ended session's 401, closes the stream
    // for good. Asking who's signed in sends an ended session to sign in;
    // anything else is worth another try.
    if (source.readyState === EventSource.CLOSED) {
      window.setTimeout(() => {
        void api
          .me()
          .catch(() => undefined)
          .then(() => watch());
      }, 5000);
    }
  };
  source.onmessage = (event: MessageEvent<string>) => {
    // Only the boot ID is read before it's known to be this page's iglu.
    const snapshot = JSON.parse(event.data) as Snapshot;
    if (snapshot.boot !== me.value?.boot) {
      outdated.value = true;
      source.close();
      return;
    }
    noticeChanges(workspaces.value, snapshot.workspaces);
    batch(() => {
      workspaces.value = snapshot.workspaces;
      projects.value = snapshot.projects;
      environments.value = snapshot.environments;
    });
  };
}
