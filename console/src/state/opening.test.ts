import assert from "node:assert/strict";
import { test } from "node:test";

import { offer } from "./opening.ts";

test("a web address opens in either browser", () => {
  assert.deepEqual(offer("https://github.com/login/device"), { url: "https://github.com/login/device", places: ["here", "workspace"] });
  assert.deepEqual(offer("HTTP://Example.com"), { url: "http://example.com/", places: ["here", "workspace"] });
});

test("the workspace's own addresses and files open only in its browser", () => {
  for (const address of ["http://localhost:3000/callback?code=1", "http://app.localhost/", "http://127.0.0.1:8085/", "http://127.1/", "http://0.0.0.0:5173/", "http://[::1]:8080/", "file:///home/dev/report.html"]) {
    assert.deepEqual(offer(address)?.places, ["workspace"], address);
  }
});

test("anything else isn't offered", () => {
  for (const address of ["javascript:alert(1)", "data:text/html,hi", "mailto:a@b.c", "about:blank", "not a url", ""]) {
    assert.equal(offer(address), null, address);
  }
});
