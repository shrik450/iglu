// Small shared pieces: glyphs, state text, the igloo and frost.

import type { ComponentChildren } from "preact";

import type { AttentionView } from "../generated/AttentionView.ts";
import type { Condition } from "../generated/Condition.ts";
import type { Phase } from "../generated/Phase.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { m } from "../messages.tsx";
import { cursor } from "../state/store.ts";
import { unreachable } from "../state/unsaved.ts";

type GlyphKind = "waiting" | "done" | "working" | "idle" | "frozen" | "asleep" | "build" | "trouble";

/** Line icons drawn on a 16-unit square, so they sit at the centre of their box as text can't. */
const ICON = {
  plus: "M8 3v10M3 8h10",
  chevron: "m6 3.5 4.5 4.5L6 12.5",
  back: "m10 3.5L5.5 8l4.5 4.5",
  close: "m4.5 4.5 7 7m0-7-7 7",
  zoom: "M3 6.5V3h3.5M13 6.5V3H9.5M3 9.5V13h3.5M13 9.5V13H9.5",
  unzoom: "M6.5 3v3.5H3M9.5 3v3.5H13M6.5 13V9.5H3M9.5 13V9.5H13",
  search: "M11.6 7A4.6 4.6 0 1 1 2.4 7a4.6 4.6 0 0 1 9.2 0Zm-1.2 3.4 3.4 3.4",
  up: "m3.5 10 4.5-4.5 4.5 4.5",
  down: "m3.5 6 4.5 4.5L12.5 6",
  bell: "M4 11.5V7.5a4 4 0 0 1 8 0v4l1 1H3Zm2.75 2.25a1.5 1.5 0 0 0 2.5 0",
  snow: "M8 1.8v12.4M2.63 4.9l10.74 6.2M2.63 11.1l10.74-6.2M8 4.1 6.78 2.64M8 4.1l1.22-1.46M4.62 6.05l-1.87.33M4.62 6.05l-.65-1.79M4.62 9.95l-.65 1.79M4.62 9.95l-1.87-.33M8 11.9l1.22 1.46M8 11.9l-1.22 1.46M11.38 9.95l1.87-.33M11.38 9.95l.65 1.79M11.38 6.05l.65-1.79M11.38 6.05l1.87.33",
} as const;

// Each state drawn on a 16-unit square, as the icons are: a font's glyphs
// differ by device and sit off the centre of their box, and a spinner would
// wobble as it turned.
const drawn = (shape: ComponentChildren) => (
  <svg class="i" viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
    {shape}
  </svg>
);

const GLYPH: Record<GlyphKind, ComponentChildren> = {
  waiting: drawn(<circle cx="8" cy="8" r="4.5" fill="currentColor" stroke="none" />),
  done: drawn(<path d="m3.5 8.5 3 3 6-7" />),
  working: drawn(
    <>
      <circle cx="8" cy="8" r="5.75" stroke-opacity=".28" stroke-width="2" />
      <path d="M8 2.25A5.75 5.75 0 0 1 13.75 8" stroke-width="2" />
    </>,
  ),
  idle: drawn(<circle cx="8" cy="8" r="4.25" />),
  frozen: drawn(<path d={ICON.snow} />),
  asleep: drawn(<rect x="4" y="4" width="8" height="8" rx="1.5" fill="currentColor" stroke="none" />),
  build: drawn(<path d="M8 3 13.5 12.5h-11Z" />),
  trouble: drawn(
    <>
      <path d="M8 2.5v7" stroke-width="2" />
      <circle cx="8" cy="13" r="1.25" fill="currentColor" stroke="none" />
    </>,
  ),
};

export function Glyph({ kind }: { kind: GlyphKind }) {
  return (
    <span class={`g g-${kind}`} aria-hidden="true">
      {GLYPH[kind]}
    </span>
  );
}


export function Icon({ name, size = 14 }: { name: keyof typeof ICON; size?: number }) {
  return (
    <svg class={`i i-${name}`} viewBox="0 0 16 16" width={size} height={size} aria-hidden="true">
      <path d={ICON[name]} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  );
}

/** A link to a workspace, which becomes the overview's cursor once opened here. */
export function WorkspaceLink({ ws, class: className, label, children }: { ws: WorkspaceView; class?: string; label?: string; children: ComponentChildren }) {
  return (
    <a
      href={`/w/${ws.name}`}
      class={className}
      aria-label={label}
      onClick={() => (cursor.value = ws.id)}
    >
      {children}
    </a>
  );
}

export function inTrouble(condition: Condition | null): boolean {
  return condition?.kind === "Error" || condition?.kind === "RuntimeFailed";
}

export function attentionGlyph(attention: AttentionView | null): GlyphKind {
  return attention?.state ?? "idle";
}

export function workspaceGlyph(ws: WorkspaceView): GlyphKind {
  if (inTrouble(ws.condition)) return "trouble";
  switch (ws.phase) {
    case "creating":
    case "starting":
      return "build";
    case "freezing":
    case "frozen":
      return "frozen";
    case "stopping":
    case "stopped":
    case "deleting":
    case "deleted":
      return "asleep";
    case "running":
      return attentionGlyph(ws.attention);
    default:
      return unreachable(ws.phase);
  }
}

