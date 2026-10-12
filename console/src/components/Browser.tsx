// A browser column: the workspace's browser, which draws its own tabs,
// address bar, menus and dialogs, and what it downloaded.

import { useEffect, useRef, useState } from "preact/hooks";

import { focusColumn, markActive } from "../actions.ts";
import { BrowserPane } from "../browser.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { browserKey, mac } from "../keyboard.ts";
import { keyboard } from "../preferences.ts";
import { chordLabel } from "../state/keys.ts";

export function Browser(props: { ws: WorkspaceView; name: string; title: string; on: boolean }) {
  const { ws, name } = props;
  const host = useRef<HTMLDivElement>(null);
  const [pane, setPane] = useState<BrowserPane | null>(null);
  // The column may stop being the active one while the pane opens.
  const on = useRef(props.on);
  on.current = props.on;
  const prefix = chordLabel(keyboard.value.prefix, mac);
  useEffect(() => pane?.describe(props.title, prefix), [pane, props.title, prefix]);
  useEffect(() => {
    if (!host.current) return;
    const opened = new BrowserPane({
      container: host.current,
      workspace: ws.id,
      session: name,
      mac,
      shortcuts: browserKey,
      wantsFocus: () => on.current,
      onFocus: () => markActive(ws, name),
    });
    setPane(opened);
    return () => {
      setPane(null);
      opened.dispose();
    };
  }, [ws.id, name]);
  const waiting = pane ? pane.waiting.value : "Opening the browser…";
  const downloaded = pane?.downloaded.value ?? null;
  return (
    <div class="browser">
      {pane && downloaded ? (
        <div class="browser-dialog" role="status" aria-label="Downloaded">
          <p translate={false}>{downloaded.name === null ? `Downloaded to ${downloaded.path}.` : `Downloaded ${downloaded.name}.`}</p>
          <span class="browser-dialog-do">
            <button type="button" class="btn" onClick={() => (pane.downloaded.value = null)}>
              Dismiss
            </button>
            {downloaded.url ? (
              <a class="btn primary" href={downloaded.url} download={downloaded.name ?? true} onClick={() => (pane.downloaded.value = null)}>
                Save to this computer
              </a>
            ) : null}
          </span>
        </div>
      ) : null}
      <div class="browser-stage">
        <div class="browser-screen" ref={host} onPointerDown={() => focusColumn(ws, name)} />
        {waiting ? <p class="browser-wait">{waiting}</p> : null}
      </div>
    </div>
  );
}
