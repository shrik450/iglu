import assert from "node:assert/strict";
import { test } from "node:test";

import { holds, named, toggled } from "./access.ts";

test("a permission is given and taken away, and an emptied workspace drops out", () => {
  const given = toggled([], "b", "view");
  assert.deepEqual(given, [{ workspace: "b", permissions: ["view"] }]);
  const more = toggled(given, "b", "read_output");
  assert.ok(holds(more, "b", "read_output"));
  assert.deepEqual(toggled(more, "b", "read_output"), given);
});

test("every permission includes seeing, so taking that away takes the rest", () => {
  assert.deepEqual(toggled([], "b", "send_input"), [{ workspace: "b", permissions: ["view", "send_input"] }]);
  const both = toggled([], "b", "send_input");
  assert.deepEqual(toggled(both, "b", "view"), []);
  assert.deepEqual(toggled(both, "b", "send_input"), [{ workspace: "b", permissions: ["view"] }]);
});

test("other workspaces' grants are left as they were", () => {
  const access = [
    { workspace: "a", permissions: ["view" as const] },
    { workspace: "b", permissions: ["view" as const] },
  ];
  assert.deepEqual(toggled(access, "b", "view"), [{ workspace: "a", permissions: ["view"] }]);
  assert.deepEqual(toggled(access, "a", "operate")[0], { workspace: "b", permissions: ["view"] });
});

test("the workspace itself comes first, even with nothing granted on it", () => {
  assert.deepEqual(named([{ workspace: "b", permissions: ["view"] }], "a"), ["a", "b"]);
  assert.deepEqual(named([{ workspace: "a", permissions: ["view"] }], "a"), ["a"]);
});