/** How a workspace in each phase looks: what the tile and lists show. */
export type Look = "awake" | "building" | "frozen" | "asleep";

export function lookOf(phase: Phase): Look {
  switch (phase) {
    case "running":
      return "awake";
    case "creating":
    case "starting":
      return "building";
    case "freezing":
    case "frozen":
      return "frozen";
    case "stopping":
    case "stopped":
    case "deleting":
    case "deleted":
      return "asleep";
    default:
      return unreachable(phase);
  }
}

/** The phase, when it's worth saying: running goes without saying. */
export function phaseText(phase: Phase): string | null {
  return phase === "running" ? null : m.phase(phase);
}

export function attentionText(attention: AttentionView): { tone: string; text: string } {
  switch (attention.state) {
    case "waiting":
      return { tone: "s-waiting", text: m.attention.waiting };
    case "done":
      return { tone: "s-done", text: m.attention.done(attention.seen === "Unseen") };
    case "working":
      return { tone: "s-working", text: m.attention.working };
    case "idle":
      return { tone: "", text: m.attention.idle };
    default:
      return unreachable(attention.state);
  }
}

/** Rows of the igloo, built bottom up: 0–4 rows, then the door. */
export function Igloo({ rows = 5, class: className = "" }: { rows?: number; class?: string }) {
  const cx = 60;
  const ground = 66;
  const radius = 52;
  const band = 13;
  const bands = [0, 1, 2, 3].map((r) => {
    const bottom = ground - band * r;
    const half = Math.sqrt(radius * radius - (ground - bottom) ** 2);
    const bricks: number[] = [];
    for (let x = cx - half - 16 + (r % 2 ? 8 : 0); x < cx + half; x += 16) bricks.push(x);
    return { r, top: bottom - band, bricks };
  });
  const clip = `dome-${rows}`;
  return (
    <svg class={`igloo ${className}`} viewBox="0 0 120 74" aria-hidden="true">
      <defs>
        <clipPath id={clip}>
          <path d={`M${cx - radius} ${ground} A${radius} ${radius} 0 0 1 ${cx + radius} ${ground} Z`} />
        </clipPath>
      </defs>
      <ellipse class="snow" cx="60" cy="68" rx="58" ry="5" />
      <g clip-path={`url(#${clip})`}>
        {bands.map(({ r, top, bricks }) => (
          <g key={r} class={`row${r < rows ? " on" : ""}`}>
            {bricks.map((x) => (
              <rect key={x} x={x.toFixed(1)} y={top + 1} width="15" height={band - 2} rx="2" />
            ))}
          </g>
        ))}
      </g>
      <path class={`door row${rows > 4 ? " on" : ""}`} d={`M50 ${ground} V55 a10 10 0 0 1 20 0 V${ground} Z`} />
    </svg>
  );
}

/** How far along a workspace that isn't up yet is, in igloo rows. */
export function buildRows(phase: Phase): number {
  return phase === "creating" ? 2 : phase === "starting" ? 4 : 5;
}

export function Frost({ label, hint }: { label: string; hint?: string | undefined }) {
  return (
    <div class="frost">
      <div class="frost-label">
        <span class="flake" aria-hidden="true">
          <Icon name="snow" size={20} />
        </span>
        <b>{label}</b>
        {hint ? <small>{hint}</small> : null}
      </div>
    </div>
  );
}

/** A frost texture, drawn once and shared through a CSS variable. */
export function paintFrost(): void {
  const canvas = document.createElement("canvas");
  canvas.width = 320;
  canvas.height = 220;
  const g = canvas.getContext("2d");
  if (!g) return;
  g.strokeStyle = "rgba(255,255,255,0.55)";
  g.lineWidth = 0.7;
  const branch = (x: number, y: number, angle: number, length: number, depth: number): void => {
    if (depth > 3 || length < 3) return;
    const x2 = x + Math.cos(angle) * length;
    const y2 = y + Math.sin(angle) * length;
    g.beginPath();
    g.moveTo(x, y);
    g.lineTo(x2, y2);
    g.stroke();
    for (const t of [1 / 3, 2 / 3]) {
      branch(x + (x2 - x) * t, y + (y2 - y) * t, angle + 0.95, length * 0.42, depth + 1);
      branch(x + (x2 - x) * t, y + (y2 - y) * t, angle - 0.95, length * 0.42, depth + 1);
    }
  };
  for (let i = 0; i < 38; i++) {
    const edge = Math.floor(Math.random() * 4);
    const [x, y, angle] =
      edge === 0 ? [Math.random() * 320, 0, Math.PI / 2] : edge === 1 ? [320, Math.random() * 220, Math.PI] : edge === 2 ? [Math.random() * 320, 220, -Math.PI / 2] : [0, Math.random() * 220, 0];
    branch(x, y, angle + (Math.random() - 0.5) * 1.2, 30 + Math.random() * 55, 0);
  }
  document.documentElement.style.setProperty("--frost-tex", `url(${canvas.toDataURL()})`);
}
