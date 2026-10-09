// Starts the console: who's signed in, the snapshot stream, keys, and the frame.

import { render } from "preact";

import { api, failure } from "./api/client.ts";
import { watch } from "./api/events.ts";
import { App } from "./app.tsx";
import { paintFrost } from "./components/bits.tsx";
import { onPageKey } from "./keyboard.ts";
import "./notify.ts";
import { me, say } from "./state/store.ts";
import { activeScreen, loadGhostty } from "./terminal.ts";

// Read-only access to the focused terminal's text, for the browser test.
Object.assign(globalThis, { iglu: { screen: activeScreen } });

paintFrost();
document.addEventListener("keydown", onPageKey);
const root = document.getElementById("app");
if (root) render(<App />, root);

try {
  me.value = await api.me();
  watch();
  void loadGhostty();
} catch (error) {
  say(`The console couldn't start: ${failure(error)}`);
}
