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
        columns.push({ name: status.name, kind: { kind: "shell" }, width: "half", state: "adopted" });
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
