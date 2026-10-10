// What a program asks of the terminal that wterm doesn't answer itself. Pure.
//
// Shells and TUIs query the terminal and wait for the answer. wterm answers
// most of them: device attributes, cursor and status reports, modes, the
// foreground and background colours, and the window's size. It leaves three
// that programs ask for: the secondary device attributes, which nvim and
// tmux send, the terminal's name and version, which fish and tmux send, and
// the cursor's colour. The console answers those.

/** A reply, due once the terminal has been given `chunk[0..end)`. */
export interface Reply {
  end: number;
  text: string;
}

export interface Scan {
  replies: Reply[];
  /** The start of a query that the next chunk may finish. */
  carry: Uint8Array;
}

const ESC = 0x1b;
const BEL = 0x07;
const ST = 0x5c; // `\`, which ends `ESC \`
/** Queries are short; a longer sequence is something else, and isn't carried. */
const LONGEST = 32;

const CURSOR_QUERY = "12;?";
/** xterm's form: type 1, firmware version 10, no ROM cartridge. */
const SECONDARY = "\x1b[>1;10;0c";
/** XTVERSION's answer: the terminal's name. */
const VERSION = "\x1bP>|iglu\x1b\\";

/** `#rrggbb` as X11 colour, `rgb:rrrr/gggg/bbbb`. */
export function xcolour(hex: string): string | null {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  return match ? `rgb:${match.slice(1, 4).map((pair) => pair + pair).join("/")}`.toLowerCase() : null;
}

function text(bytes: Uint8Array, from: number, to: number): string {
  return String.fromCharCode(...bytes.subarray(from, to));
}

/** Finds the queries in `chunk`, which follows the `carry` of the last scan.
 * `cursor` is the cursor's colour, as `#rrggbb`. */
export function scan(carry: Uint8Array, chunk: Uint8Array, cursor: string): Scan {
  const bytes = new Uint8Array(carry.length + chunk.length);
  bytes.set(carry);
  bytes.set(chunk, carry.length);
  const replies: Reply[] = [];
  const reply = (after: number, answer: string | null) => {
    if (answer) replies.push({ end: after - carry.length, text: answer });
  };
  const unfinished = (start: number): Scan => ({ replies, carry: bytes.length - start <= LONGEST ? bytes.slice(start) : new Uint8Array() });

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
      const asks = middle === j && (params === ">" || params === ">0");
      if (asks && bytes[j] === 0x63) reply(j + 1, SECONDARY);
      if (asks && bytes[j] === 0x71) reply(j + 1, VERSION);
      i = j + 1;
    } else if (kind === 0x5d) {
      // OSC, ended by BEL or by ST, which is answered in kind.
      let j = i + 2;
      while (j < bytes.length && bytes[j] !== BEL && !(bytes[j] === ESC && bytes[j + 1] === ST)) j += 1;
      if (j >= bytes.length || (bytes[j] === ESC && j + 1 >= bytes.length)) return unfinished(start);
      const end = bytes[j] === BEL ? "\x07" : "\x1b\\";
      const after = j + end.length;
      // Only a body as short as the query is read: copies can run to megabytes.
      const colour = j - (i + 2) === CURSOR_QUERY.length && text(bytes, i + 2, j) === CURSOR_QUERY ? xcolour(cursor) : null;
      if (colour) reply(after, `\x1b]12;${colour}${end}`);
      i = after;
    } else {
      i += 2;
    }
  }
  return { replies, carry: new Uint8Array() };
}
