import assert from "node:assert/strict";
import { test } from "node:test";

import { scan, xcolour } from "./replies.ts";

const cursor = "#8FD8FF";
const bytes = (s: string) => Uint8Array.from(s, (c) => c.charCodeAt(0));
const none = new Uint8Array();

test("secondary device attributes are answered where the query ends", () => {
  const chunk = bytes("prompt> \x1b[>cmore");
  assert.deepEqual(scan(none, chunk, cursor), { replies: [{ end: 12, text: "\x1b[>1;10;0c" }], opens: [], carry: none });
  assert.deepEqual(scan(none, bytes("\x1b[>0c"), cursor).replies, [{ end: 5, text: "\x1b[>1;10;0c" }]);
});

test("the terminal names itself when asked its version", () => {
  assert.deepEqual(scan(none, bytes("\x1b[>q"), cursor).replies, [{ end: 4, text: "\x1bP>|iglu\x1b\\" }]);
  assert.deepEqual(scan(none, bytes("\x1b[>0q"), cursor).replies, [{ end: 5, text: "\x1bP>|iglu\x1b\\" }]);
});

test("the cursor's colour is answered in X11 form, with the terminator it came with", () => {
  assert.deepEqual(scan(none, bytes("\x1b]12;?\x1b\\"), cursor).replies, [{ end: 8, text: "\x1b]12;rgb:8f8f/d8d8/ffff\x1b\\" }]);
  assert.deepEqual(scan(none, bytes("\x1b]12;?\x07"), cursor).replies, [{ end: 7, text: "\x1b]12;rgb:8f8f/d8d8/ffff\x07" }]);
  assert.equal(xcolour("rebeccapurple"), null);
});

test("several queries are answered in the order they were asked", () => {
  const { replies } = scan(none, bytes("\x1b]12;?\x07\x1b[6n\x1b[>c"), cursor);
  assert.deepEqual(
    replies.map((r) => r.end),
    [7, 15],
  );
});

test("a query split across chunks is answered once it completes", () => {
  const first = scan(none, bytes("output\x1b]12"), cursor);
  assert.deepEqual(first.replies, []);
  assert.deepEqual(first.carry, bytes("\x1b]12"));
  const second = scan(first.carry, bytes(";?\x07rest"), cursor);
  assert.deepEqual(second.replies, [{ end: 3, text: "\x1b]12;rgb:8f8f/d8d8/ffff\x07" }]);
  assert.deepEqual(second.carry, none);
  const csi = scan(scan(none, bytes("\x1b[>"), cursor).carry, bytes("c"), cursor);
  assert.deepEqual(csi.replies, [{ end: 1, text: "\x1b[>1;10;0c" }]);
});

test("what wterm answers, and everything else, passes unanswered", () => {
  for (const other of ["\x1b[c", "\x1b[0c", "\x1b]10;?\x07", "\x1b]11;?\x1b\\", "\x1b[6n", "\x1b[1;31m", "\x1b[?u", "\x1b]0;title\x07", "\x1b[>1q", "\x1b[1 c", "\x1b[0 q"]) {
    assert.deepEqual(scan(none, bytes(other), cursor), { replies: [], opens: [], carry: none }, JSON.stringify(other));
  }
});

test("a long unfinished sequence isn't a query, so it isn't carried", () => {
  const long = bytes(`\x1b]7;file://host/${"x".repeat(64)}`);
  assert.deepEqual(scan(none, long, cursor).carry, none);
});

test("a long sequence of another kind passes, however long", () => {
  const copy = bytes(`\x1b]52;c;${"QUFB".repeat(100_000)}\x07\x1b[>c`);
  assert.deepEqual(scan(none, copy, cursor).replies, [{ end: copy.length, text: "\x1b[>1;10;0c" }]);
});

test("an address a program asks to open is picked out, with either terminator", () => {
  const device = "https://github.com/login/device";
  const { replies, opens, carry } = scan(none, bytes(`code\x1b]4458;${device}\x1b\\more\x1b]4458;file:///a\x07`), cursor);
  assert.deepEqual({ replies, opens, carry }, { replies: [], opens: [device, "file:///a"], carry: none });
});

test("an address split across chunks is picked out once it completes, however long", () => {
  const address = `https://example.com/${"x".repeat(4000)}`;
  const whole = bytes(`\x1b]4458;${address}\x1b\\`);
  for (const at of [1, 3, 6, 100, whole.length - 1]) {
    const first = scan(none, whole.subarray(0, at), cursor);
    assert.deepEqual(first.opens, [], String(at));
    assert.deepEqual(scan(first.carry, whole.subarray(at), cursor).opens, [address], String(at));
  }
});

test("an empty or overlong address, or another OSC, asks nothing", () => {
  const long = `https://example.com/${"x".repeat(16 * 1024)}`;
  for (const other of ["\x1b]4458;\x07", `\x1b]4458;${long}\x07`, "\x1b]44\x07", "\x1b]4459;https://a.b/\x07", "\x1b]8;;https://a.b/\x07"]) {
    assert.deepEqual(scan(none, bytes(other), cursor).opens, [], JSON.stringify(other.slice(0, 20)));
  }
  assert.deepEqual(scan(none, bytes(`\x1b]4458;${long}`), cursor).carry, none);
});
