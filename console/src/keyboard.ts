// Turns key events into actions, for the page and for terminals.

import { signal } from "@preact/signals";

import { activeOf, back, columnsOf, cycleWidth, focusColumn, labelOrSay, lastWorkspace, moveColumn, nextWaiting, open, step, stepColumn, toggleFreeze, toggleZoom } from "./actions.ts";
import { type Action, BINDINGS, chordLabel, type Focus, type KeyInput, metaBytes, onKeyboard, prefixBytes, resolve } from "./state/keys.ts";
import { type KeyboardPrefs, loadKeyboard, saveKeyboard, watch } from "./state/prefs.ts";
import { unreachable } from "./state/unsaved.ts";
import { ask, current, cursor, details, listed, navigate, overlay, route } from "./state/store.ts";

/** Whether this keyboard has ⌘ and ⌥, or Ctrl and Alt. */
export const mac = /Mac|iPhone|iPad/.test(navigator.platform);
/** A touch screen with no mouse: shortcut hints are no help, and a terminal
 * needs keys the on-screen keyboard lacks. */
export const touchOnly = matchMedia("(hover: none) and (pointer: coarse)").matches;

/** This browser's keyboard settings. */
export const keyboard = signal<KeyboardPrefs>(loadKeyboard());
watch("keyboard", () => (keyboard.value = loadKeyboard()));

export function setKeyboard(prefs: KeyboardPrefs): void {
  keyboard.value = prefs;
  saveKeyboard(prefs);
}

/** The prefix and a key after it, for hints: "⌃Space 3". */
export const afterPrefix = (key: string) => `${chordLabel(keyboard.value.prefix, mac)} ${key}`;

/** How to press an action from anywhere, for hints: "⌃Space n". A kind
 * alone means its first binding; a whole action, the binding for exactly it. */
export function keysFor(action: Action["kind"] | Action): string | undefined {
  if (action === "palette") return onKeyboard("⌘K", mac);
  const wanted = typeof action === "string" ? (b: Action) => b.kind === action : (b: Action) => JSON.stringify(b) === JSON.stringify(action);
  const after = BINDINGS.find((b) => wanted(b.action) && b.after)?.after;
  return after ? afterPrefix(after.label) : undefined;
}

/** The prefix was pressed: the next key is the console's. */
export const armed = signal(false);
// It waits for that key only: a click, or leaving the window, puts it away,
// so a later key typed into a field isn't taken as a command.
window.addEventListener("pointerdown", () => (armed.value = false), { capture: true });
window.addEventListener("blur", () => (armed.value = false));

export function perform(action: Action): void {
  const ws = current.value;
  switch (action.kind) {
    case "palette":
      overlay.value = overlay.value === "palette" ? null : "palette";
      return;
    case "new":
      overlay.value = "new";
      return;
    case "next-waiting":
      nextWaiting();
      return;
    case "workspace":
      step(action.step);
      return;
    case "previews":
      navigate(route.value.view === "previews" ? { view: "overview" } : { view: "previews" });
      return;
    case "open": {
      const target = listed.value.find((w) => w.id === cursor.value) ?? listed.value[0];
      if (!ws && target) open(target);
      return;
    }
    case "back":
      back();
      return;
    case "keys":
      overlay.value = overlay.value === "keys" ? null : "keys";
      return;
    // The rest act on the open workspace, if there is one.
    case "column":
      if (ws) stepColumn(ws, action.step);
      return;
    case "column-at": {
      const column = ws && columnsOf(ws)[action.index];
      if (ws && column) focusColumn(ws, column.name);
      return;
    }
    case "move-column":
      if (ws) void moveColumn(ws, action.step);
      return;
    case "width":
      if (ws) void cycleWidth(ws);
      return;
    case "close-column": {
      const column = ws && activeOf(ws);
      if (ws && column) ask(ws, { kind: "end", column });
      return;
    }
    case "add-column":
      if (ws?.phase === "running") ask(ws, { kind: "add-column" });
      return;
    case "zoom":
      if (ws) toggleZoom(ws);
      return;
    case "last-workspace":
      lastWorkspace();
      return;
    case "label-column": {
      const column = ws && activeOf(ws);
      if (ws && column) labelOrSay(ws, column);
      return;
    }
    case "rename":
      if (ws) ask(ws, { kind: "rename" });
      return;
    case "freeze":
      if (ws) void toggleFreeze(ws);
      return;
    case "details":
      if (ws) details.value = !details.value;
      return;
    default:
      unreachable(action);
  }
}

function focusOf(target: EventTarget | null): Focus {
  if (!(target instanceof Element)) return "page";
  if (target.closest(".term-host")) return "terminal";
  if (target.closest("input, textarea, select, [contenteditable='true']")) return "field";
  if (target.closest("button, a[href], summary, [role='button'], [role='option'], dialog")) return "control";
  return "page";
}

function inputOf(event: KeyboardEvent, focus: Focus): KeyInput {
  return {
    key: event.key,
    code: event.code,
    alt: event.altKey,
    meta: event.metaKey,
    ctrl: event.ctrlKey,
    shift: event.shiftKey,
    focus,
    inWorkspace: route.value.view === "workspace",
  };
}

/** Which Option keys are down: a key event says Option is held, not which. */
const options = new Set<string>();
for (const type of ["keydown", "keyup"] as const) {
  window.addEventListener(
    type,
    (event) => {
      if (event.code === "AltLeft" || event.code === "AltRight") {
        if (type === "keydown") options.add(event.code);
        else options.delete(event.code);
      }
    },
    { capture: true },
  );
}
window.addEventListener("blur", () => options.clear());

/** Whether the Option held now is one the person reads as Meta. */
function optionIsMeta(): boolean {
  switch (keyboard.value.optionAsMeta) {
    case "off":
      return false;
    case "both":
      return true;
    case "left":
      return options.has("AltLeft") || !mac;
    default:
      return unreachable(keyboard.value.optionAsMeta);
  }
}

/** Does what the keymap says for a key; returns whether the console took it.
 * `send` writes to the focused terminal, when one has focus. */
function handle(event: KeyboardEvent, focus: Focus, send?: (text: string) => void): boolean {
  if (event.type !== "keydown" || event.isComposing) return false;
  const input = inputOf(event, focus);
  const outcome = resolve(armed.value ? "prefix" : "normal", input, { mac, ...keyboard.value });
  switch (outcome.kind) {
    case "pass": {
      if (!send || !input.alt || input.ctrl || input.meta || !optionIsMeta()) return false;
      const bytes = metaBytes(input.code, input.shift);
      if (!bytes) return false;
      event.preventDefault();
      send(bytes);
      return true;
    }
    case "arm":
      armed.value = true;
      break;
    case "hold":
      break;
    case "cancel":
      armed.value = false;
      break;
    case "send-prefix": {
      armed.value = false;
      const bytes = prefixBytes(keyboard.value.prefix);
      if (send && bytes) send(bytes);
      break;
    }
    case "act":
      armed.value = false;
      perform(outcome.action);
      break;
    default:
      unreachable(outcome);
  }
  event.preventDefault();
  return true;
}

/** The page's key handler. Terminals handle their own keys through `terminalKey`. */
export function onPageKey(event: KeyboardEvent): void {
  const focus = focusOf(event.target);
  if (focus !== "terminal") handle(event, focus);
}

/** For a terminal: returns true when the console took the key, so the terminal never sees it. */
export function terminalKey(event: KeyboardEvent, send: (text: string) => void): boolean {
  return handle(event, "terminal", send);
}
