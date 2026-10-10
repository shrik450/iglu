import assert from "node:assert/strict";
import { test } from "node:test";

import { pastedPath } from "./paste.ts";

test("a plain path pastes as it is, and any other is quoted", () => {
  assert.equal(pastedPath("/home/dev/.cache/iglu/pasted/image-2.png"), "/home/dev/.cache/iglu/pasted/image-2.png");
  assert.equal(pastedPath("/Users/a b/shot.png"), "'/Users/a b/shot.png'");
  assert.equal(pastedPath("/tmp/it's.png"), "'/tmp/it'\\''s.png'");
});
