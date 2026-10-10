// Added by iglu, not upstream (see ../UPSTREAM).
//
// Plain URLs in output, found where the pointer is. Programs print most
// links as text rather than marking them with OSC 8, and terminals open
// those too on a modifier-click. Finding them when asked, from the core's
// text, costs nothing while output streams.

import type { TerminalCore, TerminalPosition } from "@wterm/core";
import { rowText } from "./text-capture.js";

/** How far a wrapped line is followed each way from the pointer's row. */
const MOST_ROWS = 64;

const URL_PATTERN = /https?:\/\/[^\s<>"'`]+/g;

/** Trailing characters that end a sentence rather than the URL. */
const TRAILING = /[.,;:!?'"]+$/;

/** Closing brackets end the URL when it didn't open them, as in "(see https://x)". */
function trimBrackets(url: string): string {
  const pairs: Record<string, string> = { ")": "(", "]": "[", "}": "{", ">": "<" };
  let end = url.length;
  for (;;) {
    const text = url.slice(0, end).replace(TRAILING, "");
    const last = text[text.length - 1];
    const open = last ? pairs[last] : undefined;
    if (!open) return text;
    const opened = text.split(open).length - 1;
    const closed = text.split(last!).length - 1;
    if (closed <= opened) return text;
    end = text.length - 1;
  }
}

interface Cell {
  row: number;
  col: number;
}

/** The URL printed under `position`, following the line across wrapped rows. */
export function urlAt(core: TerminalCore, position: TerminalPosition): string | null {
  const total = core.getScrollbackCount() + core.getRows();
  if (position.row < 0 || position.row >= total) return null;
  let first = position.row;
  while (
    first > 0 &&
    position.row - first < MOST_ROWS &&
    rowText(core, first).metadata?.continuesPrevious
  )
    first--;
  let last = position.row;
  while (
    last < total - 1 &&
    last - position.row < MOST_ROWS &&
    rowText(core, last).metadata?.wrapsToNext
  )
    last++;

  // The line's text, and the cell each character came from.
  let text = "";
  const cells: Cell[] = [];
  for (let row = first; row <= last; row++) {
    const history = core.getScrollbackCount();
    const inHistory = row < history;
    const offset = history - row - 1;
    const cols = inHistory ? core.getScrollbackLineLen(offset) : core.getCols();
    for (let col = 0; col < cols; col++) {
      const cell = inHistory
        ? core.getScrollbackCell(offset, col)
        : core.getCell(row - history, col);
      if (cell.width === 0 || cell.spacerHead) continue;
      const chars = cell.chars ?? String.fromCodePoint(cell.char || 32);
      text += chars;
      for (let i = 0; i < chars.length; i++) cells.push({ row, col });
    }
  }

  for (const match of text.matchAll(URL_PATTERN)) {
    const url = trimBrackets(match[0]);
    const start = match.index;
    const end = start + url.length;
    const within = cells.slice(start, end);
    if (
      within.some(
        (cell) => cell.row === position.row && cell.col === position.col,
      )
    )
      return url;
  }
  return null;
}
