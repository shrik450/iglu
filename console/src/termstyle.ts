// The terminal's type and colours, as chosen in Settings (preferences.ts),
// set on the page as CSS properties, which terminals and the columns around them read,
// so a change reaches every open terminal without a reload.

import { signal } from "@preact/signals";

import type { TerminalStyle } from "./generated/TerminalStyle.ts";
import { XTERM, cellSize, colorsOf, isLight } from "./state/themes.ts";

/** The terminal font's character width as a fraction of its size, as last
 * measured: JetBrains Mono's is 0.6. */
export const advance = signal(0.6);

/** What's wrong with the font asked for, if anything, so the terminal
 * shows iglu's instead. */
export const fontTrouble = signal<"missing" | "proportional" | null>(null);

let applying: Promise<void> = Promise.resolve();
let latest = 0;

/** Resolves once the style last asked for is on the page, with its font
 * loaded: a terminal measures its cells as it opens. One asked for while
 * waiting is waited for too. */
export async function styleApplied(): Promise<void> {
  let awaited: Promise<void>;
  do {
    awaited = applying;
    await awaited;
  } while (awaited !== applying);
}

export function applyTermStyle(prefs: TerminalStyle): Promise<void> {
  const turn = ++latest;
  applying = (async () => {
    await document.fonts.load(`16px ${prefs.font ? `"${prefs.font}", ` : ""}'JetBrains Mono'`).catch(() => []);
    // A later choice may have been applied while this one's font loaded.
    if (turn !== latest) return;
    const root = document.documentElement;
    const set = (property: string, value: string | null) => (value === null ? root.style.removeProperty(property) : root.style.setProperty(property, value));

    // iglu's own colours are the stylesheet's, which follow the look; a
    // theme's take over the terminal and its column alike.
    const colors = colorsOf(prefs.theme, prefs.pasted);
    set("--term-bg", colors?.background ?? null);
    set("--term-ink", colors?.foreground ?? null);
    set("--term-dim", colors ? `color-mix(in srgb, ${colors.foreground} 55%, ${colors.background})` : null);
    set("--term-line", colors ? `color-mix(in srgb, ${colors.foreground} 14%, ${colors.background})` : null);
    set("--term-caret", colors?.cursor ?? null);
    set("--term-pick", colors?.selection ?? null);
    // A selection painted over the text, as Select All's is, lets it show through.
    set("--term-veil", colors?.selection ? `color-mix(in srgb, ${colors.selection} 40%, transparent)` : null);
    set("--term-pick-ink", colors?.selectionText ?? null);
    // Without one, selected text keeps its own colours.
    root.toggleAttribute("data-term-pick-ink", Boolean(colors?.selectionText));
    (colors?.palette ?? XTERM).forEach((colour, index) => set(`--term-ansi-${index}`, colour));
    // What's drawn on the column around a light theme needs darker hues.
    if (colors && isLight(colors)) root.dataset["termTone"] = "light";
    else delete root.dataset["termTone"];

    const context = document.createElement("canvas").getContext("2d");
    const width = (font: string, text: string) => {
      if (!context) return 0;
      context.font = `100px ${font}`;
      return context.measureText(text).width;
    };
    // A font that isn't there falls back alike whatever comes after it; a
    // proportional one can't keep to a terminal's cells.
    const asked = prefs.font && `"${prefs.font}"`;
    const missing = asked !== null && width(`${asked}, monospace`, "mmmwwwiii") === width("monospace", "mmmwwwiii") && width(`${asked}, serif`, "mmmwwwiii") === width("serif", "mmmwwwiii");
    const proportional = asked !== null && !missing && width(`${asked}, monospace`, "iiiii") !== width(`${asked}, monospace`, "WWWWW");
    fontTrouble.value = missing ? "missing" : proportional ? "proportional" : null;
    const font = prefs.font && !missing && !proportional ? prefs.font : null;
    const family = font ? `"${font}", 'JetBrains Mono'` : "'JetBrains Mono'";
    // As wterm measures a cell: by a W.
    advance.value = width(`${family}, monospace`, "W") / 100 || 0.6;
    const size = cellSize(advance.value, prefs.size);
    set("--term-face", font ? `"${font}", var(--mono)` : null);
    set("--term-size", `${size.font}px`);
    set("--term-row", `${size.row}px`);
  })();
  return applying;
}
