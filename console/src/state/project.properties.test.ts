import assert from "node:assert/strict";
import { test } from "node:test";

import fc from "fast-check";

import { parsePorts } from "./project.ts";

// PROPTEST_CASES sets how many cases each property runs, as it does for Rust's.
fc.configureGlobal({ numRuns: Number(process.env["PROPTEST_CASES"]) || 100 });

const digitsOnly = (part: string) => [...part].every((c) => c >= "0" && c <= "9");

test("ports read back as the numbers typed, once each and in order, or the first part that isn't one is named", () => {
  const part = fc.oneof(fc.nat(99_999).map(String), fc.stringMatching(/^[0-9a-z.-]{1,6}$/));
  fc.assert(
    fc.property(fc.array(part, { maxLength: 8 }), fc.constantFrom(", ", " ", ",", "\n"), (parts, between) => {
      const parsed = parsePorts(parts.join(between));
      const bad = parts.find((p) => !digitsOnly(p));
      if (bad === undefined) assert.deepEqual(parsed, { ports: [...new Set(parts.map(Number))].sort((a, b) => a - b) });
      else assert.deepEqual(parsed, { notANumber: bad });
    }),
  );
});
