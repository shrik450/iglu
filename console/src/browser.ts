// The workspace's browser in a column: its display, shown over VNC by
// noVNC, through iglud's WebSocket, which carries RFB in binary frames and
// iglu's own messages in text ones. Keys go to the browser from the
// pane's own text area, so the console's shortcuts come first and an input
// method or a phone's keyboard types into it. Closing the pane only stops
// showing it; the browser keeps running in the workspace, for agents too.
// A dropped connection reconnects on its own, as a terminal's does.

import RFB, { type Gesture, type RawChannel } from "@novnc/novnc";
import { signal } from "@preact/signals";

import type { BrowserEvent } from "./generated/BrowserEvent.ts";
import type { BrowserInput } from "./generated/BrowserInput.ts";
import { CONTROL, clipboardShortcut, displaySize, isModifier, keyFor, keysymOf, modifiersFor, preferred, scaleOf, typing } from "./state/browser.ts";
import { api, failure } from "./api/client.ts";
import { keyboardFree } from "./focus.ts";
import { MAX_FILE_BYTES } from "./state/paste.ts";
import { say } from "./state/store.ts";
import { panes } from "./terminal.ts";

/** Addresses to open in a workspace's browser, by workspace, from before its
 * column's pane was there to take them. */
const opening = new Map<string, string>();

/** Opens `address` in the workspace's browser once its column shows; null
 * forgets what was to be opened. */
export function openOnceShown(workspace: string, address: string | null): void {
  if (address === null) opening.delete(workspace);
  else opening.set(workspace, address);
}

/** RFB's ClientCutText, which carries a paste's text to the display. */
const CLIENT_CUT_TEXT = 6;
/** How long a paste waits for the display to take its text, when it never
 * asks for it, as a server without extended clipboards doesn't. */
const PASTE_WAIT_MS = 500;
const V = 0x76;

/** iglud's WebSocket as noVNC takes it: RFB's half of it. */
class Channel implements RawChannel {
  binaryType = "arraybuffer";
  protocol = "";
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  private readonly socket: WebSocket;
  /** Settles when the display next has text sent to it. */
  private cutText: (() => void) | null = null;

  constructor(socket: WebSocket) {
    this.socket = socket;
  }

  get readyState(): number {
    return this.socket.readyState;
  }

  send(data: ArrayBufferLike | ArrayBufferView): void {
    this.socket.send(data);
    const bytes = ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : new Uint8Array(data);
    if (bytes[0] === CLIENT_CUT_TEXT) this.cutText?.();
  }

  close(): void {
    this.socket.close(1000);
  }

  /** Settles once text is next sent to the display, or after a while. With
   * extended clipboards, a paste only says there's text, and the display
   * asks for it before it has it. */
  sentText(): Promise<void> {
    return new Promise((resolve) => {
      const done = () => {
        clearTimeout(timer);
        this.cutText = null;
        resolve();
      };
      const timer = setTimeout(done, PASTE_WAIT_MS);
      this.cutText = done;
    });
  }
}

/** noVNC's RFB, drawing the display in the screen's device pixels with the
 * encoding that suits how it's drawn, and scrolling to a finger's drag as
 * a phone's own browser does. */
class Screen extends RFB {
  private readonly hardware: boolean;
  /** The display's size for the screen, while asking the display for it. */
  private sizing: { w: number; h: number } | null = null;
  /** Where a finger that's scrolling last was. */
  private finger: { x: number; y: number } | null = null;

  constructor(target: HTMLElement, channel: Channel, hardware: boolean) {
    super(target, channel);
    this.hardware = hardware;
  }

  /** Asks the display for the screen's size again, as the pixel ratio changed. */
  resized(): void {
    this._requestRemoteResize();
  }

  protected override _sendEncodings(): void {
    const send = RFB.messages.clientEncodings;
    RFB.messages.clientEncodings = (sock, encodings) => send(sock, preferred(encodings, this.hardware));
    try {
      super._sendEncodings();
    } finally {
      RFB.messages.clientEncodings = send;
    }
  }

  protected override _requestRemoteResize(): void {
    // A screen with no size, as while it's hidden, leaves the display at
    // the one it has; the display would end its VNC server for a size of
    // nothing. noVNC asks again when the screen next resizes.
    this.sizing = displaySize(super._screenSize(), scaleOf(devicePixelRatio));
    if (!this.sizing) return;
    try {
      super._requestRemoteResize();
    } finally {
      this.sizing = null;
    }
  }

  protected override _screenSize(): { w: number; h: number } {
    return this.sizing ?? super._screenSize();
  }

