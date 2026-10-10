import assert from "node:assert/strict";
import { test } from "node:test";

import { type Chord, chordLabel, DEFAULT_PREFIX, withCtrl, type KeyInput, type Keymap, metaBytes, prefixBytes, resolve } from "./keys.ts";

const mac: Keymap = { mac: true, prefix: DEFAULT_PREFIX, altMoves: true };
const linux: Keymap = { ...mac, mac: false };

const key = (code: string, extra: Partial<KeyInput> = {}): KeyInput => ({
  key: code.startsWith("Key") ? code.slice(3).toLowerCase() : code,
  code,
  alt: false,
  meta: false,
  ctrl: false,
  shift: false,
  focus: "terminal",
  inWorkspace: true,
  ...extra,
});
const prefix = key("Space", { key: " ", ctrl: true });

test("a terminal keeps every chord shells and agents use", () => {
  const theirs: Partial<KeyInput>[] = [
    // readline and fish: words, history tokens, kill-line, help, list directory.
    ...["KeyB", "KeyF", "KeyD", "KeyE", "KeyP", "KeyS", "KeyV", "KeyW", "KeyU", "KeyC", "KeyT", "Period", "Comma"].map((code) => ({ code, alt: true })),
    ...["KeyA", "KeyE", "KeyK", "KeyU", "KeyW", "KeyR", "KeyL", "KeyB", "KeyF", "KeyP", "KeyN"].map((code) => ({ code, ctrl: true })),
    // Plain typing, Escape, and the arrows.
    ...["KeyF", "KeyJ", "KeyX", "Escape", "ArrowLeft", "Enter", "Slash"].map((code) => ({ code })),
  ];
  for (const extra of theirs) {
    const input = key(extra.code!, extra);
    assert.deepEqual(resolve("normal", input, mac), { kind: "pass" }, JSON.stringify(extra));
  }
});

test("off a Mac, Ctrl+K in a terminal is the shell's, and the palette elsewhere", () => {
  assert.deepEqual(resolve("normal", key("KeyK", { ctrl: true }), linux), { kind: "pass" });
  assert.deepEqual(resolve("normal", key("KeyK", { ctrl: true, focus: "page" }), linux), { kind: "act", action: { kind: "palette" } });
  assert.deepEqual(resolve("normal", key("KeyK", { meta: true }), mac), { kind: "act", action: { kind: "palette" } });
});

test("the prefix arms, and the next key is the console's", () => {
  assert.deepEqual(resolve("normal", prefix, mac), { kind: "arm" });
  assert.deepEqual(resolve("prefix", key("KeyL"), mac), { kind: "act", action: { kind: "column", step: 1 } });
  assert.deepEqual(resolve("prefix", key("KeyL", { shift: true }), mac), { kind: "act", action: { kind: "move-column", step: 1 } });
  assert.deepEqual(resolve("prefix", key("Digit3"), mac), { kind: "act", action: { kind: "column-at", index: 2 } });
  assert.deepEqual(resolve("prefix", key("Shift", { key: "Shift" }), mac), { kind: "hold" });
  assert.deepEqual(resolve("prefix", key("Escape", { key: "Escape" }), mac), { kind: "cancel" });
  assert.deepEqual(resolve("prefix", key("KeyQ"), mac), { kind: "cancel" });
  assert.deepEqual(resolve("prefix", prefix, mac), { kind: "send-prefix" });
});

test("⌥H/J/K/L move only when the person lets the console take them", () => {
  assert.deepEqual(resolve("normal", key("KeyH", { alt: true }), mac), { kind: "act", action: { kind: "column", step: -1 } });
  assert.deepEqual(resolve("normal", key("KeyJ", { alt: true }), mac), { kind: "act", action: { kind: "workspace", step: 1 } });
  assert.deepEqual(resolve("normal", key("KeyH", { alt: true }), { ...mac, altMoves: false }), { kind: "pass" });
  assert.deepEqual(resolve("normal", key("KeyH", { alt: true, focus: "field" }), mac), { kind: "pass" });
});

test("plain keys act only on pages with nothing to type into, outside a workspace", () => {
  const page = { focus: "page", inWorkspace: false } as const;
  assert.deepEqual(resolve("normal", key("KeyJ", page), mac), { kind: "act", action: { kind: "workspace", step: 1 } });
  assert.deepEqual(resolve("normal", key("Enter", page), mac), { kind: "act", action: { kind: "open" } });
  assert.deepEqual(resolve("normal", key("Slash", { ...page, shift: true }), mac), { kind: "act", action: { kind: "keys" } });
  assert.deepEqual(resolve("normal", key("KeyF", { focus: "page", inWorkspace: true }), mac), { kind: "pass" });
  assert.deepEqual(resolve("normal", key("KeyJ", { focus: "field", inWorkspace: false }), mac), { kind: "pass" });
});

test("a focused button, link or dialog keeps its own keys", () => {
  const control = { focus: "control", inWorkspace: false } as const;
  assert.deepEqual(resolve("normal", key("Enter", control), mac), { kind: "pass" });
  assert.deepEqual(resolve("normal", key("KeyJ", control), mac), { kind: "pass" });
  // The palette and the prefix still reach iglu from one.
  assert.deepEqual(resolve("normal", key("KeyK", { ...control, meta: true }), mac), { kind: "act", action: { kind: "palette" } });
  assert.deepEqual(resolve("normal", { ...prefix, ...control }, mac), { kind: "arm" });
});

test("another prefix works the same way", () => {
  const b: Chord = { code: "KeyB", ctrl: true, alt: false, shift: false, meta: false };
  const tmux = { ...mac, prefix: b };
  assert.deepEqual(resolve("normal", key("KeyB", { ctrl: true }), tmux), { kind: "arm" });
  assert.deepEqual(resolve("normal", prefix, tmux), { kind: "pass" });
  assert.equal(prefixBytes(b), "\x02");
  assert.equal(prefixBytes(DEFAULT_PREFIX), "\x00");
  assert.equal(prefixBytes({ ...b, ctrl: false }), null);
});

test("Option as Meta sends ESC and the unmodified character", () => {
  assert.equal(metaBytes("KeyB", false), "\x1bb");
  assert.equal(metaBytes("KeyB", true), "\x1bB");
  assert.equal(metaBytes("Period", false), "\x1b.");
  assert.equal(metaBytes("Digit2", true), "\x1b@");
  assert.equal(metaBytes("ArrowLeft", false), null);
  assert.equal(metaBytes("Backspace", false), null);
});

test("a chord shows the key as it's printed", () => {
  const chord = (code: string): Chord => ({ code, ctrl: true, alt: false, shift: false, meta: false });
  assert.equal(chordLabel(chord("BracketLeft"), true), "⌃[");
  assert.equal(chordLabel(chord("Backslash"), false), "Ctrl+\\");
  assert.equal(chordLabel(chord("KeyB"), true), "⌃B");
  assert.equal(chordLabel(chord("Space"), false), "Ctrl+Space");
});

test("Ctrl makes control characters of letters and a few symbols, and leaves the rest", () => {
  assert.equal(withCtrl("c"), "\x03");
  assert.equal(withCtrl("C"), "\x03");
  assert.equal(withCtrl("["), "\x1b");
  assert.equal(withCtrl("@"), "\x00");
  assert.equal(withCtrl("?"), "\x7f");
  assert.equal(withCtrl("1"), "1");
  assert.equal(withCtrl("é"), "é");
  assert.equal(withCtrl("ls"), "ls");
});
