// The console's keymap. Pure: keyboard.ts feeds it key events and does what
// it returns.
//
// Terminals own the keyboard: shells and agents bind nearly every Ctrl and
// Option chord. The console takes a key only when
// - it's ⌘K on a Mac, which no terminal program receives (Ctrl+K elsewhere,
//   outside terminals);
// - it follows the prefix, ⌃Space unless the person chose another, as in tmux;
// - it's ⌘F on a Mac in a terminal, to find in its output (Ctrl+Shift+F
//   elsewhere, which programs rarely bind);
// - it's ⌥H/J/K/L and the person lets the console take those from terminals;
// - or it's a plain key on a page where nothing interactive has focus,
//   outside a workspace, such as j/k and ↩ on the overview. A focused button,
//   link or dialog keeps its own keys, so ↩ presses the button.

export type Action =
  | { kind: "palette" }
  | { kind: "new" }
  | { kind: "next-waiting" }
  | { kind: "workspace"; step: -1 | 1 }
  | { kind: "column"; step: -1 | 1 }
  | { kind: "column-at"; index: number }
  | { kind: "move-column"; step: -1 | 1 }
  | { kind: "width" }
  | { kind: "close-column" }
  | { kind: "label-column" }
  | { kind: "zoom" }
  | { kind: "find" }
  | { kind: "last-workspace" }
  | { kind: "add-column" }
  | { kind: "rename" }
  | { kind: "freeze" }
  | { kind: "details" }
  | { kind: "previews" }
  | { kind: "open" }
  | { kind: "back" }
  | { kind: "keys" };

/** What has the keyboard: nothing in particular, a terminal, a text field,
 * or another control such as a button, link or dialog. */
export type Focus = "page" | "terminal" | "field" | "control";

export interface KeyInput {
  key: string;
  code: string;
  alt: boolean;
  meta: boolean;
  ctrl: boolean;
  shift: boolean;
  focus: Focus;
  /** Whether a workspace is open: then plain keys are never the console's. */
  inWorkspace: boolean;
}

