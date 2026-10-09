import assert from "node:assert/strict";
import { test } from "node:test";

import { formatRoute, parseRoute } from "./route.ts";

test("every route round-trips", () => {
  for (const path of ["/", "/w/palette", "/p/iglu", "/previews", "/settings"]) assert.equal(formatRoute(parseRoute(path)), path);
});

test("anything unknown is the overview", () => {
  for (const path of ["/nope", "/w/", "/w/Has-Caps", "/w/a/b", `/w/${"x".repeat(64)}`]) {
    assert.deepEqual(parseRoute(path), { view: "overview" });
  }
});
