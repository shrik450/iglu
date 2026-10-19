// The console's terminal on a page of its own, for the terminal tier
// (run.py). Each pane is the console's own TerminalPane, mounted as the
// workspace view mounts it, with the console's keys, styles and fonts; what
// its session would send arrives over the pane's WebSocket from the test,
// which answers in the session's place. No iglud, workspace or shell.
//
// It also checks wterm's renderer against Ghostty's terminal core, which
// holds the screen the renderer draws: for any output, what's drawn is what
// the core holds.

import { effect } from "@preact/signals";
import { WTerm } from "@wterm/dom";
import { GhosttyCore } from "@wterm/ghostty";
import WASM from "@wterm/ghostty/ghostty-vt.wasm";
import fc from "fast-check";

import type { Keyboard } from "../../src/generated/Keyboard.ts";
import { terminalKey } from "../../src/keyboard.ts";
import { preferences, shownLook, termStyle } from "../../src/preferences.ts";
import { newCore, rang, restyleTerminals, TerminalPane, titles } from "../../src/terminal.ts";
import { applyTermStyle } from "../../src/termstyle.ts";
import { streams } from "./streams.ts";

// As main.tsx has it.
effect(() => void applyTermStyle(termStyle.value, shownLook.value).then(restyleTerminals));

/** Every pane's workspace: what iglud would call it. */
const WORKSPACE = "harness";
const opened = new Map<string, TerminalPane>();

/** Opens a column for `session`, whose WebSocket the test then answers. */
async function open(session: string): Promise<void> {
  const column = document.createElement("section");
  column.className = "col";
  column.dataset["column"] = session;
  const host = document.createElement("div");
  host.className = "term-host";
  column.append(host);
  document.querySelector("main")?.append(column);
  const pane = new TerminalPane({
    container: host,
    core: await newCore(),
    workspace: WORKSPACE,
    session,
    shortcuts: terminalKey,
    wantsFocus: () => true,
    onFocus: () => {},
    onStatus: () => {},
    onDrop: () => {},
  });
  pane.describe(session, "Ctrl+Space");
  opened.set(session, pane);
}

function pane(session: string): TerminalPane {
  const found = opened.get(session);
  if (!found) throw new Error(`no pane for ${session}`);
  return found;
}

/** The keyboard preferences, as a snapshot from iglud would set them. */
function keyboard(changes: Partial<Keyboard>): void {
  const now = preferences.peek();
  preferences.value = { ...now, keyboard: { ...now.keyboard, ...changes } };
}

/** Resolves once anything written has been drawn. */
function drawn(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
}

/** The core's screen as text, a line per row, as TerminalPane.screen reads it. */
function held(core: GhosttyCore, cols: number, rows: number): string[] {
  const lines: string[] = [];
  for (let row = 0; row < rows; row++) {
    let line = "";
    for (let col = 0; col < cols; col++) {
      const cell = core.getCell(row, col);
      if (cell.width === 0) continue;
      line += cell.chars ?? String.fromCodePoint(cell.char || 0x20);
    }
    lines.push(line.trimEnd());
  }
  return lines;
}

/** The screen's rows as drawn, below any history. */
function shown(element: HTMLElement): string[] {
  return [...element.querySelectorAll(".term-row:not(.term-scrollback-row)")].map((row) => (row.textContent ?? "").replaceAll(" ", " ").trimEnd());
}

interface Mismatch {
  row: number;
  held: string;
  shown: string;
}

/** A terminal of its own, outside any pane, to draw output on. */
async function canvas(cols: number, rows: number): Promise<{ term: WTerm; core: GhosttyCore; element: HTMLElement; done: () => void }> {
  const column = document.createElement("section");
  column.className = "col";
  const host = document.createElement("div");
  host.className = "term-host";
  column.append(host);
  document.querySelector("main")?.append(column);
  const element = document.createElement("div");
  host.append(element);
  const core = await GhosttyCore.load({ wasmPath: WASM, scrollbackLimit: 1_000_000 });
  const term = new WTerm(element, { core, cols, rows, autoResize: false, tabExit: false });
  await term.init();
  return {
    term,
    core,
    element,
    done: () => {
      term.destroy();
      core.dispose();
      column.remove();
    },
  };
}

/** Writes `chunks` one after another, letting the terminal draw between
 * some, and returns how the drawn screen differs from the core's. */
async function compare(chunks: readonly Uint8Array[], cols: number, rows: number, pauses: readonly boolean[] = []): Promise<Mismatch[]> {
  const { term, core, element, done } = await canvas(cols, rows);
  try {
    for (const [index, chunk] of chunks.entries()) {
      term.write(chunk);
      if (pauses[index]) await drawn();
    }
    await drawn();
    const want = held(core, term.cols, term.rows);
    const got = shown(element);
    return want.flatMap((line, row) => (line === got[row] ? [] : [{ row, held: line, shown: got[row] ?? "<no row>" }]));
  } finally {
    done();
  }
}

/** Checks generated output, drawn in random chunks, against the core. */
async function differential(runs: number, seed: number): Promise<{ failed: boolean; seed: number; counterexample: string | null; mismatches: Mismatch[] }> {
  const result = await fc.check(
    fc.asyncProperty(streams(), async ({ chunks, pauses, cols, rows }) => (await compare(chunks, cols, rows, pauses)).length === 0),
    { numRuns: runs, seed },
  );
  const smallest = result.counterexample?.[0];
  return {
    failed: result.failed,
    seed: result.seed,
    // Each chunk's bytes as one character each, which JSON escapes when they aren't text.
    counterexample: smallest ? JSON.stringify(smallest, (_, value: unknown) => (value instanceof Uint8Array ? String.fromCharCode(...value) : value)) : null,
    mismatches: smallest ? await compare(smallest.chunks, smallest.cols, smallest.rows, smallest.pauses) : [],
  };
}

/** Checks a program's recorded output, in its chunks, against the core. */
function recorded(chunks: readonly string[], cols: number, rows: number): Promise<Mismatch[]> {
  const bytes = chunks.map((chunk) => Uint8Array.from(atob(chunk), (c) => c.charCodeAt(0)));
  return compare(bytes, cols, rows, bytes.map(() => true));
}

Object.assign(globalThis, {
  harness: {
    open,
    screen: (session: string) => pane(session).screen(),
    focus: (session: string) => pane(session).focus(),
    find: (session: string, query: string) => pane(session).find(query),
    title: (session: string) => titles.peek().get(`${WORKSPACE}/${session}`) ?? "",
    rang: (session: string) => rang.peek().has(`${WORKSPACE}/${session}`),
    keyboard,
    differential,
    recorded,
  },
});
