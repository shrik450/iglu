// One wterm terminal attached to one zmx session over a WebSocket.
// Closing the pane only detaches; the session keeps running in the guest.
// A dropped connection reconnects on its own, after telling the owner, which
// may find the session ended and dispose the pane instead.

import { signal } from "@preact/signals";
import type { TerminalThemeColors } from "@wterm/core";
import { type SearchState, WTerm } from "@wterm/dom";
import { GhosttyCore } from "@wterm/ghostty";

import { api, failure } from "./api/client.ts";
import { withCtrl } from "./state/keys.ts";
import { MAX_FILE_BYTES, pastedPath } from "./state/paste.ts";
import { programTitle } from "./state/program.ts";
import { scan } from "./state/replies.ts";
import { say } from "./state/store.ts";
import { type Colors, XTERM } from "./state/themes.ts";
import { styleApplied } from "./termstyle.ts";

const WASM = "/ghostty-vt.wasm";
/** Each terminal's history, in bytes: ghostty keeps it in pages, so the rows
 * it holds depend on the width. */
const SCROLLBACK = 10_000_000;

let wasm: Promise<void> | null = null;

/** The terminal's style and ghostty's WebAssembly, loaded before any pane
 * opens: a terminal measures its cells with whatever font is there then. */
export function loadTerminals(): Promise<unknown> {
  wasm ??= GhosttyCore.load({ wasmPath: WASM }).then((core) => core.dispose());
  return Promise.all([styleApplied(), wasm]);
}

interface Theme {
  background: string;
  foreground: string;
  cursor: string;
  palette: readonly string[];
}

/** A terminal's colours, read from the page where it sits: the look's, so it
 * matches its column in either, or the theme's chosen in Settings. */
function themeOf(where: Element): Theme {
  const css = getComputedStyle(where);
  const color = (name: string, otherwise: string) => {
    const value = css.getPropertyValue(name).trim();
    return /^#[0-9a-f]{6}$/i.test(value) ? value : otherwise;
  };
  return {
    background: color("--term-bg", "#070c18"),
    foreground: color("--term-ink", "#d4e0f4"),
    cursor: color("--term-caret", color("--accent", "#8fd8ff")),
    palette: XTERM.map((otherwise, index) => color(`--term-ansi-${index}`, otherwise)),
  };
}

function colorsOf(theme: Theme): TerminalThemeColors {
  const rgb = (hex: string) => parseInt(hex.slice(1), 16);
  return { foreground: rgb(theme.foreground), background: rgb(theme.background), cursor: rgb(theme.cursor), palette: theme.palette.map(rgb) };
}

/** The colours terminals draw with now, as a theme. */
export function pageColors(): Colors {
  const theme = themeOf(document.documentElement);
  return { ...theme, selection: null, selectionText: null };
}

/** Recolours every terminal after the look or the theme changes. */
export function restyleTerminals(): void {
  for (const pane of panes.values()) if (pane instanceof TerminalPane) pane.restyle();
}

/** Ctrl held from the phone's key row: what's typed next goes with it. */
export const ctrlHeld = signal(false);

/** What the console does with a column's pane: a terminal, or the browser. */
export interface Pane {
  focus(): void;
  /** Sends `text` as if typed. */
  type(text: string): void;
  /** Presses `key`, as a KeyboardEvent names it, for keys a phone's keyboard doesn't have. */
  press(key: string): void;
}

/** Mounted panes by `workspace/session`, so keyboard actions can focus one. */
export const panes = new Map<string, Pane>();

/** What each mounted pane's program calls itself (OSC 0 or 2), by
 * `workspace/session`. zmx sends it again to each new attach. */
export const titles = signal<ReadonlyMap<string, string>>(new Map());

export function setTitle(key: string, title: string): void {
  if ((titles.peek().get(key) ?? "") === title) return;
  const next = new Map(titles.peek());
  if (title) next.set(key, title);
  else next.delete(key);
  titles.value = next;
}

