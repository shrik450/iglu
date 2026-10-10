// The workspace's browser in a column: iglud screencasts the shown tab as
// JPEG images over a WebSocket, and takes what the person does as
// BrowserInput. Closing the pane only stops showing it; the browser keeps
// running in the workspace, for agents too. A dropped connection reconnects
// on its own, as a terminal's does.

import { signal } from "@preact/signals";

import type { BrowserEvent } from "./generated/BrowserEvent.ts";
import type { BrowserInput } from "./generated/BrowserInput.ts";
import type { BrowserTab } from "./generated/BrowserTab.ts";
import type { DialogKind } from "./generated/DialogKind.ts";
import type { MouseAction } from "./generated/MouseAction.ts";
import { buttonOf, clicksAfter, clipboardAction, keyOf, modifiersOf, pagePoint, type Press, tabName, wheelPixels } from "./state/browser.ts";
import { say } from "./state/store.ts";
import { panes, setTitle } from "./terminal.ts";

export interface Dialog {
  kind: DialogKind;
  message: string;
  default: string;
}

export class BrowserPane {
  readonly tabs = signal<BrowserTab[]>([]);
  readonly shown = signal<string | null>(null);
  readonly loading = signal(false);
  readonly dialog = signal<Dialog | null>(null);
  /** Why there's nothing to see yet, while there isn't. */
  readonly waiting = signal<string | null>("Opening the browser…");

  private readonly key: string;
  private readonly workspace: string;
  private readonly screen: HTMLElement;
  private readonly canvas: HTMLCanvasElement;
  /** Holds the keyboard, so an input method composes into it and a paste lands in it. */
  private readonly input: HTMLTextAreaElement;
  private readonly sizes: ResizeObserver;
  private readonly mac: boolean;
  private readonly shortcuts: (event: KeyboardEvent, send: (text: string) => void) => boolean;
  private readonly onFocus: () => void;
  private socket: WebSocket | null = null;
  private closed = false;
  private retries = 0;
  private retryTimer = 0;
  private resizeTimer = 0;
  /** The page size the images show, in CSS pixels. */
  private page = { width: 0, height: 0 };
  /** Images decode in turn; one that arrives while another decodes replaces any waiting. */
  private decoding = false;
  private next: Blob | null = null;
  /** Keys pressed on the page and not yet let go. */
  private readonly held = new Set<string>();
  private lastPress: Press | null = null;
  private buttons = 0;
  /** A pointer move waiting for the next frame, so moves go at most once a frame. */
  private move: PointerEvent | null = null;
  private moveFrame = 0;
  /** Where a finger went down, while it may yet scroll. */
  private touch: { id: number; x: number; y: number; scrolling: boolean } | null = null;

  constructor(options: {
    container: HTMLElement;
    workspace: string;
    session: string;
    mac: boolean;
    /** Whether the console takes a key; it may type into the page instead. */
    shortcuts: (event: KeyboardEvent, send: (text: string) => void) => boolean;
    wantsFocus: () => boolean;
    onFocus: () => void;
  }) {
    const { container, workspace, session } = options;
    this.key = `${workspace}/${session}`;
    this.workspace = workspace;
    this.screen = container;
    this.mac = options.mac;
    this.shortcuts = options.shortcuts;
    this.onFocus = options.onFocus;
    this.canvas = document.createElement("canvas");
    this.input = document.createElement("textarea");
    this.input.className = "browser-input";
    this.input.setAttribute("autocapitalize", "off");
    this.input.setAttribute("autocomplete", "off");
    this.input.spellcheck = false;
    container.append(this.canvas, this.input);

    this.input.addEventListener("keydown", this.keyDown);
    this.input.addEventListener("keyup", this.keyUp);
    this.input.addEventListener("compositionend", this.composed);
    this.input.addEventListener("input", this.typed);
    this.input.addEventListener("paste", this.pasted);
    this.input.addEventListener("focus", this.onFocus);
    this.canvas.addEventListener("pointerdown", this.pointerDown);
    this.canvas.addEventListener("pointermove", this.pointerMove);
    this.canvas.addEventListener("pointerup", this.pointerUp);
    this.canvas.addEventListener("pointercancel", this.pointerUp);
    this.canvas.addEventListener("contextmenu", (e) => e.preventDefault());
    container.addEventListener("wheel", this.wheel, { passive: false });
    this.sizes = new ResizeObserver(() => {
      clearTimeout(this.resizeTimer);
      this.resizeTimer = window.setTimeout(() => this.sendResize(), 100);
    });
    this.sizes.observe(container);
    panes.set(this.key, this);
    const active = document.activeElement;
    if (options.wantsFocus() && (!active || active === document.body)) this.focus();
    this.connect();
  }

  describe(title: string, prefix: string): void {
    this.input.setAttribute("aria-label", `Browser, ${title}`);
    this.input.setAttribute("aria-description", `${prefix}, then Tab leaves the browser; ${prefix}, then ? lists shortcuts.`);
  }

