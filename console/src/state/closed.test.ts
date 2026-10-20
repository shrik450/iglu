import assert from "node:assert/strict";
import { test } from "node:test";

import { closedFor } from "./closed.ts";

test("a close frame's reason is iglud's, or none", () => {
  assert.equal(closedFor("AccessEnded"), "AccessEnded");
  assert.equal(closedFor(""), null);
  assert.equal(closedFor("toString"), null);
  assert.equal(closedFor("the workspace isn't running"), null);
});
