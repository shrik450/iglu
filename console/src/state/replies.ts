// What a program asks of the terminal that ghostty-web doesn't do itself. Pure.
//
// Shells and TUIs query the terminal and wait for the answer: fish asks for
// the device attributes after every prompt and holds the keyboard until they
// come, and fish, Claude Code and others ask for the background colour to
// pick a theme. ghostty-web answers only cursor-position and status reports,
// so the console answers the rest. Programs also copy to the clipboard with
// OSC 52, as nvim, tmux and agents do; the console does that copy.

/** The colours a program may ask for, as `#rrggbb`. */
export interface Palette {
  foreground: string;
  background: string;
  cursor: string;
}

/** A reply, due once the terminal has been given `chunk[0..end)`. */
export interface Reply {
  end: number;
  text: string;
}

export interface Scan {
  replies: Reply[];
  /** Text programs asked to put on the clipboard, in order. */
  copies: string[];
  /** The start of a query that the next chunk may finish. */
  carry: Uint8Array;
}

const ESC = 0x1b;
const BEL = 0x07;
const ST = 0x5c; // `\`, which ends `ESC \`
/** Queries are short; a longer sequence is something else, and isn't carried. */
const LONGEST = 32;
/** A copy can be long, split across many messages, so more of it is carried. */
const LONGEST_COPY = 1 << 20;
const COPY = "\x1b]52;";

/** A VT220 with ANSI colour, as most terminals describe themselves. */
const PRIMARY = "\x1b[?62;22c";
/** xterm's form: type 1, firmware version 10, no ROM cartridge. */
const SECONDARY = "\x1b[>1;10;0c";

const COLOURS: Record<string, keyof Palette> = { "10": "foreground", "11": "background", "12": "cursor" };

/** `#rrggbb` as X11 colour, `rgb:rrrr/gggg/bbbb`. */
export function xcolour(hex: string): string | null {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  return match ? `rgb:${match.slice(1, 4).map((pair) => pair + pair).join("/")}`.toLowerCase() : null;
}

function text(bytes: Uint8Array, from: number, to: number): string {
  return String.fromCharCode(...bytes.subarray(from, to));
}

/** Finds the queries in `chunk`, which follows the `carry` of the last scan. */
export function scan(carry: Uint8Array, chunk: Uint8Array, palette: Palette): Scan {
  const bytes = new Uint8Array(carry.length + chunk.length);
  bytes.set(carry);
  bytes.set(chunk, carry.length);
  const replies: Reply[] = [];
  const copies: string[] = [];
  const reply = (after: number, answer: string | null) => {
    if (answer) replies.push({ end: after - carry.length, text: answer });
  };
  const unfinished = (start: number): Scan => {
    const limit = text(bytes, start, start + COPY.length) === COPY ? LONGEST_COPY : LONGEST;
    return { replies, copies, carry: bytes.length - start <= limit ? bytes.slice(start) : new Uint8Array() };
  };

  let i = 0;
  while (i < bytes.length) {
    if (bytes[i] !== ESC) {
      i += 1;
      continue;
    }
    const start = i;
    const kind = bytes[i + 1];
    if (kind === undefined) return unfinished(start);
    if (kind === 0x5b) {
      // CSI: parameter bytes, intermediate bytes, then one final byte.
      let j = i + 2;
      while (j < bytes.length && bytes[j]! >= 0x30 && bytes[j]! <= 0x3f) j += 1;
      const params = text(bytes, i + 2, j);
      const middle = j;
      while (j < bytes.length && bytes[j]! >= 0x20 && bytes[j]! <= 0x2f) j += 1;
      if (j >= bytes.length) return unfinished(start);
      if (bytes[j] === 0x63 && middle === j) {
        if (params === "" || params === "0") reply(j + 1, PRIMARY);
        else if (params === ">" || params === ">0") reply(j + 1, SECONDARY);
      }
      i = j + 1;
    } else if (kind === 0x5d) {
      // OSC, ended by BEL or by ST, which is answered in kind.
      let j = i + 2;
      while (j < bytes.length && bytes[j] !== BEL && !(bytes[j] === ESC && bytes[j + 1] === ST)) j += 1;
      if (j >= bytes.length || (bytes[j] === ESC && j + 1 >= bytes.length)) return unfinished(start);
      const end = bytes[j] === BEL ? "\x07" : "\x1b\\";
      const after = j + end.length;
      const body = text(bytes, i + 2, j);
      const query = /^(1[0-2]);\?$/.exec(body);
      const colour = query ? xcolour(palette[COLOURS[query[1]!]!]) : null;
      if (query && colour) reply(after, `\x1b]${query[1]};${colour}${end}`);
      const copy = /^52;[a-z0-9]*;(.*)$/s.exec(body);
      if (copy) {
        const decoded = clipboardText(copy[1]!);
        if (decoded !== null) copies.push(decoded);
      }
      i = after;
    } else {
      i += 2;
    }
  }
  return { replies, copies, carry: new Uint8Array() };
}

/** OSC 52's base64 payload as text; null for a query ("?") or bad base64. */
function clipboardText(payload: string): string | null {
  if (payload === "?") return null;
  try {
    return new TextDecoder().decode(Uint8Array.from(atob(payload), (c) => c.charCodeAt(0)));
  } catch {
    return null;
  }
}
