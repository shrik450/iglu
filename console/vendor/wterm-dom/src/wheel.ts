// iglu's own, not upstream (see ../UPSTREAM).

/** Whole steps a wheel event moves, and the travel left over for the next. */
export interface WheelTravel {
  steps: number;
  carried: number;
}

/**
 * Turns a wheel event's delta into whole steps, a row or a column each,
 * carrying the part of a step left over to the next event, as native
 * terminals do. A trackpad sends many small deltas for one swipe; counted one
 * step each, a program tracking the mouse would scroll many times too far.
 * Turning the other way drops what was carried. `mode` is the event's
 * `deltaMode`; `step` is a row or column in pixels, `page` the screen in steps.
 */
export function wheelTravel(
  carried: number,
  delta: number,
  mode: number,
  step: number,
  page: number,
): WheelTravel {
  const travel = mode === 1 ? delta : mode === 2 ? delta * page : delta / step;
  const total =
    Math.sign(travel) === -Math.sign(carried) ? travel : carried + travel;
  // Plus zero: less than a step back is no steps, not -0.
  const steps = Math.trunc(total) + 0;
  return { steps, carried: total - steps };
}
