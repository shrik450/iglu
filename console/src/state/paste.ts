// Pasting files on a terminal: each is kept in the workspace, and its path
// pasted, as a terminal pastes a dropped file's path.

/** The most one file may be, as iglu-domain's pasted.rs has it. */
export const MAX_FILE_BYTES = 16 * 1024 * 1024;

/** A path as a shell, and an agent reading a pasted path, would take it:
 * as it is when nothing in it needs quoting, quoted otherwise. */
export function pastedPath(path: string): string {
  return /^[\w./-]+$/.test(path) ? path : `'${path.replaceAll("'", `'\\''`)}'`;
}