  focus(): void {
    this.input.focus({ preventScroll: true });
  }

  /** Types `text` into the page. */
  type(text: string): void {
    this.send({ type: "text", text });
  }

  /** Presses and releases `key`, as a KeyboardEvent names it. */
  press(key: string): void {
    const modifiers = { alt: false, ctrl: false, meta: false, shift: false };
    const code = key.length === 1 ? "" : key;
    this.send({ type: "key", action: "press", key, code, repeat: false, location: 0, modifiers });
    this.send({ type: "key", action: "release", key, code, repeat: false, location: 0, modifiers });
  }

  navigate(address: string): void {
    this.send({ type: "navigate", address });
    this.focus();
  }

  act(input: BrowserInput): void {
    this.send(input);
  }

  answer(accept: boolean, text?: string): void {
    this.dialog.value = null;
    this.send(text === undefined ? { type: "answer", accept } : { type: "answer", accept, text });
    this.focus();
  }

  dispose(): void {
    this.closed = true;
    clearTimeout(this.retryTimer);
    clearTimeout(this.resizeTimer);
    cancelAnimationFrame(this.moveFrame);
    this.socket?.close(1000);
    this.socket = null;
    this.sizes.disconnect();
    panes.delete(this.key);
    setTitle(this.key, "");
    this.screen.removeEventListener("wheel", this.wheel);
    this.screen.replaceChildren();
  }

  private size(): { width: number; height: number; scale: number } {
    const box = this.screen.getBoundingClientRect();
    return { width: Math.max(1, Math.floor(box.width)), height: Math.max(1, Math.floor(box.height)), scale: window.devicePixelRatio || 1 };
  }

  private connect(): void {
    const url = new URL(`/v1/workspaces/${this.workspace}/browser`, location.href);
    url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
    const size = this.size();
    url.searchParams.set("width", String(size.width));
    url.searchParams.set("height", String(size.height));
    url.searchParams.set("scale", String(size.scale));
    const socket = new WebSocket(url);
    socket.binaryType = "blob";
    this.socket = socket;
    socket.onopen = () => {
      this.retries = 0;
    };
    socket.onmessage = (event: MessageEvent<Blob | string>) => {
      if (typeof event.data === "string") this.event(JSON.parse(event.data) as BrowserEvent);
      else this.draw(event.data);
    };
    socket.onclose = (event) => {
      if (this.socket !== socket || this.closed) return;
      this.socket = null;
      const delay = Math.min(10_000, 500 * 2 ** this.retries);
      this.retries += 1;
      this.waiting.value = event.reason ? `${capitalized(event.reason)}. Trying again…` : "Reconnecting…";
      this.retryTimer = window.setTimeout(() => {
        if (!this.closed) this.connect();
      }, delay);
    };
  }

  private event(event: BrowserEvent): void {
    switch (event.type) {
      case "tabs": {
        this.tabs.value = event.tabs;
        this.shown.value = event.shown;
        const tab = event.tabs.find((t) => t.id === event.shown);
        setTitle(this.key, tab ? tabName(tab) : "");
        break;
      }
      case "viewport":
        this.page = { width: event.width, height: event.height };
        break;
      case "loading":
        this.loading.value = event.loading;
        break;
      case "dialog":
        this.dialog.value = { kind: event.kind, message: event.message, default: event.default };
        break;
      case "dialog_closed":
        this.dialog.value = null;
        break;
      case "copied":
        void navigator.clipboard?.writeText(event.text).catch(() => say("The browser's selection couldn't be copied here."));
        break;
      default:
        event satisfies never;
    }
  }

  private draw(image: Blob): void {
    if (this.decoding) {
      this.next = image;
      return;
    }
    this.decoding = true;
    void createImageBitmap(image)
      .then((bitmap) => {
        if (this.closed) return bitmap.close();
        if (this.canvas.width !== bitmap.width) this.canvas.width = bitmap.width;
        if (this.canvas.height !== bitmap.height) this.canvas.height = bitmap.height;
        this.canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
        bitmap.close();
        this.waiting.value = null;
      })
      .catch(() => undefined)
      .finally(() => {
        this.decoding = false;
        const next = this.next;
        this.next = null;
        if (next) this.draw(next);
      });
  }

  private send(input: BrowserInput): void {
    if (this.socket?.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify(input));
  }

  private sendResize(): void {
    this.send({ type: "resize", ...this.size() });
  }

  /** The page point under a pointer, or null off the image. */
  private pointOf(event: { clientX: number; clientY: number }): { x: number; y: number } | null {
    const box = this.canvas.getBoundingClientRect();
    const image = { width: this.canvas.width, height: this.canvas.height };
    return pagePoint(box, image, this.page, event.clientX - box.left, event.clientY - box.top);
  }

