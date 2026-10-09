import assert from "node:assert/strict";
import { test } from "node:test";

import { parsePorts, startingAgent } from "./project.ts";

test("ports parse from a loose list", () => {
  assert.deepEqual(parsePorts("5173, 3000 3000"), { ports: [3000, 5173] });
  assert.deepEqual(parsePorts("  "), { ports: [] });
  assert.deepEqual(parsePorts("3000, http"), { error: "http isn't a port number." });
  // Out of range is iglu's to refuse, beside the input.
  assert.deepEqual(parsePorts("70000"), { ports: [70000] });
});

test("a new workspace starts the agent iglu would pick", () => {
  const agents = ["claude", "haiku", "scripted"];
  const shell = { kind: { kind: "shell" as const }, width: "half" as const };
  const scripted = { kind: { kind: "agent" as const, agent: "scripted" }, width: "half" as const };
  assert.equal(startingAgent({ agent: "haiku", opening: [scripted] }, agents), "haiku");
  assert.equal(startingAgent({ agent: null, opening: [shell, scripted] }, agents), "scripted");
  assert.equal(startingAgent({ agent: null, opening: [shell] }, agents), "claude");
  // An agent the environment no longer has isn't offered.
  assert.equal(startingAgent({ agent: "codex", opening: [] }, agents), "claude");
  assert.equal(startingAgent({ agent: null, opening: [] }, []), null);
});
