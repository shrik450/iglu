// Helpers for closed sets of values. Pure.

/** For closed sets: a new variant makes the call site a type error. */
export function unreachable(value: never): never {
  throw new Error(`unexpected value: ${JSON.stringify(value)}`);
}

/** One of a closed set's keys, from a form's raw string: options are
 * rendered from the same record, so a stale key can't be chosen. */
export function oneOf<K extends string>(labels: Readonly<Record<K, string>>, value: string): K | undefined {
  return (Object.keys(labels) as K[]).find((key) => key === value);
}