/** The pane the person last focused, for `globalThis.iglu.screen()`. */
let lastFocused: TerminalPane | null = null;

export function activeScreen(): string {
  return lastFocused?.screen() ?? "";
}

export class TerminalPane implements Pane {
  /** What finding in this terminal's output has found, while a search is on. */
  readonly found = signal<SearchState | null>(null);
  private readonly term: WTerm;
  private readonly core: GhosttyCore;
  private readonly encoder = new TextEncoder();
  /** Pauses drawing while the column is out of sight; output still lands. */
  private readonly sight: IntersectionObserver;
  private socket: WebSocket | null = null;
  private opened = false;
  private focused = false;
  private closed = false;
  private retries = 0;
  private retryTimer = 0;
  /** The size this pane last told the session, so it isn't repeated. */
  private sent = "";
  /** The start of a query the next output may finish. */
  private carry: Uint8Array = new Uint8Array();
  private theme: Theme;

  private readonly key: string;
  private readonly container: HTMLElement;
  private readonly element: HTMLElement;
  private readonly workspace: string;
  private readonly session: string;
  private readonly shortcuts: (event: KeyboardEvent, send: (text: string) => void) => boolean;
  private readonly onFocus: () => void;
  private readonly onStatus: (status: string) => void;
  private readonly onDrop: () => void;

  constructor(options: {
    container: HTMLElement;
    /** A core of its own, from `GhosttyCore.load`; the pane disposes it. */
    core: GhosttyCore;
    workspace: string;
    session: string;
    /** Whether the console takes a key; it may write to the terminal instead. */
    shortcuts: (event: KeyboardEvent, send: (text: string) => void) => boolean;
    /** Asked once the terminal has opened: whether it should take focus,
     * which it does only if nothing else has it. */
    wantsFocus: () => boolean;
    onFocus: () => void;
    onStatus: (status: string) => void;
    onDrop: () => void;
  }) {
    const { container, core, workspace, session } = options;
    this.key = `${workspace}/${session}`;
    this.container = container;
    this.core = core;
    this.workspace = workspace;
    this.session = session;
    this.shortcuts = options.shortcuts;
    this.onFocus = options.onFocus;
    this.onStatus = options.onStatus;
    this.onDrop = options.onDrop;
    this.theme = themeOf(container);
    // wterm takes over the element it's given, colours included, so the
    // look is read from the host around it.
    const element = document.createElement("div");
    container.append(element);
    this.element = element;
    this.term = new WTerm(element, {
      core,
      // The console has its own way out, the prefix; Claude Code takes Escape then Tab.
      tabExit: false,
      onData: (data) => {
        const typed = ctrlHeld.peek() ? withCtrl(data) : data;
        ctrlHeld.value = false;
        this.send(this.encoder.encode(typed));
      },
      // Mouse reports in the older encodings aren't text.
      onBinary: (data) => this.send(data),
      onClipboardWrite: (text) => void navigator.clipboard?.writeText(text).catch(() => undefined),
      onPasteFiles: (files) => void this.pasteFiles(files),
      onTitle: (title) => setTitle(this.key, programTitle(title)),
      // Ghostty's own answer: a VT220 with colour, which can set the
      // clipboard (OSC 52), so programs like nvim copy through it.
      primaryAttributes: "\x1b[?62;22;52c",
      onSearchChange: (state) => (this.found.value = state.query ? state : null),
      // Every pane tells its session its own size when that changes, focused
      // or not. zmx applies the most recent size from any client, so a
      // browser in the background stays quiet, and focusing claims the size.
      onResize: (cols, rows) => {
        if (document.visibilityState === "visible") this.sendResize(cols, rows);
      },
    });
    this.term.setThemeColors(colorsOf(this.theme));
    container.addEventListener("keydown", this.keyDown, { capture: true });
    container.addEventListener("wheel", this.wheel, { capture: true, passive: false });
    container.addEventListener("focusin", this.focusIn);
    container.addEventListener("focusout", this.focusOut);
    this.sight = new IntersectionObserver(([entry]) => this.term.setRenderingPaused(!entry?.isIntersecting));
    this.sight.observe(container);
    panes.set(this.key, this);
    void this.term.init().then(() => {
      if (this.closed) return;
      this.opened = true;
      // Opening takes a moment: the keyboard is only taken if nothing else
      // has it since, such as a column's name being edited.
      const active = document.activeElement;
      const free = !active || active === document.body || container.contains(active);
      if (options.wantsFocus() && free) this.focus();
      this.connect();
    });
  }

