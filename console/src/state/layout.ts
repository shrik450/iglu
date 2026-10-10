// A workspace's strip of columns: what's shown, in what order, how wide. Pure.

import type { ColumnKind } from "../generated/ColumnKind.ts";
import type { ColumnSpec } from "../generated/ColumnSpec.ts";
import type { ColumnState } from "../generated/ColumnState.ts";
import type { ColumnStatus } from "../generated/ColumnStatus.ts";
import type { ColumnWidth } from "../generated/ColumnWidth.ts";
import type { LayoutEntry } from "../generated/LayoutEntry.ts";
import { unreachable } from "./unsaved.ts";

const WIDTHS: readonly ColumnWidth[] = ["third", "half", "two-thirds", "full"];

export const FRACTION: Record<ColumnWidth, number> = { third: 1 / 3, half: 1 / 2, "two-thirds": 2 / 3, full: 1 };
export const LABEL: Record<ColumnWidth, string> = { third: "⅓", half: "½", "two-thirds": "⅔", full: "1" };

export function nextWidth(width: ColumnWidth): ColumnWidth {
  return WIDTHS[(WIDTHS.indexOf(width) + 1) % WIDTHS.length] ?? "half";
}

/** A column as the strip shows it. */
export interface Shown {
  name: string;
  kind: ColumnKind;
  width: ColumnWidth;
  /** What the person calls it; its session name when null. */
  label: string | null;
  /** Null until the host has said. */
  state: ColumnState | null;
}

/**
 * The asked-for columns in their order, then the sessions nobody asked for,
 * which show as shells. `statuses` is undefined before the host has listed them.
 */
export function shown(asked: readonly ColumnSpec[], statuses: readonly ColumnStatus[] | undefined): Shown[] {
  const state = new Map(statuses?.map((s) => [s.name, s.state]));
  const columns: Shown[] = asked.map((spec) => ({ ...spec, state: statuses ? (state.get(spec.name) ?? "ended") : null }));
  for (const status of statuses ?? []) {
    switch (status.state) {
      case "adopted":
        columns.push({ name: status.name, kind: { kind: "shell" }, width: "half", label: null, state: "adopted" });
        break;
      case "open":
      case "ended":
        break;
      default:
        unreachable(status.state);
    }
  }
  return columns;
}

/** What a column is called where it shows: its label, else its session name. */
export const titleOf = (column: { name: string; label: string | null }): string => column.label ?? column.name;

export function layoutOf(columns: readonly ColumnSpec[]): LayoutEntry[] {
  return columns.map(({ name, width }) => ({ name, width }));
}

/** The columns with `name` moved by `step`, or null when it can't move. */
export function moved(columns: readonly ColumnSpec[], name: string, step: -1 | 1): ColumnSpec[] | null {
  const index = columns.findIndex((c) => c.name === name);
  const target = index + step;
  if (index < 0 || target < 0 || target >= columns.length) return null;
  const next = [...columns];
  const [column] = next.splice(index, 1);
  if (column) next.splice(target, 0, column);
  return next;
}

/** The columns with `name` put just before or after `target`, or null when
 * that changes nothing or either isn't there. */
export function placed(columns: readonly ColumnSpec[], name: string, target: string, after: boolean): ColumnSpec[] | null {
  const column = columns.find((c) => c.name === name);
  if (!column || name === target || !columns.some((c) => c.name === target)) return null;
  const rest = columns.filter((c) => c.name !== name);
  const at = rest.findIndex((c) => c.name === target) + (after ? 1 : 0);
  const next = [...rest.slice(0, at), column, ...rest.slice(at)];
  return next.every((c, i) => c.name === columns[i]?.name) ? null : next;
}

/** The columns with `name` at the next width, or null when there's no such column. */
export function widened(columns: readonly ColumnSpec[], name: string): ColumnSpec[] | null {
  if (!columns.some((c) => c.name === name)) return null;
  return columns.map((c) => (c.name === name ? { ...c, width: nextWidth(c.width) } : c));
}

/** The neighbour to focus after moving by `step`, clamped to the ends. */
export function stepIndex(length: number, index: number, step: -1 | 1): number {
  if (length === 0) return -1;
  return Math.max(0, Math.min(length - 1, index + step));
}

/** A column's place in the strip, in pixels from the strip's start. */
export interface Span {
  left: number;
  width: number;
}

/**
 * Where the strip should scroll so the focused column is wholly in view,
 * moving as little as it can and leaving `peek` pixels of the neighbour on
 * the side it came into view from, so it's plain there's more that way. A
 * column too wide for that starts at the view's left edge.
 */
export function scrollTarget(spans: readonly Span[], focused: number, view: number, scroll: number, peek: number): number {
  const span = spans[focused];
  const end = spans.at(-1);
  if (!span || !end) return scroll;
  const max = Math.max(0, end.left + end.width - view);
  const clamp = (x: number) => Math.min(max, Math.max(0, Math.round(x)));
  if (span.width + 2 * peek > view) return clamp(span.left);
  const before = focused > 0 ? peek : 0;
  const after = focused < spans.length - 1 ? peek : 0;
  if (span.left - before < scroll) return clamp(span.left - before);
  if (span.left + span.width + after > scroll + view) return clamp(span.left + span.width + after - view);
  return clamp(scroll);
}

/** The columns wholly in view. */
export function inView(spans: readonly Span[], view: number, scroll: number): Set<number> {
  const shown = new Set<number>();
  spans.forEach((span, i) => {
    if (span.left >= scroll - 1 && span.left + span.width <= scroll + view + 1) shown.add(i);
  });
  return shown;
}
