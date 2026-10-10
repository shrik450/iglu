// One ghostty-web terminal attached to one zmx session over a WebSocket.
// Closing the pane only detaches; the session keeps running in the guest.
// A dropped connection reconnects on its own, after telling the owner, which
// may find the session ended and dispose the pane instead.

import { signal } from "@preact/signals";
import { FitAddon, Ghostty, Terminal } from "ghostty-web";

import { withCtrl } from "./state/keys.ts";
import { scan } from "./state/replies.ts";

let ghostty: Promise<Ghostty> | null = null;

/** The console's monospace face; terminals draw with it once it's loaded,
 * since a canvas measures its cells with whatever font is there at the time. */
const FONT = "'JetBrains Mono', ui-monospace, Menlo, monospace";

/** ghostty's WebAssembly and the terminal font, loaded once for every pane. */
export function loadGhostty(): Promise<Ghostty> {
  ghostty ??= Promise.all([Ghostty.load("/ghostty-vt.wasm"), document.fonts.load(`13px ${FONT}`).catch(() => [])]).then(([loaded]) => loaded);
  return ghostty;
}

/** A terminal's colours, read from the page's look where it sits, so the
 * canvas matches its column in either look. */
function themeOf(where: Element) {
  const css = getComputedStyle(where);
  const color = (name: string, otherwise: string) => css.getPropertyValue(name).trim() || otherwise;
  return {
    background: color("--term-bg", "#070c18"),
    foreground: color("--term-ink", "#d4e0f4"),
    cursor: color("--accent", "#8fd8ff"),
    selectionBackground: "#22315a",
  };
}

/** Recolours every terminal after the look changes. */
export function restyleTerminals(): void {
  for (const pane of panes.values()) pane.restyle();
}

/** Ctrl held from the phone's key row: what's typed next goes with it. */
export const ctrlHeld = signal(false);

/** Mounted panes by `workspace/session`, so keyboard actions can focus one. */
export const panes = new Map<string, TerminalPane>();

/** The pane the person last focused, for `globalThis.iglu.screen()`. */
let lastFocused: TerminalPane | null = null;

export function activeScreen(): string {
  return lastFocused?.screen() ?? "";
}

export class TerminalPane {
  private readonly term: Terminal;
  private readonly fit: FitAddon;
  private readonly encoder = new TextEncoder();
  private socket: WebSocket | null = null;
  private focused = false;
  private closed = false;
  private retries = 0;
  private retryTimer = 0;
  /** The size this pane last told the session, so it isn't repeated. */
  private sent = "";
  /** The start of a query the next output may finish. */
  private carry: Uint8Array = new Uint8Array();
  private theme: ReturnType<typeof themeOf>;

  private readonly key: string;
  private readonly container: HTMLElement;
  private readonly workspace: string;
  private readonly session: string;
  private readonly onFocus: () => void;
  private readonly onStatus: (status: string) => void;
  private readonly onDrop: () => void;

