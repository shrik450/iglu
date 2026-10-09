import assert from "node:assert/strict";
import { test } from "node:test";

import type { ColumnSpec } from "../generated/ColumnSpec.ts";
import { moved, nextWidth, shown, stepIndex, widened } from "./layout.ts";

const columns: ColumnSpec[] = [
  { name: "shell", kind: { kind: "shell" }, width: "half" },
  { name: "claude", kind: { kind: "agent", agent: "claude" }, width: "two-thirds" },
  { name: "server", kind: { kind: "server", command: ["npm", "run", "dev"] }, width: "third" },
];

test("widths cycle through every preset", () => {
  assert.deepEqual((["third", "half", "two-thirds", "full"] as const).map(nextWidth), ["half", "two-thirds", "full", "third"]);
});

test("asked-for columns keep their order, unknown until the host says", () => {
  assert.deepEqual(
    shown(columns, undefined).map((c) => [c.name, c.state]),
    [
      ["shell", null],
      ["claude", null],
      ["server", null],
    ],
  );
});

test("missing sessions are ended and unasked ones come last as shells", () => {
  const statuses = [
    { name: "adhoc", state: "adopted" as const, clients: 0 },
    { name: "server", state: "open" as const, clients: 1 },
    { name: "shell", state: "open" as const, clients: 0 },
  ];
  assert.deepEqual(
    shown(columns, statuses).map((c) => [c.name, c.state, c.kind.kind]),
    [
      ["shell", "open", "shell"],
      ["claude", "ended", "agent"],
      ["server", "open", "server"],
      ["adhoc", "adopted", "shell"],
    ],
  );
});

test("moving stops at the edges", () => {
  assert.deepEqual(moved(columns, "shell", 1)?.map((c) => c.name), ["claude", "shell", "server"]);
  assert.equal(moved(columns, "shell", -1), null);
  assert.equal(moved(columns, "server", 1), null);
  assert.equal(moved(columns, "adhoc", 1), null);
  assert.equal(stepIndex(3, 2, 1), 2);
  assert.equal(stepIndex(0, 0, 1), -1);
});

test("widening changes only that column", () => {
  assert.deepEqual(widened(columns, "claude")?.map((c) => c.width), ["half", "full", "third"]);
  assert.equal(widened(columns, "adhoc"), null);
});
