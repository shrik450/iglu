import assert from "node:assert/strict";
import { test } from "node:test";

import { runs, situation } from "./situation.ts";

test("a step that keeps failing says which and why, not that it's still going", () => {
  const stuck = situation({
    phase: "creating",
    condition: { kind: "error", code: "guest_failed", message: "cloning the repository failed: repository not found", at: 0 },
  });
  assert.deepEqual(stuck, {
    kind: "stuck",
    title: "Can't create it yet",
    detail: "Cloning the repository failed: repository not found. iglu keeps trying.",
  });
});

test("being held up by the host isn't the workspace's fault", () => {
  assert.equal(situation({ phase: "starting", condition: { kind: "host_offline", last_seen: null } }).kind, "held");
  const room = situation({ phase: "starting", condition: { kind: "capacity", available: 2 * 1024 ** 3, needed: 4 * 1024 ** 3 } });
  assert.equal(room.kind === "held" && room.detail, "It needs 4.0 GB and the host has 2.0 GB free. It starts when another workspace frees some.");
});

test("without a condition, the phase says it", () => {
  assert.deepEqual(situation({ phase: "creating", condition: null }), { kind: "building", label: "Creating…" });
  assert.equal(situation({ phase: "stopped", condition: null }).kind, "stopped");
  assert.equal(situation({ phase: "frozen", condition: null }).kind, "frozen");
  assert.deepEqual(situation({ phase: "running", condition: null }), { kind: "running" });
});

test("a workspace without agents says what it runs", () => {
  assert.equal(runs([{ name: "shell" }, { name: "server" }]), "shell · server");
});
