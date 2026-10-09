import assert from "node:assert/strict";
import { test } from "node:test";

import { type Palette, scan, xcolour } from "./replies.ts";

const palette: Palette = { foreground: "#d4e0f4", background: "#070c18", cursor: "#8FD8FF" };
const bytes = (s: string) => Uint8Array.from(s, (c) => c.charCodeAt(0));
const none = new Uint8Array();

test("device attributes are answered where the query ends", () => {
  const chunk = bytes("prompt> \x1b[cmore");
  assert.deepEqual(scan(none, chunk, palette), { replies: [{ end: 11, text: "\x1b[?62;22c" }], carry: none });
  assert.deepEqual(scan(none, bytes("\x1b[0c"), palette).replies, [{ end: 4, text: "\x1b[?62;22c" }]);
  assert.deepEqual(scan(none, bytes("\x1b[>c"), palette).replies, [{ end: 4, text: "\x1b[>1;10;0c" }]);
});

test("colour queries are answered in X11 form, with the terminator they came with", () => {
  assert.deepEqual(scan(none, bytes("\x1b]11;?\x1b\\"), palette).replies, [
    { end: 8, text: "\x1b]11;rgb:0707/0c0c/1818\x1b\\" },
  ]);
  assert.deepEqual(scan(none, bytes("\x1b]10;?\x07"), palette).replies, [{ end: 7, text: "\x1b]10;rgb:d4d4/e0e0/f4f4\x07" }]);
  assert.equal(xcolour("#8FD8FF"), "rgb:8f8f/d8d8/ffff");
  assert.equal(xcolour("rebeccapurple"), null);
});

test("several queries are answered in the order they were asked", () => {
  // What fish sends after a command.
  const { replies } = scan(none, bytes("\x1b]11;?\x1b\\\x1b[6n\x1b[0c"), palette);
  assert.deepEqual(
    replies.map((r) => r.end),
    [8, 16],
  );
});

test("a query split across chunks is answered once it completes", () => {
  const first = scan(none, bytes("output\x1b]11"), palette);
  assert.deepEqual(first.replies, []);
  assert.deepEqual(first.carry, bytes("\x1b]11"));
  const second = scan(first.carry, bytes(";?\x07rest"), palette);
  assert.deepEqual(second.replies, [{ end: 3, text: "\x1b]11;rgb:0707/0c0c/1818\x07" }]);
  assert.deepEqual(second.carry, none);
  const csi = scan(scan(none, bytes("\x1b["), palette).carry, bytes("c"), palette);
  assert.deepEqual(csi.replies, [{ end: 1, text: "\x1b[?62;22c" }]);
});

test("everything else passes unanswered", () => {
  for (const other of ["\x1b[6n", "\x1b[1;31m", "\x1b[?u", "\x1b]0;title\x07", "\x1b]11;#000000\x07", "\x1b[>0q", "\x1b[1 c"]) {
    assert.deepEqual(scan(none, bytes(other), palette), { replies: [], carry: none }, JSON.stringify(other));
  }
});

test("a long unfinished sequence isn't a query, so it isn't carried", () => {
  const long = bytes(`\x1b]7;file://host/${"x".repeat(64)}`);
  assert.deepEqual(scan(none, long, palette).carry, none);
});
