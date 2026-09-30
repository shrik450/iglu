// One ghostty-web terminal attached to one zmx session over a WebSocket.
// Closing the pane only detaches; the session keeps running in the guest.

import { FitAddon, type Ghostty, Terminal } from "ghostty-web";

const theme = {
  background: "#11161a",
  foreground: "#dde5e1",
  cursor: "#89d8bd",
  selectionBackground: "#285347",
};

export class TerminalPane {
  private readonly term: Terminal;
  private readonly fit: FitAddon;
  private socket: WebSocket | null = null;
  private focused = false;
  private readonly encoder = new TextEncoder();

  constructor(
    private readonly container: HTMLElement,
    ghostty: Ghostty,
    shortcuts: (event: KeyboardEvent) => boolean,
    private readonly onStatus: (status: string) => void,
  ) {
    this.term = new Terminal({ ghostty, fontSize: 14, scrollback: 10000, theme, cursorBlink: true });
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
    container.addEventListener("focusin", () => {
      this.focused = true;
      this.sendResize(this.term.cols, this.term.rows);
    });
    container.addEventListener("focusout", () => {
      this.focused = false;
    });
  }

  attach(workspace: string, session: string): void {
    this.detach();
    this.fit.fit();
    const url = new URL(`/v1/workspaces/${workspace}/terminals/${session}/attach`, location.href);
    url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
    url.searchParams.set("cols", String(this.term.cols));
    url.searchParams.set("rows", String(this.term.rows));
    const socket = new WebSocket(url);
    socket.binaryType = "arraybuffer";
    this.socket = socket;
    this.onStatus("connecting…");
    socket.onopen = () => {
      this.onStatus("");
      this.term.focus();
    };
    socket.onmessage = (event: MessageEvent<ArrayBuffer | string>) => {
      if (event.data instanceof ArrayBuffer) this.term.write(new Uint8Array(event.data));
    };
    socket.onclose = (event) => {
      if (this.socket === socket) {
        this.socket = null;
        this.onStatus(event.code === 1000 ? "detached" : "disconnected; select the tab to reconnect");
      }
    };
  }

  detach(): void {
    const socket = this.socket;
    this.socket = null;
    socket?.close(1000);
    this.term.reset();
  }

  focus(): void {
    this.term.focus();
  }

  dispose(): void {
    this.detach();
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
