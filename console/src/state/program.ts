// What a program in a terminal says about itself, made fit to show. Pure.

/** The most of a program's title the console shows. */
const LONGEST = 120;

/** A title a program set (OSC 0 or 2), as plain text: no control or
 * direction-changing characters, spaces collapsed, and cut to fit. Programs
 * write anything here, so it's only ever text. */
export function programTitle(raw: string): string {
  const plain = raw
    .replace(/[\u0000-\u001f\u007f-\u009f‎‏‪-‮⁦-⁩]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return plain.length > LONGEST ? `${plain.slice(0, LONGEST - 1)}…` : plain;
}
