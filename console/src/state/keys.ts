// The console's keymap. Pure: main.tsx feeds it key events and performs the action.
//
// With Option (Alt) held, a shortcut works from anywhere, terminals included.
// Plain keys only work when focus is on the page itself, so typing always
// reaches a terminal or a field.

export type Action =
  | { kind: "palette" }
  | { kind: "new" }
  | { kind: "next-waiting" }
  | { kind: "workspace"; step: -1 | 1 }
  | { kind: "column"; step: -1 | 1 }
  | { kind: "move-column"; step: -1 | 1 }
  | { kind: "width" }
  | { kind: "close-column" }
  | { kind: "add-column" }
  | { kind: "rename" }
  | { kind: "freeze" }
  | { kind: "details" }
  | { kind: "previews" }
  | { kind: "open" }
  | { kind: "back" }
  | { kind: "keys" };

export type Focus = "page" | "terminal" | "field";

export interface KeyInput {
  key: string;
  code: string;
  alt: boolean;
  meta: boolean;
  ctrl: boolean;
  shift: boolean;
  focus: Focus;
}

/** Letter shortcuts by physical key, so Option+letter works on a Mac too. */
function letter(code: string, shift: boolean, alt: boolean): Action | null {
  switch (code) {
    case "KeyJ":
      return { kind: "workspace", step: 1 };
    case "KeyK":
      return { kind: "workspace", step: -1 };
    case "KeyH":
      return shift ? { kind: "move-column", step: -1 } : { kind: "column", step: -1 };
    case "KeyL":
      return shift ? { kind: "move-column", step: 1 } : { kind: "column", step: 1 };
    case "KeyW":
      return { kind: "width" };
    case "KeyX":
      return { kind: "close-column" };
    case "KeyA":
      return { kind: "add-column" };
    case "KeyE":
      return { kind: "rename" };
    case "KeyF":
      return { kind: "freeze" };
    case "KeyI":
      return { kind: "details" };
    case "KeyP":
      return { kind: "previews" };
    case "KeyN":
      return alt ? { kind: "next-waiting" } : { kind: "new" };
    default:
      return null;
  }
}

export function resolve(input: KeyInput): Action | null {
  if ((input.meta || input.ctrl) && input.code === "KeyK" && !input.alt) return { kind: "palette" };
  if (input.meta || input.ctrl) return null;
  if (input.focus === "field") return null;
  if (input.alt) return letter(input.code, input.shift, true);
  if (input.focus === "terminal") return null;
  switch (input.key) {
    case "Enter":
      return { kind: "open" };
    case "Escape":
      return { kind: "back" };
    case "/":
      return { kind: "palette" };
    case "?":
      return { kind: "keys" };
    case "ArrowDown":
      return { kind: "workspace", step: 1 };
    case "ArrowUp":
      return { kind: "workspace", step: -1 };
    case "ArrowRight":
      return { kind: "column", step: 1 };
    case "ArrowLeft":
      return { kind: "column", step: -1 };
    default:
      return letter(input.code, input.shift, false);
  }
}

/** The shortcut sheet: what each key does, as `resolve` does it. Keys are
 * written as shown; `code` and `shift` say how to press them. A test checks
 * every entry against `resolve`, so the sheet can't drift from the keymap. */
export interface Shortcut {
  keys: string;
  does: string;
  press: Pick<KeyInput, "key" | "code" | "shift">;
  action: Action["kind"];
}

export const SHEET: readonly { group: string; shortcuts: readonly Shortcut[] }[] = [
  {
    group: "Anywhere",
    shortcuts: [
      { keys: "⌘ K", does: "Search and commands", press: { key: "k", code: "KeyK", shift: false }, action: "palette" },
      { keys: "n", does: "New workspace", press: { key: "n", code: "KeyN", shift: false }, action: "new" },
      { keys: "⌥ N", does: "Next workspace that needs you", press: { key: "n", code: "KeyN", shift: false }, action: "next-waiting" },
      { keys: "p", does: "Previews", press: { key: "p", code: "KeyP", shift: false }, action: "previews" },
      { keys: "?", does: "These shortcuts", press: { key: "?", code: "Slash", shift: true }, action: "keys" },
    ],
  },
  {
    group: "Workspaces",
    shortcuts: [
      { keys: "j k", does: "Next and previous workspace", press: { key: "j", code: "KeyJ", shift: false }, action: "workspace" },
      { keys: "↩", does: "Open the one under the cursor", press: { key: "Enter", code: "Enter", shift: false }, action: "open" },
      { keys: "esc", does: "Back to the overview", press: { key: "Escape", code: "Escape", shift: false }, action: "back" },
      { keys: "f", does: "Freeze or thaw", press: { key: "f", code: "KeyF", shift: false }, action: "freeze" },
      { keys: "e", does: "Rename", press: { key: "e", code: "KeyE", shift: false }, action: "rename" },
      { keys: "i", does: "Details", press: { key: "i", code: "KeyI", shift: false }, action: "details" },
    ],
  },
  {
    group: "Columns",
    shortcuts: [
      { keys: "h l", does: "Previous and next column", press: { key: "h", code: "KeyH", shift: false }, action: "column" },
      { keys: "⇧H ⇧L", does: "Move the column", press: { key: "H", code: "KeyH", shift: true }, action: "move-column" },
      { keys: "w", does: "Change its width", press: { key: "w", code: "KeyW", shift: false }, action: "width" },
      { keys: "a", does: "Add a column", press: { key: "a", code: "KeyA", shift: false }, action: "add-column" },
      { keys: "x", does: "End the column", press: { key: "x", code: "KeyX", shift: false }, action: "close-column" },
    ],
  },
];

/** Keys as written on this keyboard: a Mac's symbols, or their names elsewhere. */
export function onKeyboard(keys: string, mac: boolean): string {
  return mac ? keys : keys.replaceAll("⌘", "Ctrl+").replaceAll("⌥", "Alt+").replaceAll("⇧", "Shift+").replace(/\+ /g, "+");
}
