import assert from "node:assert/strict";
import { test } from "node:test";

import fc from "fast-check";

import { nameFrom, nameFromBranch } from "./naming.ts";
import { offer } from "./opening.ts";
import { formatRoute, parseRoute, type Route } from "./route.ts";
import { closeness, search } from "./search.ts";
import { cellSize, cellSizes, fontName, formatTheme, parseTheme, type Colors } from "./themes.ts";

// PROPTEST_CASES sets how many cases each property runs, as it does for Rust's.
fc.configureGlobal({ numRuns: Number(process.env["PROPTEST_CASES"]) || 100 });

/** Text weighted toward the separators and edges the console's parsers see. */
const tricky = fc.oneof(
  fc.string(),
  fc.string({ unit: fc.constantFrom("a", "Z", "0", "-", "/", ":", "=", "#", '"', " ", "\t", "\n", "é", "漢", "🧊", "\\", "@", "."), maxLength: 60 }),
);

const label = fc.stringMatching(/^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/);

const hex = fc.integer({ min: 0, max: 0xffffff }).map((n) => `#${n.toString(16).padStart(6, "0")}`);
const colors: fc.Arbitrary<Colors> = fc.record({
  background: hex,
  foreground: hex,
  cursor: hex,
  selection: fc.option(hex, { nil: null }),
  selectionText: fc.option(hex, { nil: null }),
  palette: fc.array(hex, { minLength: 16, maxLength: 16 }),
}, { noNullPrototype: true });

test("routes read back as they're written", () => {
  const route: fc.Arbitrary<Route> = fc.oneof(
    fc.constant({ view: "overview" as const }),
    fc.constant({ view: "previews" as const }),
    fc.constant({ view: "settings" as const }),
    label.map((name) => ({ view: "workspace" as const, name })),
    label.map((name) => ({ view: "project" as const, name })),
    label.map((name) => ({ view: "project" as const, name, settings: true as const })),
  );
  fc.assert(fc.property(route, (r) => assert.deepEqual(parseRoute(formatRoute(r)), r)));
  fc.assert(
    fc.property(tricky, (path) => {
      const parsed = parseRoute(path);
      if (parsed) assert.deepEqual(parseRoute(formatRoute(parsed)), parsed);
    }),
  );
});

test("names from text are workspace names", () => {
  fc.assert(
    fc.property(tricky, (text) => {
      const name = nameFrom(text);
      assert.equal(name === null, !/[A-Za-z]/.test(text));
      if (name !== null) {
        assert.match(name, /^[a-z][a-z0-9-]{0,39}$/);
        assert.ok(!name.endsWith("-") && !name.includes("--"), name);
      }
      assert.equal(nameFromBranch(`prefix/${text.replaceAll("/", "")}`), nameFrom(text.replaceAll("/", "")));
    }),
  );
});

test("only web and file addresses are offered, and the person's browser never gets the workspace's own", () => {
  fc.assert(
    fc.property(
      fc.oneof(tricky, fc.webUrl(), fc.constantFrom("javascript:alert(1)", "data:text/html,x", "http://localhost:3000/", "http://[::1]:8080/", "http://127.9.9.9/", "file:///etc/passwd", "http://app.localhost/")),
      (address) => {
        const offered = offer(address);
        if (!offered) return;
        const url = new URL(offered.url);
        assert.ok(["http:", "https:", "file:"].includes(url.protocol), url.protocol);
        assert.ok(offered.places.includes("workspace"));
        if (offered.places.includes("here")) {
          assert.ok(url.protocol !== "file:");
          assert.ok(!/^(localhost|.*\.localhost|127\.\d+\.\d+\.\d+|0\.0\.0\.0|\[::1?\])$/.test(url.hostname), url.hostname);
        }
      },
    ),
  );
});

test("a search finds only what it's given, closest first, keeping order among equals", () => {
  const order = ["name", "start", "word", "inside", "beside", "scattered"];
  fc.assert(
    fc.property(tricky, fc.array(fc.record({ label: tricky, sub: tricky }, { noNullPrototype: true }), { maxLength: 12 }), (query, items) => {
      const found = search(query, items);
      for (const item of found) assert.ok(items.includes(item));
      if (!query.trim()) return assert.deepEqual(found, items);
      const ranks = found.map((item) => order.indexOf(closeness(query, item.label, item.sub) ?? ""));
      for (let i = 1; i < ranks.length; i++) {
        assert.ok(ranks[i - 1]! <= ranks[i]!);
        if (ranks[i - 1] === ranks[i]) assert.ok(items.indexOf(found[i - 1]!) < items.indexOf(found[i]!));
      }
      // Whatever matches is found.
      assert.equal(found.length, items.filter((item) => closeness(query, item.label, item.sub) !== null).length);
    }),
  );
  fc.assert(fc.property(tricky.filter((s) => s.trim() === s && s !== ""), (name) => assert.equal(closeness(name, name), "name")));
});

test("themes read back as they're written, and nothing pasted throws", () => {
  fc.assert(fc.property(colors, (c) => assert.deepEqual(parseTheme(formatTheme(c)), { colors: c })));
  fc.assert(
    fc.property(tricky, (text) => {
      const parsed = parseTheme(text);
      if ("colors" in parsed) {
        for (const colour of [parsed.colors.background, parsed.colors.foreground, parsed.colors.cursor, ...parsed.colors.palette]) assert.match(colour, /^#[0-9a-f]{6}$/);
        assert.equal(parsed.colors.palette.length, 16);
      }
    }),
  );
});

test("font names are safe to put in CSS, and settle", () => {
  fc.assert(
    fc.property(tricky, (text) => {
      const name = fontName(text);
      if (name === null) return;
      assert.ok(!/["'\\;{}()<>]/.test(name), name);
      assert.equal(fontName(name), name);
    }),
  );
});

test("cells are whole pixels at every size a font comes in", () => {
  fc.assert(
    fc.property(fc.double({ min: 0.3, max: 1, noNaN: true }), fc.double({ min: 1, max: 80, noNaN: true }), (advance, asked) => {
      const size = cellSize(advance, asked);
      assert.ok(Number.isInteger(size.width) && size.width >= 1);
      assert.ok(Number.isInteger(size.row));
    }),
  );
  fc.assert(
    fc.property(fc.double({ min: 0.3, max: 1, noNaN: true }), (advance) => {
      const sizes = cellSizes(advance);
      for (let i = 1; i < sizes.length; i++) assert.ok(sizes[i]!.width > sizes[i - 1]!.width);
      for (const size of sizes) assert.ok(size.font <= 28 + 1e-9, String(size.font));
    }),
  );
});
