import assert from "node:assert/strict";
import { test } from "node:test";

import { unsavedText } from "./unsaved.ts";

test("unsaved work reads as what would be lost", () => {
  assert.equal(unsavedText({ uncommitted: 3, unpushed: { kind: "commits", count: 2 } }), "3 uncommitted files and 2 unpushed commits");
  assert.equal(unsavedText({ uncommitted: 1, unpushed: { kind: "none" } }), "1 uncommitted file");
  assert.equal(unsavedText({ uncommitted: 0, unpushed: { kind: "no_upstream" } }), "a branch that was never pushed");
});
