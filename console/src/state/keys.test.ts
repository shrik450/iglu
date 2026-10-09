import assert from "node:assert/strict";
import { test } from "node:test";

import { type KeyInput, resolve } from "./keys.ts";

const key = (code: string, extra: Partial<KeyInput> = {}): KeyInput => ({
  key: code.startsWith("Key") ? code.slice(3).toLowerCase() : code,
  code,
  alt: false,
  meta: false,
  ctrl: false,
  shift: false,
  focus: "page",
  ...extra,
});

test("plain letters act on the page", () => {
  assert.deepEqual(resolve(key("KeyJ")), { kind: "workspace", step: 1 });
  assert.deepEqual(resolve(key("KeyN")), { kind: "new" });
  assert.deepEqual(resolve(key("KeyL", { shift: true })), { kind: "move-column", step: 1 });
});

test("typing in a terminal or a field never triggers a shortcut", () => {
  for (const focus of ["terminal", "field"] as const) {
    assert.equal(resolve(key("KeyJ", { focus })), null);
    assert.equal(resolve(key("Escape", { focus })), null);
  }
});

test("Option chords work from a terminal, not from a field", () => {
  assert.deepEqual(resolve(key("KeyH", { alt: true, focus: "terminal" })), { kind: "column", step: -1 });
  assert.deepEqual(resolve(key("KeyN", { alt: true, focus: "terminal" })), { kind: "next-waiting" });
  assert.equal(resolve(key("KeyH", { alt: true, focus: "field" })), null);
});

test("Cmd-K opens search anywhere; other Cmd chords are left to the browser", () => {
  for (const focus of ["page", "terminal", "field"] as const) {
    assert.deepEqual(resolve(key("KeyK", { meta: true, focus })), { kind: "palette" });
  }
  assert.equal(resolve(key("KeyC", { meta: true })), null);
});
