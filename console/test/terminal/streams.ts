// Generated terminal output, for checking the renderer against the core:
// text of every width, cursor movement, erasing, scrolling, styles, modes and
// links, at terminal sizes small enough to wrap, split into chunks anywhere,
// even inside a character or a sequence, as output arrives over a socket.

import fc from "fast-check";

const ESC = "\x1b";
const CSI = `${ESC}[`;

/** Printable ASCII, from `min` to `max` characters long. */
function ascii(min: number, max: number): fc.Arbitrary<string> {
  return fc.string({ unit: fc.integer({ min: 0x20, max: 0x7e }).map((code) => String.fromCharCode(code)), minLength: min, maxLength: max });
}

const count = fc.integer({ min: 0, max: 30 });
const colour = fc.integer({ min: 0, max: 255 });

const sgr = fc
  .array(
    fc.oneof(
      fc.constantFrom(0, 1, 2, 3, 4, 5, 7, 8, 9, 21, 22, 23, 24, 25, 27, 28, 29, 39, 49, 53, 55).map(String),
      fc.integer({ min: 30, max: 37 }).map(String),
      fc.integer({ min: 40, max: 47 }).map(String),
      fc.integer({ min: 90, max: 97 }).map(String),
      fc.integer({ min: 100, max: 107 }).map(String),
      fc.tuple(fc.constantFrom(38, 48, 58), colour).map(([kind, n]) => `${kind};5;${n}`),
      fc.tuple(fc.constantFrom(38, 48, 58), colour, colour, colour).map(([kind, r, g, b]) => `${kind};2;${r};${g};${b}`),
      fc.constantFrom("4:3", "4:0"),
    ),
    { minLength: 0, maxLength: 4 },
  )
  .map((codes) => `${CSI}${codes.join(";")}m`);

/** One thing a program prints. */
const token = fc.oneof(
  { weight: 6, arbitrary: ascii(1, 24) },
  {
    weight: 3,
    arbitrary: fc.constantFrom("é", "ü", "ñ", "中文", "한글", "ｱ", "😀", "👍🏽", "👨‍👩‍👧", "é", "ạ̈", "─", "│", "┼", "╬", "▗", "▖", "▝", "▘", "█", "▌", "❯", "", "", "​", "️"),
  },
  { weight: 3, arbitrary: fc.constantFrom("\r", "\n", "\r\n", "\t", "\b", "\x07") },
  { weight: 3, arbitrary: sgr },
  {
    weight: 4,
    arbitrary: fc.oneof(
      fc.tuple(fc.integer({ min: 0, max: 45 }), fc.integer({ min: 0, max: 130 })).map(([row, col]) => `${CSI}${row};${col}H`),
      fc.tuple(count, fc.constantFrom("A", "B", "C", "D", "E", "F", "G", "d", "`", "a", "e")).map(([n, final]) => `${CSI}${n}${final}`),
      fc.tuple(fc.integer({ min: 0, max: 3 }), fc.constantFrom("J", "K")).map(([n, final]) => `${CSI}${n}${final}`),
      fc.tuple(count, fc.constantFrom("L", "M", "@", "P", "X", "S", "T", "b", "I", "Z")).map(([n, final]) => `${CSI}${n}${final}`),
      fc.tuple(fc.integer({ min: 0, max: 40 }), fc.integer({ min: 0, max: 40 })).map(([top, bottom]) => `${CSI}${top};${bottom}r`),
      fc.constant(`${CSI}r`),
    ),
  },
  {
    weight: 2,
    arbitrary: fc.oneof(
      fc.tuple(fc.constantFrom("1049", "47", "1047", "7", "6", "25", "2004", "1"), fc.constantFrom("h", "l")).map(([mode, set]) => `${CSI}?${mode}${set}`),
      fc.tuple(fc.constantFrom("4", "20"), fc.constantFrom("h", "l")).map(([mode, set]) => `${CSI}${mode}${set}`),
      fc.constantFrom(`${ESC}7`, `${ESC}8`, `${CSI}s`, `${CSI}u`, `${ESC}D`, `${ESC}M`, `${ESC}E`, `${ESC}H`, `${CSI}3g`, `${CSI}0g`, `${ESC}c`, `${ESC}#8`),
    ),
  },
  {
    weight: 1,
    arbitrary: fc.oneof(
      ascii(0, 12).map((title) => `${ESC}]2;${title}\x07`),
      fc.tuple(fc.nat().map((n) => `https://example.org/${n}`), ascii(1, 12)).map(([url, text]) => `${ESC}]8;;${url}${ESC}\\${text}${ESC}]8;;${ESC}\\`),
    ),
  },
);

export interface Stream {
  chunks: Uint8Array[];
  /** Whether the terminal draws after each chunk, before the next arrives. */
  pauses: boolean[];
  cols: number;
  rows: number;
}

export function streams(): fc.Arbitrary<Stream> {
  return fc
    .record({
      tokens: fc.array(token, { minLength: 1, maxLength: 120 }),
      cuts: fc.array(fc.double({ min: 0, max: 1, noNaN: true }), { maxLength: 12 }),
      pauses: fc.array(fc.boolean(), { minLength: 13, maxLength: 13 }),
      cols: fc.oneof(fc.integer({ min: 2, max: 12 }), fc.integer({ min: 13, max: 120 })),
      rows: fc.oneof(fc.integer({ min: 1, max: 4 }), fc.integer({ min: 5, max: 40 })),
    })
    .map(({ tokens, cuts, pauses, cols, rows }) => {
      const bytes = new TextEncoder().encode(tokens.join(""));
      const offsets = [...new Set(cuts.map((cut) => Math.floor(cut * bytes.length)))].sort((a, b) => a - b);
      const chunks: Uint8Array[] = [];
      let from = 0;
      for (const offset of [...offsets, bytes.length]) {
        if (offset > from) chunks.push(bytes.subarray(from, offset));
        from = Math.max(from, offset);
      }
      return { chunks, pauses: pauses.slice(0, chunks.length), cols, rows };
    });
}
