import assert from "node:assert/strict";
import { test } from "node:test";

import { situation } from "./situation.ts";

test("a step that keeps failing says which and why, not that it's still going", () => {
  const failure = { kind: "host", code: "guest_failed", detail: "cloning the repository failed: repository not found" } as const;
  const stuck = situation({ phase: "creating", condition: { kind: "error", step: "provision", failure, at: 0 } });
  assert.deepEqual(stuck, { kind: "stuck", phase: "creating", step: "provision", failure });
});

test("being held up by the host isn't the workspace's fault", () => {
  assert.equal(situation({ phase: "starting", condition: { kind: "host_offline", last_seen: null } }).kind, "held");
  const room = { kind: "capacity", available: 2 * 1024 ** 3, needed: 4 * 1024 ** 3 } as const;
  assert.deepEqual(situation({ phase: "starting", condition: room }), { kind: "held", condition: room });
});

test("without a condition, the phase says it", () => {
  assert.deepEqual(situation({ phase: "creating", condition: null }), { kind: "building", phase: "creating" });
  assert.equal(situation({ phase: "stopped", condition: null }).kind, "stopped");
  assert.deepEqual(situation({ phase: "freezing", condition: null }), { kind: "frozen", freezing: true });
  assert.deepEqual(situation({ phase: "deleting", condition: null }), { kind: "leaving", deleting: true });
  assert.deepEqual(situation({ phase: "running", condition: null }), { kind: "running" });
});
