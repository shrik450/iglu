// One ghostty-web terminal attached to one zmx session over a WebSocket.
// Closing the pane only detaches; the session keeps running in the guest.
// A dropped connection reconnects on its own, after telling the owner, which
// may find the session ended and dispose the pane instead.

import { FitAddon, Ghostty, Terminal } from "ghostty-web";

let ghostty: Promise<Ghostty> | null = null;

/** ghostty's WebAssembly, loaded once for every pane. */
export function loadGhostty(): Promise<Ghostty> {
  ghostty ??= Ghostty.load("/ghostty-vt.wasm");
  return ghostty;
}

const theme = {
  background: "#070c18",
  foreground: "#d4e0f4",
  cursor: "#8fd8ff",
  selectionBackground: "#22315a",
};

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
    shortcuts: (event: KeyboardEvent) => boolean;
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
    this.term = new Terminal({ ghostty, fontSize: 13, scrollback: 10000, theme, cursorBlink: true });
    this.fit = new FitAddon();
    this.term.loadAddon(this.fit);
    this.term.open(container);
    this.fit.fit();
    this.fit.observeResize();
    // Unlike xterm.js, ghostty-web drops the key when the handler returns true.
    this.term.attachCustomKeyEventHandler(shortcuts);
    this.term.onData((data) => this.send(this.encoder.encode(data)));
    // zmx applies the most recent resize from any client, so only the
    // focused client sends one.
    this.term.onResize(({ cols, rows }) => {
      if (this.focused) this.sendResize(cols, rows);
    });
    container.addEventListener("focusin", this.focusIn);
    container.addEventListener("focusout", this.focusOut);
    panes.set(this.key, this);
    this.connect();
  }

  private readonly focusIn = () => {
    this.focused = true;
    lastFocused = this;
    this.sendResize(this.term.cols, this.term.rows);
    this.onFocus();
  };

  private readonly focusOut = () => {
    this.focused = false;
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
      if (this.focused) this.sendResize(this.term.cols, this.term.rows);
    };
    socket.onmessage = (event: MessageEvent<ArrayBuffer | string>) => {
      if (event.data instanceof ArrayBuffer) this.term.write(new Uint8Array(event.data));
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
          this.connect();
        }
      }, delay);
    };
  }

  /** The visible screen as text, one line per row. The canvas has no text to read. */
  screen(): string {
    const buffer = this.term.buffer.active;
    const top = Math.max(0, buffer.length - this.term.rows);
    const lines: string[] = [];
    for (let y = top; y < buffer.length; y++) lines.push(buffer.getLine(y)?.translateToString(true) ?? "");
    return lines.join("\n");
  }

  focus(): void {
    this.term.focus();
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

  private sendResize(cols: number, rows: number): void {
    if (this.socket?.readyState === WebSocket.OPEN) {
      this.socket.send(JSON.stringify({ type: "resize", cols, rows }));
    }
  }
}