  /** Names the terminal for assistive tech, with how to leave it: wterm
   * gives its input the element's name and description. */
  describe(title: string, prefix: string): void {
    this.element.setAttribute("aria-label", `Terminal, ${title}`);
    this.element.setAttribute("aria-description", `${prefix}, then Tab leaves the terminal; ${prefix}, then ? lists shortcuts.`);
  }

  restyle(): void {
    this.theme = themeOf(this.container);
    this.term.setThemeColors(colorsOf(this.theme));
  }

  /** Keeps each file in the workspace and pastes its path, as a terminal
   * pastes a dropped file's path: agents that take images read them from
   * there. A paste each, for programs that take one path a paste. */
  private async pasteFiles(files: File[]): Promise<void> {
    const large = files.find((file) => file.size > MAX_FILE_BYTES);
    if (large) {
      say(`${large.name || "That file"} is larger than 16 MB, so it can't be pasted.`);
      return;
    }
    const total = files.reduce((sum, file) => sum + file.size, 0);
    if (total > 1024 * 1024) say(files.length === 1 ? `Sending ${files[0]!.name}…` : `Sending ${files.length} files…`);
    try {
      for (const [index, file] of files.entries()) {
        const { path } = await api.keepFile(this.workspace, file);
        if (this.closed) return;
        if (index > 0) this.term.type(" ");
        this.term.paste(pastedPath(path));
      }
    } catch (error) {
      say(`That couldn't be pasted: ${failure(error)}`);
    }
  }

  /** The console's keys come first; a key it takes never reaches the terminal. */
  private readonly keyDown = (event: KeyboardEvent) => {
    if (this.shortcuts(event, (text) => this.term.type(text))) event.stopPropagation();
  };

  /** A sideways swipe or Shift+wheel moves along the strip; the terminal
   * keeps vertical scrolling for its scrollback. */
  private readonly wheel = (event: WheelEvent) => {
    const sideways = Math.abs(event.deltaX) > Math.abs(event.deltaY) || (event.shiftKey && event.deltaX === 0);
    if (!sideways) return;
    event.preventDefault();
    event.stopPropagation();
    this.container.closest(".w-cols")?.scrollBy({ left: event.deltaX || event.deltaY });
  };

  private readonly focusIn = () => {
    this.focused = true;
    lastFocused = this;
    this.sendResize(this.term.cols, this.term.rows, true);
    this.onFocus();
  };

  private readonly focusOut = () => {
    this.focused = false;
  };

  private connect(): void {
    const url = new URL(`/v1/workspaces/${this.workspace}/columns/${this.session}/attach`, location.href);
    url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
    url.searchParams.set("cols", String(this.term.cols));
    url.searchParams.set("rows", String(this.term.rows));
    const socket = new WebSocket(url);
    socket.binaryType = "arraybuffer";
    this.socket = socket;
    socket.onopen = () => {
      this.retries = 0;
      this.onStatus("");
      this.sent = "";
      if (this.focused) this.sendResize(this.term.cols, this.term.rows, true);
    };
    socket.onmessage = (event: MessageEvent<ArrayBuffer | string>) => {
      if (event.data instanceof ArrayBuffer) this.output(new Uint8Array(event.data));
    };
    socket.onclose = (event) => {
      if (this.socket !== socket || this.closed) return;
      this.socket = null;
      this.onDrop();
      const delay = Math.min(10_000, 500 * 2 ** this.retries);
      this.retries += 1;
      // iglud gives a reason when it ends a terminal on purpose. One it
      // refused before connecting reaches onDrop's reload of the columns.
      this.onStatus(event.reason ? `Reconnecting… (${event.reason})` : "Reconnecting…");
      this.retryTimer = window.setTimeout(() => {
        if (this.closed) return;
        // The session replays its screen on attach: start from a clean one.
        this.term.write("\x1bc");
        this.carry = new Uint8Array();
        this.connect();
      }, delay);
    };
  }

