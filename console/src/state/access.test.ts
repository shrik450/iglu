import assert from "node:assert/strict";
import { test } from "node:test";

import { holds, named, toggled } from "./access.ts";

test("a permission is given and taken away, and an emptied workspace drops out", () => {
  const given = toggled([], "b", "view");
  assert.deepEqual(given, [{ workspace: "b", permissions: ["view"] }]);
  const more = toggled(given, "b", "read_output");
  assert.ok(holds(more, "b", "read_output"));
  assert.deepEqual(toggled(toggled(more, "b", "view"), "b", "read_output"), []);
});

test("other workspaces' grants are left as they were", () => {
  const access = [
    { workspace: "a", permissions: ["view" as const] },
    { workspace: "b", permissions: ["view" as const] },
  ];
  assert.deepEqual(toggled(access, "b", "view"), [{ workspace: "a", permissions: ["view"] }]);
});

test("the workspace itself comes first, even with nothing granted on it", () => {
  assert.deepEqual(named([{ workspace: "b", permissions: ["view"] }], "a"), ["a", "b"]);
  assert.deepEqual(named([{ workspace: "a", permissions: ["view"] }], "a"), ["a"]);
});
