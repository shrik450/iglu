import assert from "node:assert/strict";
import { test } from "node:test";

import { parsePorts } from "./project.ts";

test("ports parse from a loose list", () => {
  assert.deepEqual(parsePorts("5173, 3000 3000"), { ports: [3000, 5173] });
  assert.deepEqual(parsePorts("  "), { ports: [] });
  assert.deepEqual(parsePorts("3000, http"), { error: "http isn't a port number." });
  // Out of range is iglu's to refuse, beside the input.
  assert.deepEqual(parsePorts("70000"), { ports: [70000] });
});
