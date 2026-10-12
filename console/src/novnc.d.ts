// What the console uses of noVNC, which has no types of its own. The
// protected members are noVNC's internals, as of the version package.json
// pins: the browser pane overrides them to draw the display at the
// screen's pixel ratio, choose its encoding and scroll by touch.

declare module "@novnc/novnc" {
  /** A channel that carries RFB, as a WebSocket does. */
  export interface RawChannel {
    binaryType: string;
    protocol: string;
    readonly readyState: number;
    onopen: ((event: Event) => void) | null;
    onmessage: ((event: MessageEvent) => void) | null;
    onclose: ((event: CloseEvent) => void) | null;
    onerror: ((event: Event) => void) | null;
    send(data: ArrayBufferLike | ArrayBufferView): void;
    close(): void;
  }

  export interface Gesture {
    type: "onetap" | "twotap" | "threetap" | "drag" | "longpress" | "twodrag" | "pinch";
    clientX: number;
    clientY: number;
  }

  export default class RFB extends EventTarget {
    constructor(target: HTMLElement, channel: string | RawChannel, options?: { shared?: boolean });
    scaleViewport: boolean;
    resizeSession: boolean;
    focusOnClick: boolean;
    background: string;
    sendKey(keysym: number, code: string | null, down?: boolean): void;
    clipboardPasteFrom(text: string): void;
    disconnect(): void;

    static messages: { clientEncodings(sock: unknown, encodings: number[]): void };
    protected _canvas: HTMLCanvasElement;
    protected _sendEncodings(): void;
    protected _requestRemoteResize(): void;
    protected _screenSize(): { w: number; h: number };
    protected _handleGesture(event: CustomEvent<Gesture>): void;
  }
}
