// The person's preferences: the look, the keyboard and the terminal's style.
// iglud keeps them, so they follow the person to every browser; each
// snapshot brings them, so a change anywhere reaches every open console.
//
// The last ones seen are kept in the browser too, only so the first paint
// has them before the first snapshot does.

import { computed, signal } from "@preact/signals";

import { api, failure } from "./api/client.ts";
import type { Keyboard } from "./generated/Keyboard.ts";
import type { Look } from "./generated/Look.ts";
import type { Preferences } from "./generated/Preferences.ts";
import type { TerminalStyle } from "./generated/TerminalStyle.ts";
import { DEFAULT_PREFIX, usablePrefix } from "./state/keys.ts";
import { say } from "./state/store.ts";

/** What iglud starts everyone with, as iglu-domain's preferences.rs has it. */
const DEFAULTS: Preferences = {
  look: "auto",
  keyboard: { prefix: DEFAULT_PREFIX, alt_moves: true, option_as_meta: "left" },
  terminal: { font: null, size: 13, theme: "iglu", pasted: "" },
};

const CACHE = "iglu.preferences";

function cached(): Preferences | null {
  try {
    const raw = localStorage.getItem(CACHE);
    return raw === null ? null : (JSON.parse(raw) as Preferences);
  } catch {
    return null;
  }
}

function cache(preferences: Preferences): void {
  try {
    localStorage.setItem(CACHE, JSON.stringify(preferences));
  } catch {
    // Only the first paint of the next visit misses it.
  }
}

export const preferences = signal<Preferences>(cached() ?? DEFAULTS);

/** What iglud last said, to go back to if a change can't be kept. */
let kept: Preferences = preferences.peek();
/** A save waiting for typing to pause. */
let waiting: number | null = null;
let sending = false;
/** Whether something changed while a save was on its way. */
let again = false;

/** From a snapshot. A change not yet saved stays, rather than flickering
 * back to what iglud had before it. */
export function receive(from: Preferences): void {
  kept = from;
  if (waiting !== null || sending) return;
  preferences.value = from;
  cache(from);
}

/** Shows a change at once and keeps it: at once for a choice, and once
 * typing pauses for text, so a theme being typed isn't saved on every key. */
export function setPreferences(next: Preferences, typing = false): void {
  preferences.value = next;
  cache(next);
  if (waiting !== null) window.clearTimeout(waiting);
  waiting = null;
  if (typing) waiting = window.setTimeout(save, 400);
  else save();
}

/** One save at a time, of the latest, so they can't land out of order. */
function save(): void {
  waiting = null;
  if (sending) {
    again = true;
    return;
  }
  sending = true;
  const sent = preferences.peek();
  api
    .putPreferences(sent)
    .then(() => {
      kept = sent;
    })
    .catch((error: unknown) => {
      say(`That setting couldn't be kept: ${failure(error)}`);
      if (waiting === null && !again) {
        preferences.value = kept;
        cache(kept);
      }
    })
    .finally(() => {
      sending = false;
      if (again) {
        again = false;
        save();
      }
    });
}

// Leaving with typing unsaved saves it on the way out.
addEventListener("pagehide", () => {
  if (waiting === null && !again) return;
  window.clearTimeout(waiting ?? undefined);
  waiting = null;
  again = false;
  void api.putPreferences(preferences.peek()).catch(() => undefined);
});

export const look = computed<Look>(() => preferences.value.look);
export const setLook = (value: Look) => setPreferences({ ...preferences.peek(), look: value });

/** The keyboard, with a prefix the console can use whatever was stored. */
export const keyboard = computed<Keyboard>(() => {
  const chosen = preferences.value.keyboard;
  return usablePrefix(chosen.prefix) ? chosen : { ...chosen, prefix: DEFAULT_PREFIX };
});
export const setKeyboard = (value: Keyboard) => setPreferences({ ...preferences.peek(), keyboard: value });

export const termStyle = computed<TerminalStyle>(() => preferences.value.terminal);
export const setTermStyle = (value: TerminalStyle, typing = false) => setPreferences({ ...preferences.peek(), terminal: value }, typing);