  protected override _handleGesture(event: CustomEvent<Gesture>): void {
    if (event.detail.type !== "drag") return super._handleGesture(event);
    const { clientX: x, clientY: y } = event.detail;
    const last = this.finger;
    this.finger = event.type === "gestureend" ? null : { x, y };
    if (!last) return;
    // noVNC turns wheel turns into the steps VNC sends.
    this._canvas.dispatchEvent(new WheelEvent("wheel", { deltaX: last.x - x, deltaY: last.y - y, clientX: x, clientY: y, bubbles: true, cancelable: true }));
  }
}

export class BrowserPane {
  /** The last download the browser finished, until it's put away. */
  readonly downloaded = signal<{ name: string | null; path: string; url: string | null } | null>(null);
  /** Why there's nothing to see yet, while there isn't. */
  readonly waiting = signal<string | null>("Opening the browser…");

  private readonly key: string;
  private readonly workspace: string;
  private readonly container: HTMLElement;
  /** Holds the keyboard, so an input method composes into it and a paste lands in it. */
  private readonly input: HTMLTextAreaElement;
  private readonly mac: boolean;
  private readonly shortcuts: (event: KeyboardEvent) => boolean;
  private socket: WebSocket | null = null;
  private channel: Channel | null = null;
  private screen: Screen | null = null;
  private closed = false;
  private retries = 0;
  private retryTimer = 0;
  private ratio: MediaQueryList | null = null;
  /** Keys pressed in the browser and not yet let go, by code, as the keysyms sent. */
  private readonly held = new Map<string, number>();
  /** Modifiers the browser is to hold, once what's ahead of them is sent. */
  private modifiers: number[] = [];
  /** Keys the browser has been sent down and not up. */
  private readonly down = new Set<number>();
  /** Set while a paste waits for the display to take its text: keys
   * pressed meanwhile go after it, in turn. */
  private outbox: Promise<void> | null = null;
  /** When a copy was last asked for, so the browser's copy that follows is kept here. */
  private copied = -Infinity;
  /** An address to open once the browser shows. */
  private wanted: string | null = null;

  constructor(options: {
    container: HTMLElement;
    workspace: string;
    session: string;
    mac: boolean;
    /** Whether the console takes a key; the browser has it otherwise. */
    shortcuts: (event: KeyboardEvent) => boolean;
    wantsFocus: () => boolean;
    onFocus: () => void;
  }) {
    const { container, workspace, session } = options;
    this.key = `${workspace}/${session}`;
    this.workspace = workspace;
    this.container = container;
    this.mac = options.mac;
    this.shortcuts = options.shortcuts;
    this.input = document.createElement("textarea");
    this.input.className = "browser-input";
    this.input.setAttribute("autocapitalize", "off");
    this.input.setAttribute("autocomplete", "off");
    this.input.spellcheck = false;
    container.append(this.input);

    this.input.addEventListener("keydown", this.keyDown);
    this.input.addEventListener("keyup", this.keyUp);
    this.input.addEventListener("compositionend", this.composed);
    this.input.addEventListener("input", this.typed);
    this.input.addEventListener("paste", this.pasted);
    this.input.addEventListener("focus", options.onFocus);
    this.input.addEventListener("blur", this.letGo);
    // noVNC stops the screen's mouse events, but not pointer events, which come first.
    container.addEventListener("pointerdown", this.pointerDown);
    container.addEventListener("wheel", this.wheel, { capture: true, passive: false });
    this.watchRatio();
    panes.set(this.key, this);
    this.wanted = opening.get(workspace) ?? null;
    opening.delete(workspace);
    if (options.wantsFocus() && keyboardFree()) this.focus();
    this.connect();
  }

  describe(title: string, prefix: string): void {
    this.input.setAttribute("aria-label", `Browser, ${title}`);
    this.input.setAttribute("aria-description", `${prefix}, then Tab leaves the browser; ${prefix}, then ? lists shortcuts.`);
  }

  focus(): void {
    this.input.focus({ preventScroll: true });
  }

  /** Types `text` into the browser, as keys, or pasted when keys can't type it. */
  type(text: string): void {
    const keys = typing(text);
    if (!keys) {
      this.paste(text);
      return;
    }
    // Text typed is typed without the modifiers that made it.
    this.hold([]);
    for (const key of keys) this.tap(key);
  }

  /** Presses and releases `key`, as a KeyboardEvent names it. */
  press(key: string): void {
    const keysym = keysymOf(key, 0);
    if (keysym !== null) this.tap(keysym);
  }

  /** Opens `address` in a new tab of the browser, once it shows. */
  open(address: string): void {
    this.wanted = address;
    this.openWanted();
    this.focus();
  }

