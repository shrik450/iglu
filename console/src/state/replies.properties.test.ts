import assert from "node:assert/strict";
import { test } from "node:test";

import fc from "fast-check";

import { scan } from "./replies.ts";

// PROPTEST_CASES sets how many cases each property runs, as it does for Rust's.
fc.configureGlobal({ numRuns: Number(process.env["PROPTEST_CASES"]) || 100 });

const cursor = "#8FD8FF";
const bytes = (s: string) => Uint8Array.from(s, (c) => c.charCodeAt(0));
const printable = fc.string({ unit: fc.integer({ min: 0x20, max: 0x7e }).map((c) => String.fromCharCode(c)), maxLength: 300 });

/** What programs write: text, the queries the console answers, other
 * sequences, long copies, and offers to open, each ended by BEL or ST. */
const piece = fc.oneof(
  printable,
  fc.constantFrom("\x1b[>c", "\x1b[>0c", "\x1b[>q", "\x1b[>0q", "\x1b]12;?\x07", "\x1b]12;?\x1b\\"),
  fc.constantFrom("\x1b[6n", "\x1b[1;31m", "\x1b[?2004h", "\x1b7", "\x1bc", "\x1b]0;title\x07", "\r\n"),
  fc.tuple(fc.string({ unit: fc.constantFrom("A", "b", "+", "/", "="), maxLength: 400 }), fc.boolean()).map(([body, bel]) => `\x1b]52;c;${body}${bel ? "\x07" : "\x1b\\"}`),
  fc.tuple(printable.filter((s) => s.length > 0), fc.boolean()).map(([address, bel]) => `\x1b]4458;${address}${bel ? "\x07" : "\x1b\\"}`),
);

const output = fc.array(piece, { maxLength: 20 }).map((pieces) => bytes(pieces.join("")));

test("output split anywhere is answered as it would be whole", () => {
  fc.assert(
    fc.property(
      output.chain((all) => fc.tuple(fc.constant(all), fc.uniqueArray(fc.integer({ min: 0, max: all.length }), { maxLength: 12 }))),
      ([all, cuts]) => {
        const whole = scan(new Uint8Array(), all, cursor);
        const points = [0, ...cuts.sort((a, b) => a - b), all.length];
        let carry: Uint8Array = new Uint8Array();
        const replies: { end: number; text: string }[] = [];
        const opens: string[] = [];
        for (let i = 1; i < points.length; i++) {
          const from = points[i - 1]!;
          const chunk = all.subarray(from, points[i]!);
          const found = scan(carry, chunk, cursor);
          replies.push(...found.replies.map((r) => ({ end: from + r.end, text: r.text })));
          opens.push(...found.opens);
          carry = found.carry;
        }
        assert.deepEqual(replies, whole.replies);
        assert.deepEqual(opens, whole.opens);
      },
    ),
  );
});

test("replies come only where a query ended, and never answer each other", () => {
  fc.assert(
    fc.property(output, (all) => {
      const { replies } = scan(new Uint8Array(), all, cursor);
      for (const reply of replies) {
        const before = String.fromCharCode(...all.subarray(0, reply.end));
        assert.ok(/(\x1b\[>0?[cq]|\x1b\]12;\?(\x07|\x1b\\))$/.test(before), JSON.stringify(before.slice(-12)));
        // Fed back, as a terminal echoing them would, they ask nothing.
        assert.deepEqual(scan(new Uint8Array(), bytes(reply.text), cursor).replies, []);
      }
    }),
  );
});

test("any bytes at all are scanned without throwing", () => {
  fc.assert(
    fc.property(fc.uint8Array({ maxLength: 400 }), fc.uint8Array({ maxLength: 40 }), (chunk, carry) => {
      const found = scan(carry, chunk, cursor);
      for (const reply of found.replies) assert.ok(reply.end >= 0 && reply.end <= chunk.length);
    }),
  );
});
