import assert from "node:assert/strict";
import { test } from "node:test";

import { ALT, clipboardShortcut, CONTROL, displaySize, keyFor, keysymOf, modifiersFor, pasteStep, preferred, scaleOf, SHIFT, SUPER, typing } from "./browser.ts";

const plain = { key: "", altKey: false, ctrlKey: false, metaKey: false, shiftKey: false };

test("a key is the keysym X names it by", () => {
  assert.equal(keysymOf("a", 0), 0x61);
  assert.equal(keysymOf("é", 0), 0xe9);
  assert.equal(keysymOf("€", 0), 0x0100_20ac);
  assert.equal(keysymOf("Enter", 0), 0xff0d);
  assert.equal(keysymOf("Enter", 3), 0xff8d);
  assert.equal(keysymOf("ArrowLeft", 0), 0xff51);
  assert.equal(keysymOf("F1", 0), 0xffbe);
  assert.equal(keysymOf("F12", 0), 0xffc9);
  assert.equal(keysymOf("Shift", 1), 0xffe1);
  assert.equal(keysymOf("Shift", 2), 0xffe2);
  assert.equal(keysymOf("Unidentified", 0), null);
  assert.equal(keysymOf("F25", 0), null);
});

test("a Mac's Command is held as Control, and its Option only for keys it doesn't type with", () => {
  assert.deepEqual(modifiersFor({ ...plain, key: "c", metaKey: true }, true), [CONTROL]);
  assert.deepEqual(modifiersFor({ ...plain, key: "c", metaKey: true }, false), [SUPER]);
  // Option+F types ƒ, which isn't Alt+F to the browser.
  assert.deepEqual(modifiersFor({ ...plain, key: "ƒ", altKey: true }, true), []);
  assert.deepEqual(modifiersFor({ ...plain, key: "ArrowLeft", altKey: true }, true), [ALT]);
  assert.deepEqual(modifiersFor({ ...plain, key: "f", altKey: true }, false), [ALT]);
  assert.deepEqual(modifiersFor({ ...plain, key: "T", shiftKey: true, ctrlKey: true }, false), [SHIFT, CONTROL]);
});

test("a Mac's Command+Shift letter is the capital, as Shift makes it elsewhere", () => {
  assert.equal(keyFor({ ...plain, key: "t", metaKey: true, shiftKey: true }, true), "T");
  assert.equal(keyFor({ ...plain, key: "t", metaKey: true }, true), "t");
  assert.equal(keyFor({ ...plain, key: "[", metaKey: true, shiftKey: true }, true), "[");
});

test("copying and pasting are the platform's shortcuts", () => {
  assert.equal(clipboardShortcut({ ...plain, key: "v", metaKey: true }, true), "paste");
  assert.equal(clipboardShortcut({ ...plain, key: "V", ctrlKey: true, shiftKey: true }, false), "paste");
  assert.equal(clipboardShortcut({ ...plain, key: "c", ctrlKey: true }, false), "copy");
  assert.equal(clipboardShortcut({ ...plain, key: "x", metaKey: true }, true), "copy");
  assert.equal(clipboardShortcut({ ...plain, key: "v", metaKey: true }, false), null);
  assert.equal(clipboardShortcut({ ...plain, key: "v", ctrlKey: true, altKey: true }, false), null);
  assert.equal(clipboardShortcut({ ...plain, key: "v" }, false), null);
});

test("text is typed a keysym a character, except what only a paste brings whole", () => {
  assert.deepEqual(typing("hé!"), [0x68, 0xe9, 0x21]);
  assert.deepEqual(typing("a\r\nb\tc"), [0x61, 0xff0d, 0x62, 0xff09, 0x63]);
  assert.deepEqual(typing("€"), [0x0100_20ac]);
  assert.equal(typing("hi 😀"), null);
});

test("the display is drawn in the screen's device pixels, at a scale it takes", () => {
  assert.equal(scaleOf(2), 2);
  assert.equal(scaleOf(1.333_333), 1.33);
  // Zoomed out, or past what the display draws.
  assert.equal(scaleOf(0.9), 1);
  assert.equal(scaleOf(5), 4);
  assert.deepEqual(displaySize({ w: 640.5, h: 400 }, 2), { w: 1281, h: 800 });
});

test("a screen without a pixel each way, as a hidden one, has no size for the display", () => {
  assert.equal(displaySize({ w: 0, h: 0 }, 2), null);
  assert.equal(displaySize({ w: 640, h: 0 }, 2), null);
  assert.equal(displaySize({ w: 0.2, h: 400 }, 1), null);
  assert.deepEqual(displaySize({ w: 0.5, h: 400 }, 1), { w: 1, h: 400 });
});

test("the display is asked for H.264 when it encodes on a GPU, and ZRLE otherwise", () => {
  const offered = [1, 50, 7, 16, 0, -256];
  assert.deepEqual(preferred(offered, false), [16, 1, 50, 7, 0, -256]);
  assert.deepEqual(preferred(offered, true), [50, 16, 1, 7, 0, -256]);
  // A browser that can't decode H.264 doesn't offer it.
  assert.deepEqual(preferred([1, 7, 16, 0], true), [16, 1, 7, 0]);
});

test("a paste's keys wait for a message that carries its text, not one that only announces it", () => {
  const cutText = (length: number, ...rest: number[]) => {
    const message = new Uint8Array(8 + rest.length * 4);
    const view = new DataView(message.buffer);
    message[0] = 6;
    view.setInt32(4, length);
    rest.forEach((word, index) => view.setUint32(8 + index * 4, word));
    return message;
  };
  // Without extended clipboards, the text itself.
  assert.equal(pasteStep(Uint8Array.of(...cutText(5).subarray(0, 8), ...new TextEncoder().encode("hello"))), "text");
  assert.equal(pasteStep(cutText(0)), "text");
  // With them, a Notify announces the text and the Provide that follows the
  // display's Request carries it; caps and requests do neither.
  assert.equal(pasteStep(cutText(-4, 0x0800_0001)), "announced");
  assert.equal(pasteStep(cutText(-12, 0x1000_0001, 0x789c)), "text");
  assert.equal(pasteStep(cutText(-4, 0x0800_0000)), null);
  // Caps lists every action, Provide and Notify among them.
  assert.equal(pasteStep(cutText(-8, 0x1f00_0001, 0)), null);
  assert.equal(pasteStep(cutText(-4, 0x0200_0001)), null);
  // Not a ClientCutText at all: a key, a pointer, or too short to say.
  assert.equal(pasteStep(Uint8Array.of(4, 1, 0, 0, 0, 0, 0, 0x76)), null);
  assert.equal(pasteStep(Uint8Array.of(5, 0, 0, 10, 0, 10)), null);
  assert.equal(pasteStep(Uint8Array.of(6, 0, 0)), null);
});