  dispose(): void {
    this.closed = true;
    clearTimeout(this.retryTimer);
    this.ratio?.removeEventListener("change", this.ratioChanged);
    this.screen?.disconnect();
    this.screen = null;
    this.socket?.close(1000);
    this.socket = null;
    panes.delete(this.key);
    this.container.removeEventListener("pointerdown", this.pointerDown);
    this.container.removeEventListener("wheel", this.wheel, { capture: true });
    this.container.replaceChildren();
  }

  private connect(): void {
    const url = new URL(`/v1/workspaces/${this.workspace}/browser`, location.href);
    url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
    url.searchParams.set("scale", String(scaleOf(devicePixelRatio)));
    const socket = new WebSocket(url);
    socket.binaryType = "arraybuffer";
    const channel = new Channel(socket);
    this.socket = socket;
    this.channel = channel;
    socket.onopen = (event) => channel.onopen?.(event);
    socket.onerror = (event) => channel.onerror?.(event);
    socket.onmessage = (event: MessageEvent<ArrayBuffer | string>) => {
      if (typeof event.data === "string") this.event(JSON.parse(event.data) as BrowserEvent);
      else channel.onmessage?.(event);
    };
    socket.onclose = (event) => {
      channel.onclose?.(event);
      if (this.socket !== socket || this.closed) return;
      this.socket = null;
      this.channel = null;
      this.screen = null;
      this.held.clear();
      this.modifiers = [];
      this.down.clear();
      this.outbox = null;
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
      case "display":
        this.show(event.hardware);
        break;
      case "downloaded":
        this.downloaded.value = {
          name: event.name,
          path: event.path,
          url: event.name === null ? null : `/v1/workspaces/${this.workspace}/downloads/${encodeURIComponent(event.name)}`,
        };
        break;
      default:
        event satisfies never;
    }
  }

  /** Starts showing the display, which iglud says how it's drawn first. */
  private show(hardware: boolean): void {
    if (!this.channel || this.screen) return;
    const screen = new Screen(this.container, this.channel, hardware);
    screen.scaleViewport = true;
    screen.resizeSession = true;
    screen.focusOnClick = false;
    screen.background = "var(--term-bg)";
    screen.addEventListener("connect", () => {
      this.retries = 0;
      this.waiting.value = null;
      this.openWanted();
    });
    screen.addEventListener("clipboard", (event) => this.copy((event as CustomEvent<{ text: string }>).detail.text));
    this.screen = screen;
  }

  private openWanted(): void {
    if (this.wanted === null || this.waiting.value !== null) return;
    this.send({ type: "open", address: this.wanted });
    this.wanted = null;
  }

  private send(input: BrowserInput): void {
    if (this.socket?.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify(input));
  }

  /** Follows the screen's pixel ratio, as when the window moves to another screen. */
  private watchRatio(): void {
    this.ratio?.removeEventListener("change", this.ratioChanged);
    this.ratio = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
    this.ratio.addEventListener("change", this.ratioChanged);
  }

  private readonly ratioChanged = () => {
    this.watchRatio();
    this.send({ type: "scale", scale: scaleOf(devicePixelRatio) });
    this.screen?.resized();
  };

  /** The browser's copy, kept on this computer's clipboard while the
   * person's in the browser; a failure is told only of a copy they asked for. */
  private copy(text: string): void {
    if (document.activeElement !== this.input) return;
    const asked = performance.now() - this.copied < 2000;
    void navigator.clipboard?.writeText(text).catch(() => {
      if (asked) say("The browser's selection couldn't be copied here.");
    });
  }

  /** Gives the browser `text` on its clipboard and pastes it there, with
   * Control+V. */
  private paste(text: string): void {
    const { screen, channel } = this;
    if (!screen || !channel) return;
    this.later(async () => {
      screen.clipboardPasteFrom(text);
      await channel.sentText();
      const held = this.down.has(CONTROL);
      if (!held) this.deliver(CONTROL, true);
      this.deliver(V, true);
      this.deliver(V, false);
      if (!held) this.deliver(CONTROL, false);
    });
  }

  /** Holds just these modifiers in the browser. */
  private hold(modifiers: number[]): void {
    for (const key of this.modifiers) if (!modifiers.includes(key)) this.sendKey(key, false);
    for (const key of modifiers) if (!this.modifiers.includes(key)) this.sendKey(key, true);
    this.modifiers = modifiers;
  }

  private tap(keysym: number): void {
    this.sendKey(keysym, true);
    this.sendKey(keysym, false);
  }

  /** Sends a key, after a paste that's waiting. */
  private sendKey(keysym: number, down: boolean): void {
    if (this.outbox) this.later(() => this.deliver(keysym, down));
    else this.deliver(keysym, down);
  }

