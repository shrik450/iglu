// A workspace's threads by column. Pure.

import type { AttentionView } from "../generated/AttentionView.ts";

/** Each session's threads, keeping the server's most-urgent-first order. */
export function bySession(threads: readonly AttentionView[]): Map<string, AttentionView[]> {
  const sessions = new Map<string, AttentionView[]>();
  for (const thread of threads) {
    const list = sessions.get(thread.session);
    if (list) list.push(thread);
    else sessions.set(thread.session, [thread]);
  }
  return sessions;
}
