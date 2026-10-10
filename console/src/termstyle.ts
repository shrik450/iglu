// The terminal's type and colours, as chosen in Settings. They're set on the
// page as CSS properties, which terminals and the columns around them read,
// so a change reaches every open terminal without a reload.

import { signal } from "@preact/signals";

import { type TerminalPrefs, loadTerminal, saveTerminal, watch } from "./state/prefs.ts";
import { XTERM, cellSize, colorsOf } from "./state/themes.ts";

export const termStyle = signal<TerminalPrefs>(loadTerminal());
watch("terminal", () => (termStyle.value = loadTerminal()));

export function setTermStyle(prefs: TerminalPrefs): void {
  termStyle.value = prefs;
  saveTerminal(prefs);
}

/** The terminal font's character width as a fraction of its size, as last
 * measured: JetBrains Mono's is 0.6. */
export const advance = signal(0.6);

/** Whether the font asked for isn't installed in this browser, so the
 * terminal shows iglu's instead. */
export const fontMissing = signal(false);

let applying: Promise<void> = Promise.resolve();
let latest = 0;

/** Resolves once the style last asked for is on the page, with its font
 * loaded: a terminal measures its cells as it opens. */
export const styleApplied = (): Promise<void> => applying;

export function applyTermStyle(prefs: TerminalPrefs): Promise<void> {
  const turn = ++latest;
  applying = (async () => {
    const family = prefs.font ? `"${prefs.font}", 'JetBrains Mono'` : "'JetBrains Mono'";
    await document.fonts.load(`16px ${family}`).catch(() => []);
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
    set("--term-veil", colors?.selection ? `color-mix(in srgb, ${colors.selection} 60%, transparent)` : null);
    set("--term-pick-ink", colors?.selectionText ?? null);
    // Without one, selected text keeps its own colours.
    root.toggleAttribute("data-term-pick-ink", Boolean(colors?.selectionText));
    (colors?.palette ?? XTERM).forEach((colour, index) => set(`--term-ansi-${index}`, colour));

    const context = document.createElement("canvas").getContext("2d");
    const width = (font: string) => {
      if (!context) return 0;
      context.font = `100px ${font}`;
      return context.measureText("mmmmmwwwwwiiiii").width;
    };
    // A font that isn't there falls back alike whatever comes after it.
    fontMissing.value = prefs.font !== null && width(`"${prefs.font}", monospace`) === width("monospace") && width(`"${prefs.font}", serif`) === width("serif");
    advance.value = width(`${family}, monospace`) / 1500 || 0.6;
    const size = cellSize(advance.value, prefs.size);
    set("--term-face", prefs.font ? `"${prefs.font}", var(--mono)` : null);
    set("--term-size", `${size.font}px`);
    set("--term-row", `${size.row}px`);
  })();
  return applying;
}
