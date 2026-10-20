import assert from "node:assert/strict";
import { test } from "node:test";

import fc from "fast-check";

import { wheelTravel } from "./wheel.ts";

// PROPTEST_CASES sets how many cases each property runs, as it does for Rust's.
fc.configureGlobal({ numRuns: Number(process.env["PROPTEST_CASES"]) || 100 });

/** Steps a run of wheel events moves, carrying between them. */
function travel(deltas: readonly number[], step: number): number {
  let carried = 0;
  let steps = 0;
  for (const delta of deltas) {
    const moved = wheelTravel(carried, delta, 0, step, 24);
    steps += moved.steps;
    carried = moved.carried;
  }
  return steps;
}

// Pixel steps of a power of two, so the arithmetic is exact.
const step = fc.constantFrom(8, 16, 32);

test("a swipe moves as many steps however its travel is split", () => {
  fc.assert(
    fc.property(step, fc.array(fc.integer({ min: 0, max: 200 }), { maxLength: 30 }), fc.boolean(), (step, sizes, up) => {
      const deltas = sizes.map((d) => (up ? -d : d));
      const total = deltas.reduce((a, b) => a + b, 0);
      assert.equal(travel(deltas, step), Math.trunc(total / step) + 0);
    }),
  );
});

test("turning the other way drops what was carried", () => {
  fc.assert(
    fc.property(step, fc.integer({ min: 1, max: 1000 }), fc.integer({ min: 1, max: 1000 }), (step, down, up) => {
      const after = wheelTravel(wheelTravel(0, down, 0, step, 24).carried, -up, 0, step, 24);
      assert.equal(after.steps, Math.trunc(-up / step) + 0);
      assert.ok(Math.abs(after.carried) < 1);
    }),
  );
});

test("whatever is carried is less than a step", () => {
  fc.assert(
    fc.property(fc.double({ min: -1, max: 1, noNaN: true, maxExcluded: true, minExcluded: true }), fc.double({ min: -1e4, max: 1e4, noNaN: true }), fc.constantFrom(0, 1, 2), fc.double({ min: 1, max: 64, noNaN: true }), (carried, delta, mode, step) => {
      const moved = wheelTravel(carried, delta, mode, step, 24);
      assert.ok(Number.isInteger(moved.steps));
      assert.ok(Math.abs(moved.carried) < 1);
      assert.ok(!Object.is(moved.steps, -0));
    }),
  );
});
