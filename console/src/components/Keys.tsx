// The keyboard shortcuts: the full sheet, and the hint shown after the prefix.
// Both read the keymap's own table, so they can't disagree with it.

import { armed, mac } from "../keyboard.ts";
import { m } from "../messages.tsx";
import { keyboard } from "../preferences.ts";
import { type Binding, BINDINGS, chordLabel, onKeyboard } from "../state/keys.ts";
import { overlay } from "../state/store.ts";
import { DialogHead, Modal } from "./Modal.tsx";

const GROUPS = ["anywhere", "workspaces", "columns"] as const;

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
  if (binding.action.kind === "find") all.push(onKeyboard(mac ? "⌘ F" : "⌃⇧ F", mac));
  if (binding.after) all.push(`${prefix} ${binding.after.label}`);
  if (binding.alt && keyboard.value.alt_moves) all.push(onKeyboard(`⌥${binding.alt.label.toUpperCase()}`, mac));
  return all;
}

export function Keys() {
  const close = () => (overlay.value = null);
  const prefix = chordLabel(keyboard.value.prefix, mac);
  return (
    <Modal labelledby="keys-h" onClose={close}>
      <div class="kbox">
        <DialogHead id="keys-h" title={m.keys.title} onClose={close} autofocus />
        <p class="muted">{m.keys.about(keyboard.value.alt_moves, <Chord keys={onKeyboard("⌥H ⌥J ⌥K ⌥L", mac)} />, <Chord keys={prefix} />)}</p>
        <div class="kgroups">
          {GROUPS.map((group) => (
            <section key={group}>
              <h3>{m.keys.group(group)}</h3>
              <dl>
                {BINDINGS.filter((b) => b.group === group).map((b) => (
                  <div key={JSON.stringify(b.action)}>
                    <dt>
                      {ways(b, prefix).map((w) => (
                        <Chord key={w} keys={w} />
                      ))}
                      {b.page && !b.after ? b.page.map((k) => <Chord key={k.label} keys={k.label} />) : null}
                    </dt>
                    <dd>{m.keys.does(b.action)}</dd>
                  </div>
                ))}
                {group === "columns" ? (
                  <>
                    <div>
                      <dt>
                        <Chord keys={`${prefix} 1–9`} />
                      </dt>
                      <dd>{m.keys.toColumn}</dd>
                    </div>
                    <div>
                      <dt>
                        <Chord keys={onKeyboard("⌘ click", mac)} />
                      </dt>
                      <dd>{m.keys.openLink}</dd>
                    </div>
                  </>
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
    <div class="prefix-hint" role="status" aria-label={m.keys.waiting}>
      {(["columns", "workspaces", "anywhere"] as const).map((group) => (
        <section key={group}>
          <h3>{m.keys.group(group)}</h3>
          <ul>
            {BINDINGS.flatMap((b) =>
              b.after && b.group === group
                ? [
                    <li key={JSON.stringify(b.action)}>
                      <kbd>{b.after.label}</kbd> {m.keys.does(b.action)}
                    </li>,
                  ]
                : [],
            )}
            {group === "columns" ? (
              <li>
                <kbd>1–9</kbd> {m.keys.toColumn}
              </li>
            ) : null}
            {group === "anywhere" ? (
              <>
                <li>
                  <kbd>{prefix}</kbd> {m.keys.sendPrefix(prefix)}
                </li>
                <li>
                  <kbd>esc</kbd> {m.keys.cancel}
                </li>
              </>
            ) : null}
          </ul>
        </section>
      ))}
    </div>
  );
}
