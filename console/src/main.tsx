// Starts the console: who's signed in, the snapshot stream, keys, and the frame.

import { effect } from "@preact/signals";
import { render } from "preact";

import { api, failure } from "./api/client.ts";
import { watch } from "./api/events.ts";
import { App } from "./app.tsx";
import { paintFrost } from "./components/bits.tsx";
import { onPageKey } from "./keyboard.ts";
import "./notify.ts";
import { look, termStyle } from "./preferences.ts";
import { me, say } from "./state/store.ts";
import { activeScreen, loadTerminals, restyleTerminals } from "./terminal.ts";
import { applyTermStyle } from "./termstyle.ts";

// Read-only access to the focused terminal's text, for the browser test.
Object.assign(globalThis, { iglu: { screen: activeScreen } });

paintFrost();
effect(() => {
  const value = look.value;
  if (value === "auto") delete document.documentElement.dataset["look"];
  else document.documentElement.dataset["look"] = value;
});
// Terminals paint their own colours, so they follow a change of look: the
// one chosen in Settings, once it's applied, or the system's.
effect(() => {
  void look.value;
  queueMicrotask(restyleTerminals);
});
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", restyleTerminals);
// And the type and colours chosen for them, once those are on the page.
effect(() => void applyTermStyle(termStyle.value).then(restyleTerminals));
document.addEventListener("keydown", onPageKey);
const root = document.getElementById("app");
if (root) render(<App />, root);

try {
  me.value = await api.me();
  watch();
  void loadTerminals();
} catch (error) {
  say(`The console couldn't start: ${failure(error)}`);
}
