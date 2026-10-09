// Turns key events into actions, for the page and for terminals.

import { activeOf, back, cycleWidth, moveColumn, nextWaiting, open, step, stepColumn, toggleFreeze } from "./actions.ts";
import { type Action, type Focus, resolve } from "./state/keys.ts";
import { unreachable } from "./state/unsaved.ts";
import { addingColumn, closing, current, cursor, details, listed, navigate, overlay, renaming, route } from "./state/store.ts";

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
    // The rest act on the open workspace, if there is one.
    case "column":
      if (ws) stepColumn(ws, action.step);
      return;
    case "move-column":
      if (ws) void moveColumn(ws, action.step);
      return;
    case "width":
      if (ws) void cycleWidth(ws);
      return;
    case "close-column":
      if (ws) closing.value = activeOf(ws);
      return;
    case "add-column":
      if (ws?.phase === "running") addingColumn.value = true;
      return;
    case "rename":
      if (ws) renaming.value = true;
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
  return "page";
}

function inputOf(event: KeyboardEvent, focus: Focus) {
  return { key: event.key, code: event.code, alt: event.altKey, meta: event.metaKey, ctrl: event.ctrlKey, shift: event.shiftKey, focus };
}

/** The page's key handler. Terminals handle their own keys through `terminalShortcut`. */
export function onPageKey(event: KeyboardEvent): void {
  const focus = focusOf(event.target);
  if (focus === "terminal") return;
  const action = resolve(inputOf(event, focus));
  if (!action) return;
  event.preventDefault();
  perform(action);
}

/** For ghostty: returns true when the key was a console shortcut, so the terminal drops it. */
export function terminalShortcut(event: KeyboardEvent): boolean {
  if (event.type !== "keydown") return false;
  const action = resolve(inputOf(event, "terminal"));
  if (!action) return false;
  event.preventDefault();
  perform(action);
  return true;
}
