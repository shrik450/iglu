import assert from "node:assert/strict";
import { test } from "node:test";
import { wheelTravel } from "./wheel.ts";

const PIXELS = 0;
const LINES = 1;
const PAGES = 2;

test("a swipe's small deltas add up to one step a row", () => {
  let carried = 0;
  let steps = 0;
  for (let i = 0; i < 20; i++) {
    const travel = wheelTravel(carried, 4, PIXELS, 16, 40);
    carried = travel.carried;
    steps += travel.steps;
  }
  assert.equal(steps, 5);
  assert.equal(carried, 0);
});

test("up is negative steps, and the remainder keeps its sign", () => {
  assert.deepEqual(wheelTravel(0, -40, PIXELS, 16, 40), { steps: -2, carried: -0.5 });
});

test("turning the other way drops what was carried", () => {
  assert.deepEqual(wheelTravel(0.75, -8, PIXELS, 16, 40), { steps: 0, carried: -0.5 });
});

test("lines and pages count in steps whatever a row's height", () => {
  assert.deepEqual(wheelTravel(0, 3, LINES, 16, 40), { steps: 3, carried: 0 });
  assert.deepEqual(wheelTravel(0, 1, PAGES, 16, 40), { steps: 40, carried: 0 });
});
