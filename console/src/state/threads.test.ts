import assert from "node:assert/strict";
import { test } from "node:test";

import type { AttentionView } from "../generated/AttentionView.ts";
import { bySession } from "./threads.ts";

const thread = (session: string, key: string) => ({ session, thread: key }) as AttentionView;

test("threads group by session in the order they came", () => {
  const grouped = bySession([thread("claude", "b"), thread("shell", "session"), thread("claude", "a")]);
  assert.deepEqual([...grouped.keys()], ["claude", "shell"]);
  assert.deepEqual(grouped.get("claude")?.map((t) => t.thread), ["b", "a"]);
});
