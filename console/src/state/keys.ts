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
  | { kind: "back" };

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
