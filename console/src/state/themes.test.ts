import assert from "node:assert/strict";
import { test } from "node:test";

import { THEMES, XTERM, cellSize, cellSizes, colorsOf, fontName, formatTheme, isLight, parseTheme } from "./themes.ts";

test("a Ghostty theme file reads as its colours", () => {
  const text = "# Dracula\npalette = 0=#21222c\npalette = 15=ffffff\nbackground = #282a36\nforeground = #F8F8F2\ncursor-color = #f8f8f2\ncursor-text = #282a36\nselection-background = #44475a\n";
  const parsed = parseTheme(text);
  assert.ok("colors" in parsed, JSON.stringify(parsed));
  assert.equal(parsed.colors.background, "#282a36");
  assert.equal(parsed.colors.foreground, "#f8f8f2");
  assert.equal(parsed.colors.selection, "#44475a");
  assert.equal(parsed.colors.selectionText, null);
  assert.equal(parsed.colors.palette[0], "#21222c");
  assert.equal(parsed.colors.palette[15], "#ffffff");
  // What isn't given is xterm's.
  assert.equal(parsed.colors.palette[1], XTERM[1]);
});

test("a whole config can be pasted: what isn't a colour is passed over", () => {
  const parsed = parseTheme("font-family = Berkeley Mono\nfont-size = 14\nbackground = #123\nforeground = #abcdef\n");
  assert.ok("colors" in parsed);
  assert.equal(parsed.colors.background, "#112233");
  assert.equal(parsed.colors.cursor, "#abcdef");
});

test("what Ghostty takes that iglu has no use for passes", () => {
  const parsed = parseTheme('background = "#101010"\nforeground = #eeeeee\ncursor-color =\npalette = 200=#ff0000\npalette = "1=#ff0000"');
  assert.ok("colors" in parsed, JSON.stringify(parsed));
  assert.equal(parsed.colors.background, "#101010");
  assert.equal(parsed.colors.cursor, "#eeeeee");
  assert.equal(parsed.colors.palette[1], "#ff0000");
  assert.equal(parsed.colors.palette.length, 16);
});

test("what can't be read says where", () => {
  assert.deepEqual(parseTheme("background = #000000\nforeground = white"), { error: "Line 2: white isn't a colour iglu reads; use hex, like #1e1e2e." });
  assert.deepEqual(parseTheme("background = #000000\npalette = #ffffff"), { error: "Line 2: a palette entry is `palette = 0=#1e1e2e`." });
  assert.deepEqual(parseTheme("background #000000"), { error: "Line 1 isn't `key = value`." });
  assert.deepEqual(parseTheme("background = #000000"), { error: "A theme needs at least a background and a foreground." });
});

test("every theme iglu has goes out and comes back in Ghostty's format", () => {
  for (const [name, colors] of Object.entries(THEMES)) {
    assert.deepEqual(parseTheme(formatTheme(colors)), { colors }, name);
    assert.equal(colors.palette.length, 16, name);
  }
});

test("a theme is found by name, and iglu's own has no colours of its own", () => {
  assert.equal(colorsOf("iglu", ""), null);
  assert.equal(colorsOf("Dracula", ""), THEMES["Dracula"]);
  assert.equal(colorsOf("toString", ""), null);
  assert.equal(colorsOf("pasted", "background = #101010\nforeground = #eeeeee")?.background, "#101010");
  assert.equal(colorsOf("pasted", "nonsense"), null);
});

test("light themes are told from dark ones by their background", () => {
  assert.equal(isLight(THEMES["Solarized Light"]!), true);
  assert.equal(isLight(THEMES["Solarized Dark"]!), false);
});

test("a font's name is taken only when it can be quoted as it is", () => {
  assert.equal(fontName("  Berkeley   Mono "), "Berkeley Mono");
  assert.equal(fontName("Iosevka Term 2.0"), "Iosevka Term 2.0");
  assert.equal(fontName(""), null);
  assert.equal(fontName('Fira"; color: red'), null);
});

test("text is sized so every cell is whole pixels", () => {
  // JetBrains Mono advances 0.6 of its size: iglu's 13⅓px is 8 by 17.
  assert.deepEqual(cellSize(0.6, 13), { font: 8 / 0.6, width: 8, row: 17 });
  assert.equal(cellSize(0.6, 15).width, 9);
  assert.equal(cellSize(0.6, 1).width, 1);
  const sizes = cellSizes(0.6);
  assert.deepEqual(
    sizes.map((s) => s.width),
    [6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
  );
  assert.ok(sizes.every((s) => Number.isInteger(s.row) && Number.isInteger(s.width)));
});
