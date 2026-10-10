// The workspace's browser as a column shows it: where on the page a point
// on the screen is, and what a key or a click is to the browser.

import type { BrowserTab } from "../generated/BrowserTab.ts";
import type { KeyInput } from "../generated/KeyInput.ts";
import type { Modifiers } from "../generated/Modifiers.ts";
import type { MouseButton } from "../generated/MouseButton.ts";

interface Size {
  width: number;
  height: number;
}

/** Where on the page a point in the screen's box is, in the page's CSS
 * pixels, or null when it's off the image. The image is shown whole, from
 * the top left, as large as the box allows; `page` is the size it shows. */
export function pagePoint(box: Size, image: Size, page: Size, x: number, y: number): { x: number; y: number } | null {
  if (!image.width || !image.height || !box.width || !box.height) return null;
  const shown = Math.min(box.width / image.width, box.height / image.height);
  const width = image.width * shown;
  const height = image.height * shown;
  if (x < 0 || y < 0 || x > width || y > height) return null;
  return { x: (x / width) * page.width, y: (y / height) * page.height };
}

/** The modifiers held, as the browser should take them. It runs on Linux,
 * where shortcuts use Control, so a Mac's Command is sent as Control. */
export function modifiersOf(event: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }, mac: boolean): Modifiers {
  return {
    alt: event.altKey,
    ctrl: event.ctrlKey || (mac && event.metaKey),
    meta: !mac && event.metaKey,
    shift: event.shiftKey,
  };
}

/** A key for the browser, or null for one an input method is composing
 * with, whose text comes when it's done. */
export function keyOf(event: KeyboardEvent, mac: boolean): KeyInput | null {
  if (event.isComposing || event.key === "Process" || event.key === "Unidentified" || event.key === "Dead") return null;
  // Command is Control to the browser, so the key itself is too.
  const command = mac && event.key === "Meta";
  return {
    action: event.type === "keyup" ? "release" : "press",
    key: command ? "Control" : event.key,
    code: command ? event.code.replace("Meta", "Control") : event.code,
    repeat: event.repeat,
    location: event.location,
    modifiers: modifiersOf(event, mac),
  };
}

/** What a shortcut held with `key` does to the clipboard: the browser's
 * clipboard isn't this one, so copying asks for the selection, and pasting
 * is left to the paste that follows. */
export function clipboardAction(modifiers: Modifiers, key: string): "copy" | "paste" | null {
  if (!modifiers.ctrl || modifiers.alt || modifiers.meta) return null;
  switch (key.toLowerCase()) {
    case "c":
    case "x":
      return "copy";
    case "v":
      return "paste";
    default:
      return null;
  }
}

/** The button a `PointerEvent.button` names. */
export function buttonOf(button: number): MouseButton {
  return (["left", "middle", "right", "back", "forward"] as const)[button] ?? "none";
}

/** A press, for counting a double or triple click. */
export interface Press {
  at: number;
  x: number;
  y: number;
  button: number;
  clicks: number;
}

/** How many presses in a row a press makes: one more than the last when
 * it's the same button, soon after and close by. */
export function clicksAfter(last: Press | null, now: Omit<Press, "clicks">): number {
  const again = last !== null && last.button === now.button && now.at - last.at < 500 && Math.hypot(now.x - last.x, now.y - last.y) < 5;
  return again ? Math.min(last.clicks + 1, 3) : 1;
}

/** A wheel's turn in CSS pixels, whichever unit it came in. */
export function wheelPixels(delta: number, mode: number, pageHeight: number): number {
  switch (mode) {
    case 1:
      return delta * 16;
    case 2:
      return delta * pageHeight;
    default:
      return delta;
  }
}

/** What a tab's address bar shows: nothing for a blank page, where it
 * offers to take an address instead. */
export function addressOf(tab: BrowserTab | undefined): string {
  return !tab || tab.url === "about:blank" ? "" : tab.url;
}

/** A tab's name: its title, or where it is while it has none. */
export function tabName(tab: BrowserTab): string {
  if (tab.title && tab.title !== tab.url) return tab.title;
  if (tab.url === "about:blank" || !tab.url) return "New tab";
  try {
    const url = new URL(tab.url);
    return url.host || tab.url;
  } catch {
    return tab.url;
  }
}
