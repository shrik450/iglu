// Changed by iglu, not upstream (see ../UPSTREAM):
// - A selection spanning rows no longer mounted reads them through `fill`,
//   rather than leaving the browser to copy only the mounted ones.
// - A selection ending in a row's blank tail leaves the blanks out, as it
//   does when it ends at the row's end.

import type { TerminalRowMetadata } from "@wterm/core";

/** Text offsets are UTF-16 offsets in a rendered row, not terminal columns. */
export interface RenderedRowText {
  text: string;
  specialCells: {
    start: number;
    end: number;
    col: number;
    width: number;
    omit: boolean;
  }[];
  metadata: TerminalRowMetadata | null;
}

export interface SelectedRow {
  row: number;
  element: HTMLElement;
  content: RenderedRowText;
}

/** Expand partial graphemes and remove layout-only wide-glyph spacer heads. */
function selectedText(
  content: RenderedRowText,
  start: number,
  end: number,
): string {
  const parts: string[] = [];
  let cursor = start;
  for (const cell of content.specialCells) {
    if (cell.end <= start) continue;
    if (cell.start >= end) break;
    if (cursor < cell.start) parts.push(content.text.slice(cursor, cell.start));
    if (!cell.omit) parts.push(content.text.slice(cell.start, cell.end));
    cursor = cell.end;
  }
  if (cursor < end) parts.push(content.text.slice(cursor, end));
  return parts.join("");
}

/** A row's text and wrapping, for a row the selection spans but that isn't
 * mounted; null when it can't be read. */
export type RowFill = (
  row: number,
) => { text: string; metadata: TerminalRowMetadata | null } | null;

/**
 * Extract only a selection wholly owned by this terminal. Work against the
 * painted snapshot: core state can already be ahead of the visible frame.
 * Rows the selection spans that aren't mounted come from `fill`. Without it,
 * missing rows, multiple ranges, and selections outside the terminal stay
 * with the browser.
 */
export function getSelectionText(
  terminal: HTMLElement,
  rows: Iterable<SelectedRow>,
  fill?: RowFill,
): string | null {
  return readSelection(terminal, rows, fill)?.text ?? null;
}

export function readSelection(
  terminal: HTMLElement,
  rows: Iterable<SelectedRow>,
  fill?: RowFill,
) {
  const selection = terminal.ownerDocument.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount !== 1)
    return null;
  const range = selection.getRangeAt(0);
  if (
    !terminal.contains(range.startContainer) ||
    !terminal.contains(range.endContainer)
  )
    return null;
  const active = terminal.ownerDocument.activeElement;
  // Input controls have a separate selection that need not clear DOM ranges.
  if (active && (active.tagName === "INPUT" || active.tagName === "TEXTAREA")) {
    const input = active as HTMLInputElement | HTMLTextAreaElement;
    if (input.selectionStart !== input.selectionEnd) return null;
  }

  const parts: string[] = [];
  let first: { row: SelectedRow; offset: number } | undefined;
  let last: { row: SelectedRow; offset: number } | undefined;
  let previous: SelectedRow | undefined;
  for (const current of rows) {
    const { element, content } = current;
    if (!range.intersectsNode(element)) continue;
    const selected = terminal.ownerDocument.createRange();
    selected.selectNodeContents(element);
    if (element.contains(range.startContainer))
      selected.setStart(range.startContainer, range.startOffset);
    if (element.contains(range.endContainer))
      selected.setEnd(range.endContainer, range.endOffset);
    const before = terminal.ownerDocument.createRange();
    before.selectNodeContents(element);
    before.setEnd(selected.startContainer, selected.startOffset);
    const start = before.toString().length;
    const end = start + selected.toString().length;
    // A host modifying terminal-owned text makes the saved offsets invalid.
    if (element.textContent !== content.text) return null;
    first ??= { row: current, offset: start };
    // Only blanks after the end: the row's padding, left out of the copy.
    const tail =
      !content.metadata?.wrapsToNext && !/[^ ]/.test(content.text.slice(end));
    last = {
      row: current,
      offset: tail
        ? Math.max(start, content.text.replace(/ +$/, "").length)
        : end,
    };
    if (previous) {
      // Rows scrolled out of the DOM since the selection began are read from
      // the core, so the copy is whole.
      let before = previous.content.metadata;
      if (current.row !== previous.row + 1) {
        if (!fill) return null;
        for (let row = previous.row + 1; row < current.row; row++) {
          const gap = fill(row);
          if (!gap) return null;
          if (!(before?.wrapsToNext && gap.metadata?.continuesPrevious))
            parts.push("\n");
          parts.push(
            gap.metadata?.wrapsToNext ? gap.text : gap.text.replace(/ +$/, ""),
          );
          before = gap.metadata;
        }
      }
      const wrapped =
        before?.wrapsToNext && content.metadata?.continuesPrevious;
      if (!wrapped) parts.push("\n");
    }
    let text = start === end ? "" : selectedText(content, start, end);
    if (tail) text = text.replace(/ +$/, "");
    parts.push(text);
    previous = current;
  }
  return first && last
    ? {
        text: parts.join(""),
        first,
        last,
        backward:
          selection.anchorNode !== range.startContainer ||
          selection.anchorOffset !== range.startOffset,
      }
    : null;
}

/** Anchor an edge to an included cell, so an end at a wrap follows that cell. */
export function cellAtOffset(
  content: RenderedRowText,
  offset: number,
  end: boolean,
) {
  let delta = 0;
  for (const cell of content.specialCells) {
    if (
      (offset >= cell.start && offset < cell.end && !end) ||
      (offset > cell.start && offset <= cell.end && end)
    ) {
      return { col: cell.col, after: end };
    }
    if (cell.end <= offset) delta += cell.width - (cell.end - cell.start);
  }
  const after = offset > 0 && (end || offset === content.text.length);
  return { col: offset + delta - (after ? 1 : 0), after };
}

export function offsetAtCell(
  content: RenderedRowText,
  col: number,
  after: boolean,
): number {
  let delta = 0;
  for (const cell of content.specialCells) {
    if (col >= cell.col && col < cell.col + cell.width)
      return after ? cell.end : cell.start;
    if (cell.col < col) delta += cell.end - cell.start - cell.width;
  }
  return col + delta + (after ? 1 : 0);
}

export function textPoint(
  element: HTMLElement,
  offset: number,
): [Node, number] | null {
  const walker = element.ownerDocument.createTreeWalker(
    element,
    4 /* SHOW_TEXT */,
  );
  let node: Node | null;
  while ((node = walker.nextNode())) {
    const length = node.textContent?.length ?? 0;
    if (offset <= length) return [node, offset];
    offset -= length;
  }
  return null;
}