  /** Does `send` after what's waiting to be sent, if anything is. */
  private later(send: () => void | Promise<void>): void {
    const screen = this.screen;
    const after = (this.outbox ?? Promise.resolve()).then(() => (this.screen === screen ? send() : undefined));
    this.outbox = after;
    void after.finally(() => {
      if (this.outbox === after) this.outbox = null;
    });
  }

  private deliver(keysym: number, down: boolean): void {
    this.screen?.sendKey(keysym, null, down);
    if (down) this.down.add(keysym);
    else this.down.delete(keysym);
  }

  private readonly pointerDown = () => {
    this.focus();
  };

  /** A sideways swipe or Shift+wheel moves along the strip, as it does over
   * a terminal; the browser scrolls to the rest. */
  private readonly wheel = (event: WheelEvent) => {
    // A finger's drag, which noVNC scrolls the browser to.
    if (!event.isTrusted) return;
    const sideways = Math.abs(event.deltaX) > Math.abs(event.deltaY) || (event.shiftKey && event.deltaX === 0);
    if (!sideways) return;
    event.preventDefault();
    event.stopPropagation();
    this.container.closest(".w-cols")?.scrollBy({ left: event.deltaX || event.deltaY });
  };

  private readonly keyDown = (event: KeyboardEvent) => {
    if (this.shortcuts(event)) {
      event.stopPropagation();
      return;
    }
    // An input method's or a phone keyboard's, typed once it's done; see composed and typed.
    if (event.isComposing || event.key === "Process" || event.key === "Unidentified" || event.key === "Dead") return;
    const clipboard = clipboardShortcut(event, this.mac);
    // The paste event that follows brings this clipboard's text.
    if (clipboard === "paste") return;
    if (clipboard === "copy") this.copied = performance.now();
    event.preventDefault();
    // The browser repeats a key held down itself.
    if (event.repeat) return;
    this.hold(modifiersFor(event, this.mac));
    if (isModifier(event.key)) return;
    const keysym = keysymOf(keyFor(event, this.mac), event.location);
    if (keysym === null) return;
    // A Mac doesn't tell of a key let go while Command is held.
    if (this.mac && event.metaKey) {
      this.tap(keysym);
      return;
    }
    this.held.set(event.code || event.key, keysym);
    this.sendKey(keysym, true);
  };

  /** Lets go only of what the browser saw pressed. */
  private readonly keyUp = (event: KeyboardEvent) => {
    if (isModifier(event.key)) {
      this.hold(modifiersFor(event, this.mac));
      return;
    }
    const keysym = this.held.get(event.code || event.key);
    if (keysym === undefined) return;
    this.held.delete(event.code || event.key);
    event.preventDefault();
    this.sendKey(keysym, false);
  };

  /** Keys held as the browser loses the keyboard would stay held in it. */
  private readonly letGo = () => {
    for (const keysym of this.held.values()) this.sendKey(keysym, false);
    this.held.clear();
    this.hold([]);
  };

  /** What an input method composed, once it's done. */
  private readonly composed = (event: CompositionEvent) => {
    if (event.data) this.type(event.data);
    this.input.value = "";
  };

  /** Text typed without keys the browser could take, as from a phone's keyboard. */
  private readonly typed = (event: Event) => {
    const input = event as InputEvent;
    if (input.isComposing) return;
    if (input.inputType === "deleteContentBackward") this.press("Backspace");
    else if (input.inputType === "insertLineBreak" || input.inputType === "insertParagraph") this.press("Enter");
    else if (this.input.value) this.type(this.input.value);
    this.input.value = "";
  };

  /** A paste's text is pasted in the browser; its files are kept in the
   * workspace, where the browser's file chooser finds them. */
  private readonly pasted = (event: ClipboardEvent) => {
    event.preventDefault();
    const text = event.clipboardData?.getData("text/plain");
    const files = Array.from(event.clipboardData?.files ?? []);
    if (text) this.paste(text);
    else if (files.length) void this.keep(files);
  };

  private async keep(files: File[]): Promise<void> {
    const large = files.find((file) => file.size > MAX_FILE_BYTES);
    if (large) {
      say(`${large.name || "That file"} is larger than 16 MB, so it can't be pasted.`);
      return;
    }
    try {
      const paths: string[] = [];
      for (const file of files) paths.push((await api.keepFile(this.workspace, file)).path);
      say(paths.length === 1 ? `Kept in the workspace at ${paths[0]}, for the browser to open.` : `Kept ${paths.length} files in the workspace, in ${dirOf(paths[0]!)}, for the browser to open.`);
    } catch (error) {
      say(`That couldn't be pasted: ${failure(error)}`);
    }
  }
}

function dirOf(path: string): string {
  return path.slice(0, path.lastIndexOf("/")) || "/";
}

function capitalized(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}
