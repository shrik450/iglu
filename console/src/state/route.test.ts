import assert from "node:assert/strict";
import { test } from "node:test";

import { formatRoute, parseRoute } from "./route.ts";

test("every route round-trips", () => {
  for (const path of ["/", "/w/palette", "/p/iglu", "/p/iglu/settings", "/previews", "/settings"]) assert.equal(formatRoute(parseRoute(path) ?? { view: "overview" }), path);
});

test("anything else isn't the console's", () => {
  for (const path of ["/nope", "/w/", "/w/Has-Caps", "/w/a/b", `/w/${"x".repeat(64)}`, "/auth/login", "/v1/me"]) {
    assert.equal(parseRoute(path), null);
  }
});
