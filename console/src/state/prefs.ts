// Per-browser conveniences: the look, folded groups, the keyboard and the
// terminal's type and colours. Storage can be missing or refuse writes;
// everything works without it.

import { type Chord, DEFAULT_PREFIX, usablePrefix } from "./keys.ts";
import { THEMES, fontName } from "./themes.ts";

export type Look = "auto" | "dark" | "light";


function read(key: string): unknown {
  try {
    const raw = localStorage.getItem(`iglu.${key}`);
    return raw === null ? null : (JSON.parse(raw) as unknown);
  } catch {
    return null;
  }
}

function write(key: string, value: unknown): void {
  try {
    localStorage.setItem(`iglu.${key}`, JSON.stringify(value));
  } catch {
    // Storage is a convenience; losing it only loses the preference.
  }
}

/** Calls `changed` when another tab of this browser changes the preference,
 * so every tab follows it without a reload. */
export function watch(key: "look" | "collapsed" | "keyboard" | "terminal", changed: () => void): void {
  addEventListener("storage", (event) => {
    if (event.key === `iglu.${key}`) changed();
  });
}

export function loadLook(): Look {
  const value = read("look");
  return value === "dark" || value === "light" ? value : "auto";
}

export const saveLook = (look: Look) => write("look", look);

export function loadCollapsed(): Set<string> {
  const value = read("collapsed");
  return new Set(Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : []);
}

export const saveCollapsed = (keys: ReadonlySet<string>) => write("collapsed", [...keys]);

/** Which Option key a terminal reads as Meta; the other types characters. */
export type OptionAsMeta = "left" | "both" | "off";

export interface KeyboardPrefs {
  prefix: Chord;
  altMoves: boolean;
  optionAsMeta: OptionAsMeta;
}

export const DEFAULT_KEYBOARD: KeyboardPrefs = { prefix: DEFAULT_PREFIX, altMoves: true, optionAsMeta: "left" };

function chordOf(value: unknown): Chord | null {
  if (typeof value !== "object" || value === null) return null;
  const v = value as Record<string, unknown>;
  const flags = ["ctrl", "alt", "shift", "meta"] as const;
  if (typeof v.code !== "string" || !flags.every((f) => typeof v[f] === "boolean")) return null;
  const chord: Chord = { code: v.code, ctrl: v.ctrl as boolean, alt: v.alt as boolean, shift: v.shift as boolean, meta: v.meta as boolean };
  return usablePrefix(chord) ? chord : null;
}

export function loadKeyboard(): KeyboardPrefs {
  const value = read("keyboard");
  if (typeof value !== "object" || value === null) return DEFAULT_KEYBOARD;
  const v = value as Record<string, unknown>;
  return {
    prefix: chordOf(v.prefix) ?? DEFAULT_KEYBOARD.prefix,
    altMoves: typeof v.altMoves === "boolean" ? v.altMoves : DEFAULT_KEYBOARD.altMoves,
    optionAsMeta: v.optionAsMeta === "both" || v.optionAsMeta === "off" || v.optionAsMeta === "left" ? v.optionAsMeta : DEFAULT_KEYBOARD.optionAsMeta,
  };
}

export const saveKeyboard = (prefs: KeyboardPrefs) => write("keyboard", prefs);

export interface TerminalPrefs {
  /** A font installed where the browser runs; none is iglu's, JetBrains Mono. */
  font: string | null;
  /** The text's size, in pixels, before it's nudged to whole-pixel cells. */
  size: number;
  /** "iglu", a theme's name, or "pasted". */
  theme: string;
  /** A theme pasted in Ghostty's format, kept while another is tried. */
  pasted: string;
}

export const DEFAULT_TERMINAL: TerminalPrefs = { font: null, size: 13, theme: "iglu", pasted: "" };

export function loadTerminal(): TerminalPrefs {
  const value = read("terminal");
  if (typeof value !== "object" || value === null) return DEFAULT_TERMINAL;
  const v = value as Record<string, unknown>;
  const theme = typeof v.theme === "string" && (v.theme === "iglu" || v.theme === "pasted" || Object.hasOwn(THEMES, v.theme)) ? v.theme : DEFAULT_TERMINAL.theme;
  return {
    font: typeof v.font === "string" ? fontName(v.font) : null,
    size: typeof v.size === "number" && v.size >= 6 && v.size <= 40 ? v.size : DEFAULT_TERMINAL.size,
    theme,
    pasted: typeof v.pasted === "string" ? v.pasted : "",
  };
}

export const saveTerminal = (prefs: TerminalPrefs) => write("terminal", prefs);
