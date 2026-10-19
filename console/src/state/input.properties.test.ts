import assert from "node:assert/strict";
import { test } from "node:test";

import fc from "fast-check";

import type { ColumnSpec } from "../generated/ColumnSpec.ts";
import { displaySize, preferred, scaleOf, typing } from "./browser.ts";
import { DEFAULT_PREFIX, metaBytes, prefixBytes, resolve, sameChord, withCtrl, type Chord, type KeyInput } from "./keys.ts";
import { moved, placed, scrollTarget, stepIndex, widened } from "./layout.ts";

// PROPTEST_CASES sets how many cases each property runs, as it does for Rust's.
fc.configureGlobal({ numRuns: Number(process.env["PROPTEST_CASES"]) || 100 });

const code = fc.oneof(
  fc.constantFrom("KeyA", "KeyK", "KeyF", "KeyH", "KeyJ", "KeyL", "Space", "Slash", "Enter", "Escape", "Tab", "Digit1", "ArrowUp", "Semicolon", "Comma", "BracketLeft"),
  fc.stringMatching(/^Key[A-Z]$/),
);
const chord: fc.Arbitrary<Chord> = fc.record({ code, ctrl: fc.boolean(), alt: fc.boolean(), shift: fc.boolean(), meta: fc.boolean() }, { noNullPrototype: true });
const input = (focus: KeyInput["focus"]): fc.Arbitrary<KeyInput> =>
  fc.record(
    {
      key: fc.constantFrom("a", "k", "Control", "Shift", "Meta", "Alt", " ", "Enter"),
      code,
      alt: fc.boolean(),
      meta: fc.boolean(),
      ctrl: fc.boolean(),
      shift: fc.boolean(),
      optionMeta: fc.boolean(),
      focus: fc.constant(focus),
      inWorkspace: fc.boolean(),
    },
    { noNullPrototype: true },
  );

test("terminals keep every key but the prefix, ⌘ on a Mac, and what the person gave the console", () => {
  fc.assert(
    fc.property(input("terminal"), fc.boolean(), fc.boolean(), fc.option(chord, { nil: undefined }), (key, mac, altMoves, prefix) => {
      const keymap = { mac, altMoves, prefix: prefix ?? DEFAULT_PREFIX };
      const outcome = resolve("normal", key, keymap);
      if (outcome.kind === "pass") return;
      const isPrefix = sameChord(key, keymap.prefix);
      const command = mac && key.meta && !key.ctrl;
      const find = !mac && key.ctrl && key.shift && !key.meta && key.code === "KeyF";
      const moves = altMoves && key.alt && key.optionMeta && !key.ctrl && !key.meta;
      assert.ok(isPrefix || command || find || moves, JSON.stringify({ key, keymap, outcome }));
    }),
  );
});

test("a field keeps its plain keys", () => {
  fc.assert(
    fc.property(input("field"), fc.boolean(), (key, mac) => {
      fc.pre(!key.ctrl && !key.meta && !key.alt);
      const outcome = resolve("normal", key, { mac, altMoves: true, prefix: DEFAULT_PREFIX });
      assert.ok(outcome.kind === "pass" || sameChord(key, DEFAULT_PREFIX), JSON.stringify(outcome));
    }),
  );
});

test("a usable prefix reaches a terminal as one control character", () => {
  fc.assert(
    fc.property(chord, (prefix) => {
      const sent = prefixBytes(prefix);
      if (sent === null) return;
      assert.equal(sent.length, 1);
      assert.ok(sent.charCodeAt(0) < 0x20, JSON.stringify(sent));
      assert.ok(prefix.ctrl && !prefix.alt && !prefix.meta);
    }),
  );
});

test("Option as Meta sends Escape and one character", () => {
  fc.assert(
    fc.property(code, fc.boolean(), (key, shift) => {
      const sent = metaBytes(key, shift);
      if (sent !== null) {
        assert.equal(sent.length, 2);
        assert.equal(sent[0], "\x1b");
      }
    }),
  );
});

test("Ctrl makes letters control characters and leaves the rest", () => {
  fc.assert(
    fc.property(fc.string({ maxLength: 3 }), (typed) => {
      const sent = withCtrl(typed);
      if (/^[A-Za-z]$/.test(typed)) assert.equal(sent.charCodeAt(0), typed.toUpperCase().charCodeAt(0) - 64);
      if (typed.length !== 1) assert.equal(sent, typed);
      assert.ok(sent.length <= Math.max(1, typed.length));
    }),
  );
});

