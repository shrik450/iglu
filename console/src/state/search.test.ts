import assert from "node:assert/strict";
import { test } from "node:test";

import { closeness, search } from "./search.ts";

const labels = (items: { label: string }[]) => items.map((item) => item.label);

test("a workspace's exact name comes before commands that mention it", () => {
  const items = [{ label: "Rename api-fix" }, { label: "Shell in api-fix" }, { label: "api-fix" }, { label: "api-fix-two" }];
  assert.deepEqual(labels(search("api-fix", items)), ["api-fix", "api-fix-two", "Rename api-fix", "Shell in api-fix"]);
});

test("closer matches come first, and equals keep their order", () => {
  const items = [
    { label: "scattered: s-e-t" },
    { label: "Notes", sub: "settings for later" },
    { label: "Presets" },
    { label: "Open settings" },
    { label: "Settings and secrets" },
    { label: "Settings" },
    { label: "Keyboard settings" },
  ];
  assert.deepEqual(labels(search("Settings", items)), [
    "Settings",
    "Settings and secrets",
    "Open settings",
    "Keyboard settings",
    "Notes",
  ]);
  assert.deepEqual(labels(search("set", items)), [
    "Settings and secrets",
    "Settings",
    "Open settings",
    "Keyboard settings",
    "Presets",
    "Notes",
    "scattered: s-e-t",
  ]);
});

test("each kind of match", () => {
  assert.equal(closeness("API", "api"), "name");
  assert.equal(closeness(" ap ", "api"), "start");
  assert.equal(closeness("fix", "api-fix"), "word");
  assert.equal(closeness("pi", "api"), "inside");
  assert.equal(closeness("main", "api", "⎇ main"), "beside");
  assert.equal(closeness("am", "api", "⎇ main"), "scattered");
  assert.equal(closeness("zz", "api", "⎇ main"), null);
});

test("an empty search shows everything as it was", () => {
  const items = [{ label: "b" }, { label: "a" }];
  assert.deepEqual(labels(search("  ", items)), ["b", "a"]);
});