/** A chord as a person sets it: a physical key and its modifiers. */
export interface Chord {
  code: string;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

export interface Keymap {
  mac: boolean;
  prefix: Chord;
  /** Take ⌥H/J/K/L from terminals to move between columns and workspaces. */
  altMoves: boolean;
}

export const DEFAULT_PREFIX: Chord = { code: "Space", ctrl: true, alt: false, shift: false, meta: false };

export type Mode = "normal" | "prefix";

export type Outcome =
  /** Not the console's: the terminal or the page has it. */
  | { kind: "pass" }
  | { kind: "act"; action: Action }
  /** The prefix: the next key is the console's. */
  | { kind: "arm" }
  /** A modifier pressed after the prefix; still waiting for the key. */
  | { kind: "hold" }
  /** The prefix twice: the terminal gets the prefix itself. */
  | { kind: "send-prefix" }
  /** After the prefix, Escape or a key that means nothing. */
  | { kind: "cancel" };

interface Key {
  code: string;
  shift?: boolean;
  /** As shown: "h", "⇧H", "↩". */
  label: string;
}

export interface Binding {
  action: Action;
  does: string;
  group: "Anywhere" | "Workspaces" | "Columns";
  /** After the prefix. */
  after?: Key;
  /** Taken from terminals when the person allows it. */
  alt?: Key;
  /** On a page with nothing to type into, outside a workspace. */
  page?: readonly Key[];
}

const letter = (l: string, shift = false): Key => ({ code: `Key${l.toUpperCase()}`, shift, label: shift ? `⇧${l.toUpperCase()}` : l });

export const BINDINGS: readonly Binding[] = [
  { action: { kind: "palette" }, does: "Search and commands", group: "Anywhere", after: { code: "Slash", label: "/" }, page: [{ code: "Slash", label: "/" }] },
  { action: { kind: "new" }, does: "New workspace", group: "Anywhere", after: letter("n"), page: [letter("n")] },
  { action: { kind: "next-waiting" }, does: "Next workspace that needs you", group: "Anywhere", after: letter("u"), page: [letter("u")] },
  { action: { kind: "previews" }, does: "Previews", group: "Anywhere", after: letter("p"), page: [letter("p")] },
  { action: { kind: "keys" }, does: "All shortcuts", group: "Anywhere", after: { code: "Slash", shift: true, label: "?" }, page: [{ code: "Slash", shift: true, label: "?" }] },
  {
    action: { kind: "workspace", step: 1 },
    does: "Next workspace",
    group: "Workspaces",
    after: letter("j"),
    alt: letter("j"),
    page: [letter("j"), { code: "ArrowDown", label: "↓" }],
  },
  {
    action: { kind: "workspace", step: -1 },
    does: "Previous workspace",
    group: "Workspaces",
    after: letter("k"),
    alt: letter("k"),
    page: [letter("k"), { code: "ArrowUp", label: "↑" }],
  },
  { action: { kind: "open" }, does: "Open the selected workspace", group: "Workspaces", page: [{ code: "Enter", label: "↩" }] },
  { action: { kind: "back" }, does: "Back to the overview", group: "Workspaces", page: [{ code: "Escape", label: "esc" }] },
  { action: { kind: "last-workspace" }, does: "The workspace you were in before", group: "Workspaces", after: { code: "Semicolon", label: ";" } },
  { action: { kind: "rename" }, does: "Rename", group: "Workspaces", after: letter("r") },
  { action: { kind: "freeze" }, does: "Freeze or thaw", group: "Workspaces", after: letter("f") },
  { action: { kind: "details" }, does: "Details", group: "Workspaces", after: letter("i") },
  { action: { kind: "column", step: -1 }, does: "Previous column", group: "Columns", after: letter("h"), alt: letter("h") },
  { action: { kind: "column", step: 1 }, does: "Next column", group: "Columns", after: letter("l"), alt: letter("l") },
  { action: { kind: "move-column", step: -1 }, does: "Move the column left", group: "Columns", after: letter("h", true) },
  { action: { kind: "move-column", step: 1 }, does: "Move the column right", group: "Columns", after: letter("l", true) },
  { action: { kind: "width" }, does: "Change its width", group: "Columns", after: letter("w") },
  { action: { kind: "zoom" }, does: "Zoom it to fill the page, or put it back", group: "Columns", after: letter("z") },
  { action: { kind: "find" }, does: "Find in its output", group: "Columns", after: letter("s") },
  { action: { kind: "add-column" }, does: "New column", group: "Columns", after: letter("c") },
  { action: { kind: "label-column" }, does: "Rename the column", group: "Columns", after: { code: "Comma", label: "," } },
  { action: { kind: "close-column" }, does: "End the column", group: "Columns", after: letter("x") },
];

const matches = (key: Key, input: KeyInput) => key.code === input.code && Boolean(key.shift) === input.shift;

export const sameChord = (input: Omit<Chord, "code"> & { code: string }, chord: Chord) =>
  input.code === chord.code && input.ctrl === chord.ctrl && input.alt === chord.alt && input.shift === chord.shift && input.meta === chord.meta;

const MODIFIERS = new Set(["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock", "Fn"]);

const act = (action: Action): Outcome => ({ kind: "act", action });

export function resolve(mode: Mode, input: KeyInput, keymap: Keymap): Outcome {
  if (MODIFIERS.has(input.key)) return mode === "prefix" ? { kind: "hold" } : { kind: "pass" };

  if (mode === "prefix") {
    if (sameChord(input, keymap.prefix)) return { kind: "send-prefix" };
    if (input.ctrl || input.meta || input.alt) return { kind: "cancel" };
    const digit = /^Digit([1-9])$/.exec(input.code);
    if (digit) return act({ kind: "column-at", index: Number(digit[1]) - 1 });
    const binding = BINDINGS.find((b) => b.after && matches(b.after, input));
    return binding ? act(binding.action) : { kind: "cancel" };
  }

  if (sameChord(input, keymap.prefix)) return { kind: "arm" };

  // ⌘ never reaches a terminal program; Ctrl+K is the shell's kill-line, so
  // off a Mac it's the palette only outside terminals.
  const command = keymap.mac ? input.meta && !input.ctrl : input.ctrl && !input.meta && input.focus !== "terminal";
  if (command && input.code === "KeyK" && !input.alt && !input.shift) return act({ kind: "palette" });

  const find = keymap.mac ? input.meta && !input.ctrl && !input.shift : input.ctrl && input.shift && !input.meta;
  if (find && input.focus === "terminal" && input.code === "KeyF" && !input.alt) return act({ kind: "find" });

  if (keymap.altMoves && input.alt && !input.ctrl && !input.meta && input.focus !== "field") {
    const binding = BINDINGS.find((b) => b.alt && matches(b.alt, input));
    if (binding) return act(binding.action);
  }

  if (input.focus === "page" && !input.inWorkspace && !input.ctrl && !input.meta && !input.alt) {
    const binding = BINDINGS.find((b) => b.page?.some((k) => matches(k, input)));
    if (binding) return act(binding.action);
  }
  return { kind: "pass" };
}

/** A chord as written on this keyboard: ⌃Space on a Mac, Ctrl+Space elsewhere. */
/** Keys whose code names don't say what's printed on them. */
const PRINTED: Readonly<Record<string, string>> = { BracketLeft: "[", BracketRight: "]", Backslash: "\\", Slash: "/", Period: ".", Comma: ",", Semicolon: ";", Quote: "'", Backquote: "`", Minus: "-", Equal: "=" };

export function chordLabel(chord: Chord, mac: boolean): string {
  const key = PRINTED[chord.code] ?? chord.code.replace(/^Key|^Digit/, "");
  const mods = mac
    ? `${chord.ctrl ? "⌃" : ""}${chord.alt ? "⌥" : ""}${chord.shift ? "⇧" : ""}${chord.meta ? "⌘" : ""}`
    : `${chord.ctrl ? "Ctrl+" : ""}${chord.alt ? "Alt+" : ""}${chord.shift ? "Shift+" : ""}${chord.meta ? "Win+" : ""}`;
  return `${mods}${key}`;
}

/** What a terminal receives for the prefix when it's pressed twice. */
export function prefixBytes(chord: Chord): string | null {
  if (!chord.ctrl || chord.alt || chord.meta) return null;
  if (chord.code === "Space") return "\x00";
  const letter = /^Key([A-Z])$/.exec(chord.code);
  if (letter) return String.fromCharCode(letter[1]!.charCodeAt(0) - 64);
  const symbols: Record<string, string> = { BracketLeft: "\x1b", Backslash: "\x1c", BracketRight: "\x1d" };
  return symbols[chord.code] ?? null;
}

/** Whether a chord can be the prefix: a Ctrl chord a terminal can receive. */
export const usablePrefix = (chord: Chord) => prefixBytes(chord) !== null;

/** Keys as written on this keyboard: a Mac's symbols, or their names elsewhere. */
export function onKeyboard(keys: string, mac: boolean): string {
  return mac ? keys : keys.replaceAll("⌘", "Ctrl+").replaceAll("⌥", "Alt+").replaceAll("⇧", "Shift+").replaceAll("⌃", "Ctrl+").replace(/\+ /g, "+");
}

// US-layout characters for Option-as-Meta: what the key types, without and with Shift.
const PUNCTUATION: Record<string, [string, string]> = {
  Minus: ["-", "_"],
  Equal: ["=", "+"],
  BracketLeft: ["[", "{"],
  BracketRight: ["]", "}"],
  Backslash: ["\\", "|"],
  Semicolon: [";", ":"],
  Quote: ["'", '"'],
  Comma: [",", "<"],
  Period: [".", ">"],
  Slash: ["/", "?"],
  Backquote: ["`", "~"],
  Space: [" ", " "],
};
const SHIFTED_DIGITS = ")!@#$%^&*(";

/** Option held as Meta: ESC, then what the key types without Option. Null
 * for keys the terminal already encodes, such as arrows and Backspace. */
export function metaBytes(code: string, shift: boolean): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return `\x1b${shift ? letter[1]! : letter[1]!.toLowerCase()}`;
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return `\x1b${shift ? SHIFTED_DIGITS[Number(digit[1])]! : digit[1]!}`;
  const symbol = PUNCTUATION[code];
  return symbol ? `\x1b${symbol[shift ? 1 : 0]}` : null;
}

/** What Ctrl makes of a typed character: a letter, or one of @ [ \\ ] ^ _ ?,
 * becomes its control character, as a terminal sends it. Anything else, or
 * more than one character, goes as it is. */
export function withCtrl(typed: string): string {
  if (typed.length !== 1) return typed;
  if (typed === "?") return "\x7f";
  const code = typed.toUpperCase().charCodeAt(0);
  return code >= 0x40 && code <= 0x5f ? String.fromCharCode(code & 0x1f) : typed;
}
