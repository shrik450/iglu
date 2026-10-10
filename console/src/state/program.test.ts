import assert from "node:assert/strict";
import { test } from "node:test";

import { programTitle } from "./program.ts";

test("a program's title shows as plain text", () => {
  assert.equal(programTitle("  vim\tnotes.md  "), "vim notes.md");
  assert.equal(programTitle("evil‮txt.exe\u0007"), "evil txt.exe");
  assert.equal(programTitle(""), "");
  const long = programTitle("x".repeat(500));
  assert.equal(long.length, 120);
  assert.ok(long.endsWith("…"));
});
