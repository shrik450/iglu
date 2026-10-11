// A browser column: the workspace's browser, with its address bar and tabs.

import { useEffect, useId, useRef, useState } from "preact/hooks";

import { focusColumn, markActive } from "../actions.ts";
import { BrowserPane, type Dialog, type Menu } from "../browser.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { mac, terminalKey } from "../keyboard.ts";
import { keyboard } from "../preferences.ts";
import { addressOf, stepOption, tabName, typedOption } from "../state/browser.ts";
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
  const menu = pane?.menu.value ?? null;
  const files = pane?.files.value ?? null;
  const downloaded = pane?.downloaded.value ?? null;
  return (
    <div class="browser">
      <Bar pane={pane} onUse={() => markActive(ws, name)} />
      {pane && dialog ? <Asking pane={pane} dialog={dialog} /> : null}
      {pane && menu ? <Choosing pane={pane} menu={menu} /> : null}
      {pane && files ? <Uploading pane={pane} files={files} /> : null}
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

/** How long typed letters add up to one search, as in a select. */
const TYPING_MS = 700;

/** A select the page opened, as options to choose from over where it is:
 * the browser's own menu never shows in its images. */
function Choosing({ pane, menu }: { pane: BrowserPane; menu: Menu }) {
  const id = useId();
  const list = useRef<HTMLUListElement>(null);
  const [active, setActive] = useState(menu.selected ?? stepOption(menu.options, -1, 1));
  const typed = useRef({ text: "", at: 0 });
  // Each select the page opens starts at its own choice.
  useEffect(() => {
    setActive(menu.selected ?? stepOption(menu.options, -1, 1));
    list.current?.focus({ preventScroll: true });
  }, [menu]);
  useEffect(() => list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" }), [active]);
  const place = pane.menuPlace(menu);
  if (!place) return null;
  // Below the select, unless there's more room above it.
  const below = window.innerHeight - (place.top + place.height);
  const style =
    below >= 200 || below >= place.top
      ? { left: place.left, minWidth: place.width, top: place.top + place.height, maxHeight: below - 8 }
      : { left: place.left, minWidth: place.width, bottom: window.innerHeight - place.top, maxHeight: place.top - 8 };
  const choose = (i: number) => {
    if (menu.options[i] && !menu.options[i].disabled) pane.choose(i);
  };
  const rows = menu.options.flatMap((option, i) => {
    const heading = option.group !== null && option.group !== menu.options[i - 1]?.group;
    const row = (
      <li
        key={i}
        id={`${id}-${i}`}
        data-index={i}
        role="option"
        aria-selected={i === active}
        aria-disabled={option.disabled || undefined}
        class={`${i === menu.selected ? "now" : ""}${option.group !== null ? " in" : ""}`}
        onMouseMove={() => !option.disabled && setActive(i)}
        onClick={() => choose(i)}
      >
        {option.label}
      </li>
    );
    return heading ? [<li key={`g${i}`} role="presentation" class="group">{option.group}</li>, row] : [row];
  });
  return (
    <ul
      ref={list}
      class="browser-menu"
      role="listbox"
      aria-label="Options"
      tabIndex={0}
      aria-activedescendant={`${id}-${active}`}
      style={style}
      onBlur={() => pane.choose(null)}
      onKeyDown={(e) => {
        const go = (i: number) => (e.preventDefault(), e.stopPropagation(), setActive(i));
        if (e.key === "ArrowDown") go(stepOption(menu.options, active, 1));
        else if (e.key === "ArrowUp") go(stepOption(menu.options, active, -1));
        else if (e.key === "Home") go(stepOption(menu.options, -1, 1));
        else if (e.key === "End") go(stepOption(menu.options, menu.options.length, -1));
        else if (e.key === "Enter" || (e.key === " " && !typed.current.text)) {
          e.preventDefault();
          e.stopPropagation();
          choose(active);
        } else if (e.key === "Escape" || e.key === "Tab") {
          e.preventDefault();
          e.stopPropagation();
          pane.choose(null);
        } else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          const now = Date.now();
          const text = now - typed.current.at < TYPING_MS ? typed.current.text + e.key : e.key;
          typed.current = { text, at: now };
          const found = typedOption(menu.options, text.length > 1 ? active - 1 : active, text);
          if (found !== null) go(found);
          else e.preventDefault();
        }
      }}
    >
      {rows}
    </ul>
  );
}

/** The page's file input, asking for files, whose own picker never shows
 * in the browser's images: chosen here, and sent to the workspace for it. */
function Uploading({ pane, files }: { pane: BrowserPane; files: { multiple: boolean; accept: string; sending: boolean } }) {
  const picker = useRef<HTMLInputElement>(null);
  const choose = useRef<HTMLButtonElement>(null);
  useEffect(() => choose.current?.focus(), []);
  return (
    <div
      class="browser-dialog"
      role="alertdialog"
      aria-label="The page asks for files"
      onKeyDown={(e) => {
        if (e.key !== "Escape" || files.sending) return;
        e.preventDefault();
        e.stopPropagation();
        void pane.giveFiles([]);
      }}
    >
      <p>{files.sending ? "Sending…" : files.multiple ? "The page asks for files." : "The page asks for a file."}</p>
      <input
        ref={picker}
        type="file"
        hidden
        multiple={files.multiple}
        accept={files.accept || undefined}
        onChange={(e) => void pane.giveFiles(Array.from(e.currentTarget.files ?? []))}
      />
      <span class="browser-dialog-do">
        <button type="button" class="btn" disabled={files.sending} onClick={() => void pane.giveFiles([])}>
          Cancel
        </button>
        <button type="button" class="btn primary" ref={choose} disabled={files.sending} onClick={() => picker.current?.click()}>
          {files.multiple ? "Choose files…" : "Choose a file…"}
        </button>
      </span>
    </div>
  );
}