const columns = fc.uniqueArray(fc.stringMatching(/^[a-z][a-z0-9]{0,6}$/), { maxLength: 8 }).map((names) =>
  names.map((name): ColumnSpec => ({ name, kind: { kind: "shell" }, width: "half", label: null })),
);

test("moving or placing a column only reorders the strip", () => {
  fc.assert(
    fc.property(columns, fc.nat(), fc.nat(), fc.boolean(), fc.constantFrom(-1 as const, 1 as const), (cols, a, b, after, step) => {
      const names = cols.map((c) => c.name).sort();
      const name = cols[a % Math.max(1, cols.length)]?.name ?? "absent";
      const target = cols[b % Math.max(1, cols.length)]?.name ?? "absent";
      for (const next of [moved(cols, name, step), placed(cols, name, target, after), widened(cols, name)]) {
        if (next) assert.deepEqual(next.map((c) => c.name).sort(), names);
      }
      const there = moved(cols, name, step);
      if (there) assert.deepEqual(moved(there, name, step === 1 ? -1 : 1), cols);
    }),
  );
});

test("stepping between columns stays in the strip", () => {
  fc.assert(
    fc.property(fc.nat(20), fc.integer({ min: -1, max: 21 }), fc.constantFrom(-1 as const, 1 as const), (length, index, step) => {
      const next = stepIndex(length, index, step);
      if (length === 0) assert.equal(next, -1);
      else assert.ok(next >= 0 && next < length);
    }),
  );
});

test("the strip scrolls to show the focused column whole, when it fits", () => {
  fc.assert(
    fc.property(
      fc.array(fc.integer({ min: 50, max: 900 }), { minLength: 1, maxLength: 10 }),
      fc.nat(),
      fc.integer({ min: 200, max: 2000 }),
      fc.nat(5000),
      fc.integer({ min: 0, max: 40 }),
      (widths, at, view, scroll, peek) => {
        let left = 0;
        const spans = widths.map((width) => {
          const span = { left, width };
          left += width;
          return span;
        });
        const focused = at % spans.length;
        const target = scrollTarget(spans, focused, view, scroll, peek);
        const max = Math.max(0, left - view);
        assert.ok(target >= 0 && target <= max, `${target} outside 0..${max}`);
        const span = spans[focused]!;
        if (span.width + 2 * peek <= view) assert.ok(span.left >= target && span.left + span.width <= target + view, JSON.stringify({ span, target, view }));
      },
    ),
  );
});

test("the browser's display always has a size, or none", () => {
  fc.assert(
    fc.property(fc.double({ min: 0, max: 5000, noNaN: true }), fc.double({ min: 0, max: 5000, noNaN: true }), fc.double({ min: 0.5, max: 8, noNaN: true }), (w, h, ratio) => {
      const scale = scaleOf(ratio);
      assert.ok(scale >= 1 && scale <= 4);
      const size = displaySize({ w, h }, scale);
      if (size) assert.ok(size.w >= 1 && size.h >= 1 && Number.isInteger(size.w) && Number.isInteger(size.h));
      else assert.ok(Math.round(w * scale) <= 0 || Math.round(h * scale) <= 0);
    }),
  );
});

test("text types into the browser key by key, unless it needs pasting", () => {
  fc.assert(
    fc.property(fc.string({ unit: "grapheme", maxLength: 20 }), (text) => {
      const keys = typing(text);
      const beyond = [...text].some((c) => (c.codePointAt(0) ?? 0) > 0xffff);
      assert.equal(keys === null, beyond);
      if (keys) assert.equal(keys.length, [...text.replaceAll("\r\n", "\n")].length);
    }),
  );
});

test("encodings offered are those the display has, the wanted first", () => {
  fc.assert(
    fc.property(fc.uniqueArray(fc.constantFrom(0, 1, 2, 5, 7, 16, 21, 50, -223), { maxLength: 9 }), fc.boolean(), (encodings, hardware) => {
      const offered = preferred(encodings, hardware);
      assert.deepEqual([...offered].sort(), [...encodings].sort());
    }),
  );
});
