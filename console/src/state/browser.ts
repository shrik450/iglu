// The workspace's browser as a column shows it, over VNC: what a key is to
// the browser, as an X keysym, which modifiers it's pressed with, and how
// the display is drawn for the screen it's shown on.

/** Keysyms of the keys a KeyboardEvent names, by location where the left
 * and right ones differ. */
const NAMED: Record<string, number | [left: number, right: number]> = {
  Backspace: 0xff08,
  Tab: 0xff09,
  Enter: 0xff0d,
  Pause: 0xff13,
  ScrollLock: 0xff14,
  Escape: 0xff1b,
  Home: 0xff50,
  ArrowLeft: 0xff51,
  ArrowUp: 0xff52,
  ArrowRight: 0xff53,
  ArrowDown: 0xff54,
  PageUp: 0xff55,
  PageDown: 0xff56,
  End: 0xff57,
  PrintScreen: 0xff61,
  Insert: 0xff63,
  ContextMenu: 0xff67,
  NumLock: 0xff7f,
  Delete: 0xffff,
  Shift: [0xffe1, 0xffe2],
  Control: [0xffe3, 0xffe4],
  Alt: [0xffe9, 0xffea],
  Meta: [0xffeb, 0xffec],
  AltGraph: 0xfe03,
};

export const SHIFT = 0xffe1;
export const CONTROL = 0xffe3;
export const ALT = 0xffe9;
export const SUPER = 0xffeb;
const RETURN = 0xff0d;
const TAB = 0xff09;
const KP_ENTER = 0xff8d;
const F1 = 0xffbe;
const LOCATION_RIGHT = 2;
const LOCATION_NUMPAD = 3;

/** The keysym of a character: Latin-1 ones are their own code points, the
 * rest are Unicode keysyms. */
export function charKeysym(char: string): number {
  const point = char.codePointAt(0) ?? 0;
  if ((point >= 0x20 && point <= 0x7e) || (point >= 0xa0 && point <= 0xff)) return point;
  return 0x0100_0000 + point;
}

/** Whether a key is a modifier, which the browser is told of by holding
 * it (see `modifiersFor`) rather than as a key of its own. */
export function isModifier(key: string): boolean {
  return key === "Shift" || key === "Control" || key === "Alt" || key === "Meta" || key === "AltGraph" || key === "CapsLock";
}

/** The keysym of a key as a KeyboardEvent names it, or null for one the
 * browser has no use for. */
export function keysymOf(key: string, location: number): number | null {
  if (key === "Enter" && location === LOCATION_NUMPAD) return KP_ENTER;
  const named = NAMED[key];
  if (named !== undefined) return typeof named === "number" ? named : named[location === LOCATION_RIGHT ? 1 : 0];
  const f = /^F([1-9]|1\d|2[0-4])$/.exec(key);
  if (f) return F1 + Number(f[1]) - 1;
  return [...key].length === 1 ? charKeysym(key) : null;
}

interface Held {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}

/** Whether a key types a character, as its KeyboardEvent names it. */
function printable(key: string): boolean {
  return [...key].length === 1;
}

/** The modifiers the browser should see held with a key. It runs on
 * Linux, where shortcuts use Control, so a Mac's Command is Control; and a
 * Mac's Option that typed a character typed it, so isn't held as Alt. */
export function modifiersFor(event: Held, mac: boolean): number[] {
  const held: number[] = [];
  if (event.shiftKey) held.push(SHIFT);
  if (event.ctrlKey || (mac && event.metaKey)) held.push(CONTROL);
  if (event.altKey && !(mac && printable(event.key))) held.push(ALT);
  if (event.metaKey && !mac) held.push(SUPER);
  return held;
}

/** The key a key event presses, as the browser should take it: a Mac
 * names a letter lowercase while Command is held, Shift or not. */
export function keyFor(event: Held, mac: boolean): string {
  return mac && event.metaKey && event.shiftKey && /^[a-z]$/.test(event.key) ? event.key.toUpperCase() : event.key;
}

/** What a shortcut does with the clipboard: the browser's clipboard isn't
 * this one, so a paste is left to the paste event that follows, which
 * brings this clipboard's text. */
export function clipboardShortcut(event: Held, mac: boolean): "copy" | "paste" | null {
  const command = mac ? event.metaKey || event.ctrlKey : event.ctrlKey;
  if (!command || event.altKey) return null;
  switch (event.key.toLowerCase()) {
    case "c":
    case "x":
      return "copy";
    case "v":
      return "paste";
    default:
      return null;
  }
}

/** The keysyms that type `text`, each pressed and let go in turn; null
 * when it can only be pasted: keysyms past Unicode's first plane, as most
 * emoji are, don't reach the browser whole. */
export function typing(text: string): number[] | null {
  const keys: number[] = [];
  for (const char of text.replaceAll("\r\n", "\n")) {
    if ((char.codePointAt(0) ?? 0) > 0xffff) return null;
    keys.push(char === "\n" ? RETURN : char === "\t" ? TAB : charKeysym(char));
  }
  return keys;
}

/** The scale the display is drawn at for a screen with this pixel ratio:
 * as the display takes it, between 1 and 4 in hundredths. */
export function scaleOf(ratio: number): number {
  return Math.min(4, Math.max(1, Math.round(ratio * 100) / 100));
}

/** How large the display should be for a box of this size, in CSS pixels,
 * at `scale`: in device pixels, so the browser draws as sharp as the
 * screen shows. A box without a whole pixel each way, as a hidden one,
 * has no size for it. */
export function displaySize(box: { w: number; h: number }, scale: number): { w: number; h: number } | null {
  const size = { w: Math.round(box.w * scale), h: Math.round(box.h * scale) };
  return size.w > 0 && size.h > 0 ? size : null;
}

/** RFB encodings, as noVNC numbers them. */
const ZRLE = 16;
const H264 = 50;

/** The encodings to offer the display, most wanted first: H.264 when the
 * display encodes on the host's GPU; otherwise ZRLE, which is lossless,
 * and smallest for text. */
export function preferred(encodings: readonly number[], hardware: boolean): number[] {
  const first = (hardware ? [H264, ZRLE] : [ZRLE]).filter((e) => encodings.includes(e));
  return [...first, ...encodings.filter((e) => !first.includes(e))];
}
