// The keyboard shortcuts, on ?.

import { mac } from "../keyboard.ts";
import { onKeyboard, SHEET } from "../state/keys.ts";
import { overlay } from "../state/store.ts";

export function Keys() {
  const close = () => (overlay.value = null);
  return (
    <div class="overlay" onClick={(e) => e.target === e.currentTarget && close()}>
      <div class="kbox" role="dialog" aria-labelledby="keys-h" onKeyDown={(e) => (e.key === "Escape" || e.key === "?") && close()}>
        <div class="khead">
          <h2 id="keys-h">Keyboard shortcuts</h2>
          <button type="button" class="btn icon" aria-label="Close" autoFocus onClick={close}>
            ×
          </button>
        </div>
        <div class="kgroups">
          {SHEET.map(({ group, shortcuts }) => (
            <section key={group}>
              <h3>{group}</h3>
              <dl>
                {shortcuts.map((s) => (
                  <div key={s.keys}>
                    <dt>
                      {onKeyboard(s.keys, mac).split(" ").map((k) => (
                        <kbd key={k}>{k}</kbd>
                      ))}
                    </dt>
                    <dd>{s.does}</dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
        <p class="muted">Letters work when no terminal or field has the cursor. Hold {onKeyboard("⌥", mac).replace("+", "")} to use them from a terminal.</p>
      </div>
    </div>
  );
}
