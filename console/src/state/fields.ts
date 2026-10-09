// Where a form shows an error about one of its inputs. Pure.

/** The input an error's field is about: its first segment, so `ports[1]`
 * is the `ports` input and `target.kind` the `target` one. */
export function inputOf(field: string): string {
  return field.split(/[.[]/, 1)[0] ?? field;
}
