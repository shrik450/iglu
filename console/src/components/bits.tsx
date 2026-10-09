// Small shared pieces: glyphs, state text, the igloo and frost.

import type { ComponentChildren } from "preact";

import { open } from "../actions.ts";
import type { AttentionView } from "../generated/AttentionView.ts";
import type { Condition } from "../generated/Condition.ts";
import type { Phase } from "../generated/Phase.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { unreachable } from "../state/unsaved.ts";

type GlyphKind = "waiting" | "done" | "working" | "idle" | "exited" | "frozen" | "asleep" | "build" | "trouble";

const GLYPH: Record<GlyphKind, string> = {
  waiting: "●",
  done: "✓",
  working: "◐",
  idle: "○",
  exited: "·",
  frozen: "❄",
  asleep: "■",
  build: "△",
  trouble: "!",
};

/** A link to a workspace that opens it in place; modified clicks open a tab. */
export function WorkspaceLink({ ws, class: className, label, children }: { ws: WorkspaceView; class?: string; label?: string; children: ComponentChildren }) {
  return (
    <a
      href={`/w/${ws.name}`}
      class={className}
      aria-label={label}
      onClick={(e) => {
        if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
        e.preventDefault();
        open(ws);
      }}
    >
      {children}
    </a>
  );
}

export function Glyph({ kind }: { kind: GlyphKind }) {
  return (
    <span class={`g g-${kind}`} aria-hidden="true">
      {GLYPH[kind]}
    </span>
  );
}

export function inTrouble(condition: Condition | null): boolean {
  return condition?.kind === "error" || condition?.kind === "runtime_failed";
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
  return phase === "running" ? null : phase;
}

export function attentionText(attention: AttentionView): { tone: string; text: string } {
  switch (attention.state) {
    case "waiting":
      return { tone: "s-waiting", text: "needs you" };
    case "done":
      return { tone: "s-done", text: attention.seen === "unseen" ? "done · new" : "done" };
    case "working":
      return { tone: "s-working", text: "working" };
    case "idle":
      return { tone: "", text: "idle" };
    case "exited":
      return { tone: "", text: "exited" };
  }
}

export function conditionText(condition: Condition): string {
  switch (condition.kind) {
    case "error":
      return `${condition.message}; retrying`;
    case "capacity":
      return "waiting for the host to have room";
    case "runtime_failed":
      return "the runtime reports this workspace as broken; stop or delete it";
    case "host_offline":
      return "the host isn't answering";
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

export function Frost({ label, hint }: { label: string; hint?: string }) {
  return (
    <div class="frost">
      <div class="frost-label">
        <span class="flake" aria-hidden="true">
          ❄
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
