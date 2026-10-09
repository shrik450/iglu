import assert from "node:assert/strict";
import { test } from "node:test";

import type { ColumnSpec } from "../generated/ColumnSpec.ts";
import { inView, moved, nextWidth, placed, scrollTarget, shown, stepIndex, titleOf, widened } from "./layout.ts";

const columns: ColumnSpec[] = [
  { name: "shell", kind: { kind: "shell" }, width: "half", label: null },
  { name: "claude", kind: { kind: "agent", agent: "claude" }, width: "two-thirds", label: null },
  { name: "server", kind: { kind: "server", command: ["npm", "run", "dev"] }, width: "third", label: "preview" },
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

test("the focused column scrolls wholly into view, with a sliver of the next one", () => {
  // Three 600px columns with 10px gaps in a 1000px view.
  const spans = [0, 610, 1220].map((left) => ({ left, width: 600 }));
  assert.equal(scrollTarget(spans, 0, 1000, 0, 48), 0);
  // Moving right: the second column's right edge, plus a peek of the third.
  assert.equal(scrollTarget(spans, 1, 1000, 0, 48), 258);
  // The last has nothing after it, so it sits against the end.
  assert.equal(scrollTarget(spans, 2, 1000, 258, 48), 820);
  // Moving left again: the first column's left edge, which is the start.
  assert.equal(scrollTarget(spans, 0, 1000, 820, 48), 0);
  // Already in view: nothing moves.
  assert.equal(scrollTarget(spans, 1, 1000, 300, 48), 300);
});

test("a column too wide to show with a peek starts at the view's edge", () => {
  const spans = [
    { left: 0, width: 500 },
    { left: 510, width: 980 },
    { left: 1500, width: 500 },
  ];
  assert.equal(scrollTarget(spans, 1, 1000, 0, 48), 510);
});

test("a column is in view only when all of it is", () => {
  const spans = [0, 610, 1220].map((left) => ({ left, width: 600 }));
  assert.deepEqual([...inView(spans, 1000, 0)], [0]);
  assert.deepEqual([...inView(spans, 1000, 258)], [1]);
  assert.deepEqual([...inView(spans, 1300, 0)], [0, 1]);
});

test("a column shows its label, or its session name when it has none", () => {
  assert.deepEqual(shown(columns, undefined).map(titleOf), ["shell", "claude", "preview"]);
});

test("a column dropped beside another goes there, and a drop that changes nothing is nothing", () => {
  const names = (list: ColumnSpec[] | null) => list?.map((c) => c.name) ?? null;
  assert.deepEqual(names(placed(columns, "server", "shell", false)), ["server", "shell", "claude"]);
  assert.deepEqual(names(placed(columns, "shell", "server", true)), ["claude", "server", "shell"]);
  assert.deepEqual(names(placed(columns, "shell", "server", false)), ["claude", "shell", "server"]);
  assert.equal(placed(columns, "shell", "claude", false), null);
  assert.equal(placed(columns, "claude", "shell", true), null);
  assert.equal(placed(columns, "shell", "shell", true), null);
  assert.equal(placed(columns, "adhoc", "shell", true), null);
});
