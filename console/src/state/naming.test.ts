import assert from "node:assert/strict";
import { test } from "node:test";

import { called, nameFrom, nameFromBranch } from "./naming.ts";

test("a label shows in place of the name", () => {
  assert.equal(called({ name: "more-ui-tweaks", label: "More UI tweaks" }), "More UI tweaks");
  assert.equal(called({ name: "more-ui-tweaks", label: null }), "more-ui-tweaks");
});

// The same cases as names::tests in iglu-domain, which makes the real name.
test("names are made from letters and digits", () => {
  assert.equal(nameFrom("More UI Tweaks"), "more-ui-tweaks");
  assert.equal(nameFrom("  fix: the login (again)! "), "fix-the-login-again");
  assert.equal(nameFrom("2024 review"), "review");
  assert.equal(nameFrom("Café ☕ notes"), "caf-notes");
  assert.equal(nameFrom("修正"), null);
  const long = nameFrom("word ".repeat(20)) ?? "";
  assert.ok(long.length <= 40 && !long.endsWith("-"), long);
});

test("branches name by their last part", () => {
  assert.equal(nameFromBranch("su/login-fix"), "login-fix");
  assert.equal(nameFromBranch("main"), "main");
  assert.equal(nameFromBranch("feature/JIRA-123_Thing"), "jira-123-thing");
});
