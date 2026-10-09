// The keyboard shortcuts: the full sheet, and the hint shown after the prefix.
// Both read the keymap's own table, so they can't disagree with it.

import { armed, keyboard, mac } from "../keyboard.ts";
import { type Binding, BINDINGS, chordLabel, onKeyboard } from "../state/keys.ts";
import { overlay } from "../state/store.ts";
import { Modal } from "./Modal.tsx";

const GROUPS = ["Anywhere", "Workspaces", "Columns"] as const;

function Chord({ keys }: { keys: string }) {
  return (
    <span class="chord">
      {keys.split(" ").map((k) => (
        <kbd key={k}>{k}</kbd>
      ))}
    </span>
  );
}

function ways(binding: Binding, prefix: string): string[] {
  const all: string[] = [];
  if (binding.action.kind === "palette") all.push(onKeyboard("⌘ K", mac));
  if (binding.after) all.push(`${prefix} ${binding.after.label}`);
  if (binding.alt && keyboard.value.altMoves) all.push(onKeyboard(`⌥${binding.alt.label.toUpperCase()}`, mac));
  return all;
}

export function Keys() {
  const close = () => (overlay.value = null);
  const prefix = chordLabel(keyboard.value.prefix, mac);
  return (
    <Modal labelledby="keys-h" onClose={close}>
      <div class="kbox">
        <div class="khead">
          <h2 id="keys-h">Keyboard shortcuts</h2>
          <button type="button" class="btn icon" aria-label="Close" autofocus onClick={close}>
            ×
          </button>
        </div>
        <p class="muted">
          {keyboard.value.altMoves ? (
            <>
              Terminals keep every key but <Chord keys={onKeyboard("⌥H ⌥J ⌥K ⌥L", mac)} />, which move between columns and workspaces; Settings can give
              those back.{" "}
            </>
          ) : (
            "Terminals keep every key. "
          )}
          To reach iglu from one, press <Chord keys={prefix} />, then a key. Pressing it twice sends <Chord keys={prefix} /> to the terminal. On the
          overview, the keys work without it.
        </p>
        <div class="kgroups">
          {GROUPS.map((group) => (
            <section key={group}>
              <h3>{group}</h3>
              <dl>
                {BINDINGS.filter((b) => b.group === group).map((b) => (
                  <div key={b.does}>
                    <dt>
                      {ways(b, prefix).map((w) => (
                        <Chord key={w} keys={w} />
                      ))}
                      {b.page && !b.after ? b.page.map((k) => <Chord key={k.label} keys={k.label} />) : null}
                    </dt>
                    <dd>{b.does}</dd>
                  </div>
                ))}
                {group === "Columns" ? (
                  <div>
                    <dt>
                      <Chord keys={`${prefix} 1–9`} />
                    </dt>
                    <dd>Go to a column</dd>
                  </div>
                ) : null}
              </dl>
            </section>
          ))}
        </div>
      </div>
    </Modal>
  );
}

/** After the prefix: what the next key can do, columns first since they're
 * what a terminal is among. */
export function PrefixHint() {
  if (!armed.value) return null;
  const prefix = chordLabel(keyboard.value.prefix, mac);
  return (
    <div class="prefix-hint" role="status" aria-label="iglu is waiting for a key">
      {(["Columns", "Workspaces", "Anywhere"] as const).map((group) => (
        <section key={group}>
          <h3>{group}</h3>
          <ul>
            {BINDINGS.flatMap((b) =>
              b.after && b.group === group
                ? [
                    <li key={b.does}>
                      <kbd>{b.after.label}</kbd> {b.does}
                    </li>,
                  ]
                : [],
            )}
            {group === "Columns" ? (
              <li>
                <kbd>1–9</kbd> Go to a column
              </li>
            ) : null}
            {group === "Anywhere" ? (
              <>
                <li>
                  <kbd>{prefix}</kbd> Send {prefix} to the terminal
                </li>
                <li>
                  <kbd>esc</kbd> Cancel
                </li>
              </>
            ) : null}
          </ul>
        </section>
      ))}
    </div>
  );
}
