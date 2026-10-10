import assert from "node:assert/strict";
import { test } from "node:test";

import { addressOf, buttonOf, clicksAfter, clipboardAction, keyOf, modifiersOf, pagePoint, tabName, wheelPixels } from "./browser.ts";

test("a point on the screen is the point on the page under it", () => {
  // A 1600×1200 image of an 800×600 page, shown in a 400×400 box: 400×300 from the top left.
  const box = { width: 400, height: 400 };
  const image = { width: 1600, height: 1200 };
  const page = { width: 800, height: 600 };
  assert.deepEqual(pagePoint(box, image, page, 200, 150), { x: 400, y: 300 });
  assert.deepEqual(pagePoint(box, image, page, 0, 0), { x: 0, y: 0 });
  // Below the image is off the page.
  assert.equal(pagePoint(box, image, page, 200, 350), null);
  // Before any image, nothing is.
  assert.equal(pagePoint(box, { width: 0, height: 0 }, page, 10, 10), null);
});

test("a Mac's Command reaches the browser as Control", () => {
  const command = { altKey: false, ctrlKey: false, metaKey: true, shiftKey: false };
  assert.deepEqual(modifiersOf(command, true), { alt: false, ctrl: true, meta: false, shift: false });
  assert.deepEqual(modifiersOf(command, false), { alt: false, ctrl: false, meta: true, shift: false });
});

test("a Mac's Command key itself is a Control key to the browser", () => {
  const command = { type: "keydown", key: "Meta", code: "MetaRight", repeat: false, location: 2, isComposing: false, altKey: false, ctrlKey: false, metaKey: true, shiftKey: false } as KeyboardEvent;
  assert.deepEqual(keyOf(command, true), {
    action: "press",
    key: "Control",
    code: "ControlRight",
    repeat: false,
    location: 2,
    modifiers: { alt: false, ctrl: true, meta: false, shift: false },
  });
  assert.equal(keyOf(command, false)?.key, "Meta");
  assert.equal(keyOf({ ...command, key: "Process" } as KeyboardEvent, true), null);
});

test("copying asks for the selection and pasting waits for the paste", () => {
  const ctrl = { alt: false, ctrl: true, meta: false, shift: false };
  assert.equal(clipboardAction(ctrl, "c"), "copy");
  assert.equal(clipboardAction(ctrl, "X"), "copy");
  assert.equal(clipboardAction(ctrl, "v"), "paste");
  assert.equal(clipboardAction(ctrl, "a"), null);
  assert.equal(clipboardAction({ ...ctrl, ctrl: false }, "c"), null);
});

test("presses close together in time and place count up to a triple click", () => {
  const first = { at: 0, x: 10, y: 10, button: 0 };
  assert.equal(clicksAfter(null, first), 1);
  const second = { at: 200, x: 12, y: 11, button: 0 };
  assert.equal(clicksAfter({ ...first, clicks: 1 }, second), 2);
  assert.equal(clicksAfter({ ...second, clicks: 3 }, { ...second, at: 300 }), 3);
  assert.equal(clicksAfter({ ...first, clicks: 1 }, { ...second, at: 900 }), 1);
  assert.equal(clicksAfter({ ...first, clicks: 1 }, { ...second, x: 40 }), 1);
  assert.equal(clicksAfter({ ...first, clicks: 1 }, { ...second, button: 2 }), 1);
});

test("buttons and wheels in the browser's terms", () => {
  assert.equal(buttonOf(0), "left");
  assert.equal(buttonOf(2), "right");
  assert.equal(buttonOf(7), "none");
  assert.equal(wheelPixels(3, 1, 600), 48);
  assert.equal(wheelPixels(1, 2, 600), 600);
  assert.equal(wheelPixels(-12.5, 0, 600), -12.5);
});

test("tabs are named by title, then by where they are", () => {
  assert.equal(tabName({ id: "1", title: "Docs", url: "https://example.com/docs" }), "Docs");
  assert.equal(tabName({ id: "1", title: "localhost:3000/a", url: "localhost:3000/a" }), "localhost:3000/a");
  assert.equal(tabName({ id: "1", title: "", url: "http://localhost:3000/a" }), "localhost:3000");
  assert.equal(tabName({ id: "1", title: "about:blank", url: "about:blank" }), "New tab");
  assert.equal(addressOf({ id: "1", title: "", url: "about:blank" }), "");
  assert.equal(addressOf(undefined), "");
});