  private mouse(action: MouseAction, event: PointerEvent | WheelEvent, extra: { button?: number; clicks?: number; dx?: number; dy?: number } = {}): void {
    const point = this.pointOf(event);
    if (!point) return;
    this.send({
      type: "mouse",
      action,
      x: point.x,
      y: point.y,
      button: extra.button === undefined ? "none" : buttonOf(extra.button),
      buttons: this.buttons,
      clicks: extra.clicks ?? 0,
      delta_x: extra.dx ?? 0,
      delta_y: extra.dy ?? 0,
      modifiers: modifiersOf(event, this.mac),
    });
  }

  private readonly pointerDown = (event: PointerEvent) => {
    event.preventDefault();
    this.focus();
    this.canvas.setPointerCapture(event.pointerId);
    if (event.pointerType === "touch") {
      this.touch = { id: event.pointerId, x: event.clientX, y: event.clientY, scrolling: false };
      return;
    }
    this.pressAt(event);
  };

  private pressAt(event: PointerEvent, button = event.button, buttons = event.buttons): void {
    const now = { at: event.timeStamp, x: event.clientX, y: event.clientY, button };
    const clicks = clicksAfter(this.lastPress, now);
    this.lastPress = { ...now, clicks };
    this.buttons = buttons;
    this.mouse("press", event, { button, clicks });
  }

  private readonly pointerMove = (event: PointerEvent) => {
    const touch = this.touch;
    if (touch?.id === event.pointerId) {
      // A finger that moves scrolls the page, as it would on the phone's own browser.
      const dx = touch.x - event.clientX;
      const dy = touch.y - event.clientY;
      if (!touch.scrolling && Math.hypot(dx, dy) < 8) return;
      touch.scrolling = true;
      touch.x = event.clientX;
      touch.y = event.clientY;
      this.mouse("wheel", event, { dx, dy });
      return;
    }
    if (event.pointerType === "touch") return;
    this.move = event;
    if (!this.moveFrame) {
      this.moveFrame = requestAnimationFrame(() => {
        this.moveFrame = 0;
        const move = this.move;
        this.move = null;
        if (move) {
          this.buttons = move.buttons;
          this.mouse("move", move);
        }
      });
    }
  };

  private readonly pointerUp = (event: PointerEvent) => {
    const touch = this.touch;
    if (touch?.id === event.pointerId) {
      this.touch = null;
      // A tap is a click.
      if (!touch.scrolling && event.type === "pointerup") {
        this.pressAt(event, 0, 1);
        this.buttons = 0;
        this.mouse("release", event, { button: 0, clicks: this.lastPress?.clicks ?? 1 });
      }
      return;
    }
    this.buttons = event.buttons;
    this.mouse("release", event, { button: event.button, clicks: this.lastPress?.clicks ?? 1 });
  };

  /** Scrolls the page; a sideways swipe or Shift+wheel moves along the
   * strip instead, as it does over a terminal. */
  private readonly wheel = (event: WheelEvent) => {
    event.preventDefault();
    const sideways = Math.abs(event.deltaX) > Math.abs(event.deltaY) || (event.shiftKey && event.deltaX === 0);
    if (sideways) {
      this.screen.closest(".w-cols")?.scrollBy({ left: event.deltaX || event.deltaY });
      return;
    }
    const height = this.page.height || this.screen.clientHeight;
    this.mouse("wheel", event, { dx: wheelPixels(event.deltaX, event.deltaMode, height), dy: wheelPixels(event.deltaY, event.deltaMode, height) });
  };

  private readonly keyDown = (event: KeyboardEvent) => {
    if (this.shortcuts(event, (text) => this.type(text))) {
      event.stopPropagation();
      return;
    }
    const key = keyOf(event, this.mac);
    if (!key) return;
    const clipboard = clipboardAction(key.modifiers, key.key);
    // The paste that follows brings this clipboard's text.
    if (clipboard === "paste") return;
    if (clipboard === "copy") this.send({ type: "copy" });
    event.preventDefault();
    this.held.add(event.code || event.key);
    this.send({ type: "key", ...key });
  };

  /** Releases only what the page saw pressed: a key pressed elsewhere, such
   * as Enter in the address bar, may be let go here. */
  private readonly keyUp = (event: KeyboardEvent) => {
    const key = keyOf(event, this.mac);
    if (!key || !this.held.delete(event.code || event.key)) return;
    event.preventDefault();
    this.send({ type: "key", ...key });
  };

  /** What an input method composed, once it's done. */
  private readonly composed = (event: CompositionEvent) => {
    if (event.data) this.type(event.data);
    this.input.value = "";
  };

  /** Text typed without keys the browser could take, as from a phone's keyboard. */
  private readonly typed = (event: Event) => {
    if ((event as InputEvent).isComposing) return;
    if (this.input.value) this.type(this.input.value);
    this.input.value = "";
  };

  private readonly pasted = (event: ClipboardEvent) => {
    event.preventDefault();
    const text = event.clipboardData?.getData("text/plain");
    if (text) this.type(text);
    else if (event.clipboardData?.files.length) say("Files can't be pasted into the browser.");
  };
}

function capitalized(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}