  constructor(options: {
    container: HTMLElement;
    ghostty: Ghostty;
    workspace: string;
    session: string;
    /** Whether the console takes a key; it may write to the terminal instead. */
    shortcuts: (event: KeyboardEvent, send: (text: string) => void) => boolean;
    /** Asked once the terminal has opened: whether it should take focus. */
    wantsFocus: () => boolean;
    onFocus: () => void;
    onStatus: (status: string) => void;
    onDrop: () => void;
  }) {
    const { container, ghostty, workspace, session, shortcuts } = options;
    this.key = `${workspace}/${session}`;
    this.container = container;
    this.workspace = workspace;
    this.session = session;
    this.onFocus = options.onFocus;
    this.onStatus = options.onStatus;
    this.onDrop = options.onDrop;
    this.theme = themeOf(container);
    this.term = new Terminal({ ghostty, fontSize: 13, fontFamily: FONT, scrollback: 10000, theme: this.theme, cursorBlink: false, cursorStyle: "underline" });
    this.fit = new FitAddon();
    this.term.loadAddon(this.fit);
    // ghostty-web focuses a terminal as it opens, and again a moment later,
    // so the last column to open would take focus from the one meant to have
    // it. Its focus is switched off while it opens; then the pane focuses if
    // it's the one that should.
    this.term.focus = () => {};
    this.term.open(container);
    Reflect.deleteProperty(this.term, "focus");
    window.setTimeout(() => {
      if (options.wantsFocus()) this.focus();
    });
    this.fit.fit();
    this.fit.observeResize();
    // Unlike xterm.js, ghostty-web drops the key when the handler returns true.
    this.term.attachCustomKeyEventHandler((event) => shortcuts(event, (text) => this.send(this.encoder.encode(text))));
    this.term.onData((data) => {
      const typed = ctrlHeld.peek() ? withCtrl(data) : data;
      ctrlHeld.value = false;
      this.send(this.encoder.encode(typed));
    });
    // Every pane tells its session its own size when that changes, focused
    // or not. zmx applies the most recent size from any client, so a
    // browser in the background stays quiet, and focusing claims the size.
    this.term.onResize(({ cols, rows }) => {
      if (document.visibilityState === "visible") this.sendResize(cols, rows);
    });
    // A sideways swipe or Shift+wheel moves along the strip; the terminal
    // keeps vertical scrolling for its scrollback.
    this.term.attachCustomWheelEventHandler((event) => {
      const sideways = Math.abs(event.deltaX) > Math.abs(event.deltaY) || (event.shiftKey && event.deltaX === 0);
      if (!sideways) return false;
      container.closest(".w-cols")?.scrollBy({ left: event.deltaX || event.deltaY });
      return true;
    });
    container.addEventListener("focusin", this.focusIn);
    container.addEventListener("focusout", this.focusOut);
    panes.set(this.key, this);
    this.connect();
  }

  restyle(): void {
    this.theme = themeOf(this.container);
    this.term.options.theme = this.theme;
  }

  private readonly focusIn = () => {
    this.focused = true;
    lastFocused = this;
    this.term.options.cursorBlink = true;
    this.term.options.cursorStyle = "block";
    this.sendResize(this.term.cols, this.term.rows, true);
    this.onFocus();
  };

  // Only the terminal with the keyboard blinks a block; the others keep a
  // still underline, so it's plain where typing goes.
  private readonly focusOut = () => {
    this.focused = false;
    this.term.options.cursorBlink = false;
    this.term.options.cursorStyle = "underline";
  };

  private connect(): void {
    this.fit.fit();
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
        if (!this.closed) {
          this.term.reset();
          this.carry = new Uint8Array();
          this.connect();
        }
      }, delay);
    };
  }

  /** Shows output, answering the queries in it in the order they were asked,
   * interleaved with the ones ghostty answers itself as it reads. */
  private output(bytes: Uint8Array): void {
    const { replies, copies, carry } = scan(this.carry, bytes, this.theme);
    this.carry = carry;
    for (const copy of copies) void navigator.clipboard?.writeText(copy).catch(() => undefined);
    let shown = 0;
    for (const reply of replies) {
      this.term.write(bytes.subarray(shown, reply.end));
      this.send(this.encoder.encode(reply.text));
      shown = reply.end;
    }
    this.term.write(bytes.subarray(shown));
  }

  /** The visible screen as text, one line per row. The canvas has no text to read. */
  screen(): string {
    const buffer = this.term.buffer.active;
    const top = Math.max(0, buffer.length - this.term.rows);
    const lines: string[] = [];
    for (let y = top; y < buffer.length; y++) lines.push(buffer.getLine(y)?.translateToString(true) ?? "");
    return lines.join("\n");
  }

  /** Gives this terminal the keyboard, now. ghostty-web's own focus() does
   * it again a moment later, which would take the keyboard back from a field
   * opened in between, such as the column's name. */
  focus(): void {
    this.term.element?.focus();
  }

  /** Sends `text` as if typed, for keys a phone's keyboard doesn't have. */
  type(text: string): void {
    this.send(this.encoder.encode(text));
  }

  dispose(): void {
    this.closed = true;
    clearTimeout(this.retryTimer);
    this.socket?.close(1000);
    this.socket = null;
    panes.delete(this.key);
    if (lastFocused === this) lastFocused = null;
    this.container.removeEventListener("focusin", this.focusIn);
    this.container.removeEventListener("focusout", this.focusOut);
    this.fit.dispose();
    this.term.dispose();
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
