// A browser column: the workspace's browser, with its address bar and tabs.

import { useEffect, useRef, useState } from "preact/hooks";

import { focusColumn, markActive } from "../actions.ts";
import { BrowserPane, type Dialog } from "../browser.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { mac, terminalKey } from "../keyboard.ts";
import { keyboard } from "../preferences.ts";
import { addressOf, tabName } from "../state/browser.ts";
import { chordLabel } from "../state/keys.ts";
import { Icon } from "./bits.tsx";

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
      shortcuts: terminalKey,
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
  const dialog = pane?.dialog.value ?? null;
  return (
    <div class="browser">
      <Bar pane={pane} onUse={() => markActive(ws, name)} />
      {pane && dialog ? <Asking pane={pane} dialog={dialog} /> : null}
      <div class="browser-stage">
        <div class="browser-screen" ref={host} onMouseDown={() => focusColumn(ws, name)} />
        {waiting ? <p class="browser-wait">{waiting}</p> : null}
      </div>
    </div>
  );
}

/** The address bar and tabs; there before the pane is, so the screen keeps its size. */
function Bar({ pane, onUse }: { pane: BrowserPane | null; onUse: () => void }) {
  const tabs = pane?.tabs.value ?? [];
  const shown = tabs.find((t) => t.id === pane?.shown.value);
  // What's typed into the address bar, until it's gone to or put back.
  const [draft, setDraft] = useState<string | null>(null);
  // Kept while the page moves on, as a browser keeps what's typed in its
  // address bar; another tab has its own address.
  useEffect(() => setDraft(null), [shown?.id]);
  // Nothing to act on until the browser has shown a tab.
  const ready = pane !== null && shown !== undefined;
  const loading = pane?.loading.value ?? false;
  return (
    <>
      <div class="browser-bar" onFocusCapture={onUse}>
        <button type="button" aria-label="Back" title="Back" disabled={!ready} onClick={() => pane?.act({ type: "back" })}>
          <Icon name="back" size={12} />
        </button>
        <button type="button" aria-label="Forward" title="Forward" disabled={!ready} onClick={() => pane?.act({ type: "forward" })}>
          <Icon name="chevron" size={12} />
        </button>
        <button type="button" class={loading ? "loading" : undefined} aria-label="Reload" title={loading ? "Loading…" : "Reload"} disabled={!ready} onClick={() => pane?.act({ type: "reload" })}>
          <Icon name="reload" size={12} />
        </button>
        <form
          class="browser-go"
          onSubmit={(e) => {
            e.preventDefault();
            const address = (draft ?? "").trim();
            if (address) pane?.navigate(address);
            setDraft(null);
          }}
        >
          <input
            class="browser-address"
            aria-label="Address"
            placeholder="Go to an address, like localhost:3000"
            value={draft ?? addressOf(shown)}
            disabled={!ready}
            spellcheck={false}
            autocapitalize="off"
            autocomplete="off"
            translate={false}
            onFocus={(e) => e.currentTarget.select()}
            onInput={(e) => setDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key !== "Escape") return;
              e.preventDefault();
              e.stopPropagation();
              setDraft(null);
              pane?.focus();
            }}
          />
        </form>
        <button type="button" aria-label="New tab" title="New tab" disabled={!ready} onClick={() => pane?.act({ type: "open" })}>
          <Icon name="plus" size={12} />
        </button>
      </div>
      {tabs.length > 1 ? (
        <div class="browser-tabs" role="tablist" aria-label="Tabs">
          {tabs.map((tab) => (
            <div key={tab.id} class={`browser-tab${tab.id === shown?.id ? " on" : ""}`}>
              <button type="button" role="tab" aria-selected={tab.id === shown?.id} title={tab.url} translate={false} onClick={() => (pane?.act({ type: "show", tab: tab.id }), pane?.focus())}>
                {tabName(tab)}
              </button>
              <button type="button" class="browser-tab-x" aria-label={`Close ${tabName(tab)}`} onClick={() => pane?.act({ type: "close", tab: tab.id })}>
                <Icon name="close" size={9} />
              </button>
            </div>
          ))}
        </div>
      ) : null}
    </>
  );
}

/** The page's alert, confirm or prompt, which holds it until it's answered. */
function Asking({ pane, dialog }: { pane: BrowserPane; dialog: Dialog }) {
  const [text, setText] = useState(dialog.default);
  const field = useRef<HTMLInputElement>(null);
  const ok = useRef<HTMLButtonElement>(null);
  useEffect(() => (dialog.kind === "prompt" ? field.current?.select() : ok.current?.focus()), [dialog]);
  const question = dialog.kind === "beforeunload" ? "Leave this page? What's changed on it may not be saved." : dialog.message;
  return (
    <div
      class="browser-dialog"
      role="alertdialog"
      aria-label="The page asks"
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        e.preventDefault();
        e.stopPropagation();
        pane.answer(false);
      }}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          pane.answer(true, dialog.kind === "prompt" ? text : undefined);
        }}
      >
      <p>{question}</p>
      {dialog.kind === "prompt" ? <input ref={field} value={text} aria-label="Answer" onInput={(e) => setText(e.currentTarget.value)} /> : null}
      <span class="browser-dialog-do">
        {dialog.kind === "alert" ? null : (
          <button type="button" class="btn" onClick={() => pane.answer(false)}>
            {dialog.kind === "beforeunload" ? "Stay" : "Cancel"}
          </button>
        )}
        <button type="submit" class="btn primary" ref={ok}>
          {dialog.kind === "beforeunload" ? "Leave" : "OK"}
        </button>
      </span>
      </form>
    </div>
  );
}