  /** Shows output, answering the queries wterm leaves in the order they were
   * asked, interleaved with the ones it answers itself as it reads. */
  private output(bytes: Uint8Array): void {
    const { replies, carry } = scan(this.carry, bytes, this.theme.cursor);
    this.carry = carry;
    let shown = 0;
    for (const reply of replies) {
      if (reply.end > shown) this.term.write(bytes.subarray(shown, reply.end));
      this.send(this.encoder.encode(reply.text));
      shown = reply.end;
    }
    if (shown < bytes.length) this.term.write(bytes.subarray(shown));
  }

  /** The visible screen as text, one line per row. */
  screen(): string {
    const lines: string[] = [];
    for (let row = 0; row < this.term.rows; row++) {
      let line = "";
      for (let col = 0; col < this.term.cols; col++) {
        const cell = this.core.getCell(row, col);
        if (cell.width === 0) continue;
        line += cell.chars ?? String.fromCodePoint(cell.char || 0x20);
      }
      lines.push(line.trimEnd());
    }
    return lines.join("\n");
  }

  /** Gives this terminal the keyboard, now. Before it has opened there's
   * nothing to focus; it asks `wantsFocus` once it has. */
  focus(): void {
    if (this.opened) this.term.focus();
  }

  /** Finds `query` in the output, history included, starting from the newest
   * match; an empty query ends the search. */
  find(query: string): void {
    this.term.search(query, { newestFirst: true });
  }

  /** Moves to the next match back in the output, or forward. */
  findAgain(back: boolean): void {
    if (back) this.term.findPrevious();
    else this.term.findNext();
  }

  /** Sends `text` as if typed. */
  type(text: string): void {
    this.term.type(text);
  }

  /** Presses `key`, as a KeyboardEvent names it, for keys a phone's keyboard
   * doesn't have: encoded as the program's modes want it, and with Ctrl when
   * it's held from the key row. */
  press(key: string): void {
    const ctrlKey = ctrlHeld.peek();
    ctrlHeld.value = false;
    this.term.press(new KeyboardEvent("keydown", { key, ctrlKey }));
  }

  dispose(): void {
    this.closed = true;
    clearTimeout(this.retryTimer);
    this.socket?.close(1000);
    this.socket = null;
    panes.delete(this.key);
    setTitle(this.key, "");
    if (lastFocused === this) lastFocused = null;
    this.sight.disconnect();
    this.container.removeEventListener("keydown", this.keyDown, { capture: true });
    this.container.removeEventListener("wheel", this.wheel, { capture: true });
    this.container.removeEventListener("focusin", this.focusIn);
    this.container.removeEventListener("focusout", this.focusOut);
    this.term.destroy();
    this.core.dispose();
    this.container.replaceChildren();
  }

  private send(data: Uint8Array): void {
    if (this.socket?.readyState === WebSocket.OPEN) this.socket.send(data);
  }

  /** Tells the session this pane's size, unless it already knows; `claim`
   * says it again, since another client may have set its own since. */
  private sendResize(cols: number, rows: number, claim = false): void {
    const size = `${cols}x${rows}`;
    if (this.socket?.readyState !== WebSocket.OPEN || (size === this.sent && !claim)) return;
    this.sent = size;
    this.socket.send(JSON.stringify({ type: "resize", cols, rows }));
  }
}

/** A core for one pane, ghostty's terminal state, once the font and the
 * WebAssembly have loaded. */
export async function newCore(): Promise<GhosttyCore> {
  await loadTerminals();
  return GhosttyCore.load({ wasmPath: WASM, scrollbackLimit: SCROLLBACK });
}
