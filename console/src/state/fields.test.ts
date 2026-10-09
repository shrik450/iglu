import assert from "node:assert/strict";
import { test } from "node:test";

import { inputOf } from "./fields.ts";

test("a field is shown by the input its first segment names", () => {
  assert.equal(inputOf("source"), "source");
  assert.equal(inputOf("ports[1]"), "ports");
  assert.equal(inputOf("target.kind"), "target");
  assert.equal(inputOf("opening[0].kind"), "opening");
});
