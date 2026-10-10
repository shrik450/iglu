// One workspace: the tree beside it, its header, and its strip of columns.

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";

import {
  activeOf,
  addColumn,
  attempt,
  closeColumn,
  columnsOf,
  cycleWidth,
  focusColumn,
  labelColumn,
  labelOrSay,
  loadColumns,
  loadLive,
  markActive,
  moveColumn,
  openColumn,
  placeColumn,
  publish,
  remove,
  rename,
  restartColumn,
  saveOpening,
  setState,
  toggleFreeze,
  toggleZoom,
  unpublish,
  zoomedIn,
} from "../actions.ts";
import { api } from "../api/client.ts";
import { FieldError, type Form, FormError, InputError, invalid, textOf, useForm, useGrab } from "../components/forms.tsx";
import { CopyLink, ONLY_YOU } from "../components/CopyLink.tsx";
import { Previews } from "../components/Previews.tsx";
import {
  attentionGlyph,
  attentionText,
  buildRows,
  Frost,
  Glyph,
  Icon,
  Igloo,
  phaseText,
  workspaceGlyph,
} from "../components/bits.tsx";
import type { ActivityEntry } from "../generated/ActivityEntry.ts";
import type { AttentionView } from "../generated/AttentionView.ts";
import type { ColumnKind } from "../generated/ColumnKind.ts";
import type { ColumnState } from "../generated/ColumnState.ts";
import type { RouteView } from "../generated/RouteView.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { keysFor, mac, terminalKey, touchOnly } from "../keyboard.ts";
import { keyboard } from "../preferences.ts";
import { chordLabel } from "../state/keys.ts";
import { FRACTION, inView, LABEL, scrollTarget, type Shown, titleOf } from "../state/layout.ts";
import { bySession } from "../state/threads.ts";
import { unreachable, unsavedText } from "../state/unsaved.ts";
import { situation } from "../state/situation.ts";
import { ask, collapsed, details, groups, inside, isAsking, navigate, projects, question, route, settle } from "../state/store.ts";
import { ctrlHeld, newCore, panes, TerminalPane } from "../terminal.ts";

export function Workspace({ ws }: { ws: WorkspaceView | null }) {
  const r = route.value;
  return (
    <div class={`wsv${ws && zoomedIn(ws) ? " zoomed" : ""}`} data-phase={ws?.phase}>
      <Tree current={ws} />
      {ws ? (
        <Main ws={ws} />
      ) : (
        <div class="empty-state">
          <p class="empty">No workspace called {r.view === "workspace" ? r.name : "that"}.</p>
          <button type="button" class="btn" onClick={() => navigate({ view: "overview" })}>
            Overview
          </button>
        </div>
      )}
    </div>
  );
}

function Tree({ current }: { current: WorkspaceView | null }) {
  const toggle = (key: string) => {
    const next = new Set(collapsed.value);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    collapsed.value = next;
  };
  // Finding a workspace by name is the palette's job, so the list is only a list.
  return (
    <nav class="side" aria-label="Workspaces">
      <div class="tree">
        {groups.value
          .filter((group) => group.workspaces.length > 0)
          .map((group) => {
            const closed = collapsed.value.has(group.key);
            const need = group.workspaces.filter((ws) => ws.needs_you).length;
            return (
              <div key={group.key} role="group" aria-label={group.label}>
                <div class="t-group">
                  <button type="button" class="chev" aria-expanded={!closed} aria-label={`${closed ? "Expand" : "Collapse"} ${group.label}`} onClick={() => toggle(group.key)}>
                    <Icon name="chevron" size={12} />
                  </button>
                  <span class="label" translate={false}>
                    {group.label}
                  </span>
                  <span class={`cnt${need ? " hot" : ""}`}>{need ? `● ${need}` : group.workspaces.length}</span>
                </div>
                {/* A folded project still shows the open workspace, and a phone, with no
                    projects to unfold, shows them all. */}
                {group.workspaces.map((ws) => (
                  <a
                    key={ws.id}
                    href={`/w/${ws.name}`}
                    class={`t-ws${ws.needs_you ? " needs" : ""}${ws.phase === "stopped" || ws.phase === "frozen" ? " asleep" : ""}${closed && ws.id !== current?.id ? " folded" : ""}`}
                    aria-current={ws.id === current?.id ? "page" : undefined}
                    // In the one-row list on a phone, the current workspace may be off to the side.
                    ref={(el) => {
                      if (ws.id === current?.id) el?.scrollIntoView({ inline: "nearest", block: "nearest" });
                    }}
                    onClick={(e) => {
                      if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
                      e.preventDefault();
                      navigate({ view: "workspace", name: ws.name });
                    }}
                  >
                    <Glyph kind={workspaceGlyph(ws)} />
                    <span class="n" translate={false}>
                      {ws.name}
                    </span>
                  </a>
                ))}
              </div>
            );
          })}
      </div>
    </nav>
  );
}

function Main({ ws }: { ws: WorkspaceView }) {
  const running = ws.phase === "running";
  // Columns added elsewhere need their sessions' states too.
  const asked = ws.columns.map((c) => c.name).join(" ");
  useEffect(() => {
    if (running) void loadColumns(ws);
  }, [ws.id, running, asked]);
  useEffect(() => {
    if (!running) return;
    void loadLive(ws);
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void loadLive(ws);
    }, 5000);
    return () => window.clearInterval(timer);
  }, [ws.id, running]);
  useEffect(() => {
    if (ws.attention?.seen === "unseen" && document.visibilityState === "visible") void api.seen(ws.id).catch(() => undefined);
  }, [ws.id, ws.attention?.updated_at]);
  return (
    <div class="main">
      <Header ws={ws} />
      {running ? <Strip ws={ws} /> : <div />}
      <div class="w-body">
        {running ? <Columns ws={ws} /> : <Resting ws={ws} />}
        {details.value ? <Details ws={ws} /> : null}
      </div>
      {running && touchOnly ? <KeyRow ws={ws} /> : null}
    </div>
  );
}

/** Keys a phone's keyboard lacks, for the column with the keyboard. A tap
 * doesn't take focus, so the on-screen keyboard stays up. Ctrl applies to the
 * next key, tapped or typed. Keys go as the program asks for them, so arrows
 * suit a shell and a full-screen program alike; ^C is always an interrupt. */
const ROW: readonly ({ label: string; name: string } & ({ key: string } | { text: string }))[] = [
  { label: "esc", name: "Escape", key: "Escape" },
  { label: "tab", name: "Tab", key: "Tab" },
  { label: "←", name: "Left", key: "ArrowLeft" },
  { label: "↓", name: "Down", key: "ArrowDown" },
  { label: "↑", name: "Up", key: "ArrowUp" },
  { label: "→", name: "Right", key: "ArrowRight" },
  { label: "^C", name: "Interrupt (Ctrl+C)", text: "\x03" },
];

function KeyRow({ ws }: { ws: WorkspaceView }) {
  const pane = () => panes.get(`${ws.id}/${activeOf(ws) ?? ""}`);
  return (
    <div class="keyrow" role="toolbar" aria-label="Terminal keys" onPointerDown={(e) => e.preventDefault()}>
      <button type="button" aria-pressed={ctrlHeld.value} aria-label="Ctrl, for the next key" onClick={() => (ctrlHeld.value = !ctrlHeld.value)}>
        ctrl
      </button>
      {ROW.map((key) => (
        <button key={key.name} type="button" aria-label={key.name} onClick={() => ("key" in key ? pane()?.press(key.key) : pane()?.type(key.text))}>
          {key.label}
        </button>
      ))}
    </div>
  );
}

function Header({ ws }: { ws: WorkspaceView }) {
  const phase = phaseText(ws.phase);
  const naming = useForm(async (data) => {
    const name = data.get("name");
    if (typeof name !== "string" || name.trim() === ws.name) {
      settle(ws, "rename");
      return;
    }
    await rename(ws, name.trim());
  });
  const publishing = useForm(async (data) => {
    const text = String(data.get("port") ?? "").trim();
    const port = Number(text);
    if (!/^\d+$/.test(text)) throw new InputError("port", `${text || "That"} isn't a port number.`);
    await publish(ws, port);
  });
  return (
    <>
      <header class="w-head">
        {isAsking("rename") ? (
          <form class="rename" onSubmit={naming.onSubmit} onKeyDown={(e) => e.key === "Escape" && settle(ws)}>
            <RenameInput ws={ws} form={naming} />
            <button type="submit" class="btn">
              Rename
            </button>
            <button type="button" class="btn" onClick={() => settle(ws)}>
              Cancel
            </button>
            <FieldError form={naming} input="name" />
            <FormError form={naming} />
          </form>
        ) : (
          <div class="w-title">
            <Glyph kind={workspaceGlyph(ws)} />
            <button type="button" class="nm" title={`Rename (${keysFor("rename")})`} translate={false} onClick={() => ask(ws, { kind: "rename" })}>
              <h1>{ws.name}</h1>
            </button>
            {ws.checkout ? <Branch ws={ws} /> : null}
            {phase ? <span class="state">{phase}</span> : null}
          </div>
        )}
        <div class="w-ports">
          <Previews ws={ws} />
          {ws.phase === "running" ? <Listening ws={ws} /> : null}
          {ws.phase === "running" ? (
            isAsking("port") ? (
              <form class="addport" onSubmit={publishing.onSubmit} onKeyDown={(e) => e.key === "Escape" && settle(ws)}>
                <PortInput form={publishing} />
                <button type="submit" class="btn" disabled={publishing.busy}>
                  Publish
                </button>
                <FieldError form={publishing} input="port" />
                <FormError form={publishing} />
              </form>
            ) : (
              <button type="button" class="chip" aria-label="Publish a port" onClick={() => ask(ws, { kind: "port" })}>
                <Icon name="plus" size={11} />
                port
              </button>
            )
          ) : null}
        </div>
        <div class="w-actions">
          {ws.phase === "running" ? (
            <button type="button" class="btn icon" aria-label="Freeze" title={`Freeze (${keysFor("freeze")})`} onClick={() => void toggleFreeze(ws)}>
              ❄
            </button>
          ) : ws.phase === "frozen" ? (
            <button type="button" class="btn" title={`Thaw (${keysFor("freeze")})`} onClick={() => void toggleFreeze(ws)}>
              Thaw
            </button>
          ) : ws.phase === "stopped" ? (
            <button type="button" class="btn primary" onClick={() => void setState(ws, "running")}>
              Start
            </button>
          ) : null}
          <button type="button" class="btn icon" aria-label="Details" title={`Details (${keysFor("details")})`} aria-pressed={details.value} onClick={() => (details.value = !details.value)}>
            ⋯
          </button>
        </div>
      </header>
      <Trouble ws={ws} />
    </>
  );
}

/** Trouble in a running workspace, above its columns; any other shows it in their place. */
function Trouble({ ws }: { ws: WorkspaceView }) {
  if (ws.phase !== "running") return null;
  const now = situation(ws);
  switch (now.kind) {
    case "stuck":
    case "broken":
    case "held":
      return (
        <p class={`condition${now.kind === "held" ? " calm" : ""}`} translate={false}>
          <b>{now.title}.</b> {now.detail}
        </p>
      );
    case "building":
    case "frozen":
    case "stopped":
    case "leaving":
    case "running":
      return null;
    default:
      return unreachable(now);
  }
}

/** The branch as Git sees it now, with what's ahead, behind or changed. */
function Branch({ ws }: { ws: WorkspaceView }) {
  const git = inside.value[ws.id]?.git;
  const branch = git?.branch ?? ws.checkout?.branch ?? "";
  const marks = git
    ? [
        git.ahead ? `↑${git.ahead}` : "",
        git.behind ? `↓${git.behind}` : "",
        git.changed + git.untracked ? `${git.changed + git.untracked} changed` : "",
        git.conflicted ? `${git.conflicted} conflicted` : "",
      ].filter(Boolean)
    : [];
  // A branch named after the workspace goes without saying.
  const named = branch && branch !== ws.name;
  if (!named && !marks.length) return null;
  return (
    <span class="sub" title={branch ? `On ${branch}` : undefined}>
      <span translate={false}>⎇{named ? ` ${branch}` : ""}</span>
      {marks.length ? <span class="git"> {marks.join(" · ")}</span> : null}
    </span>
  );
}

/** Ports something is listening on without a preview yet: one click publishes. */
function Listening({ ws }: { ws: WorkspaceView }) {
  const unpublished = (inside.value[ws.id]?.listeners ?? []).filter((l) => l.route === null);
  return (
    <>
      {unpublished.map((l) =>
        l.reachable ? (
          <button
            key={l.port}
            type="button"
            class="chip listening"
            aria-label={`Publish :${l.port}${l.process ? ` (${l.process})` : ""}`}
            title={`${l.process || "Something"} is listening${l.column ? ` in ${l.column}` : ""}. Publish it.`}
            onClick={() => void attempt(() => publish(ws, l.port))}
          >
            + :{l.port} <span translate={false}>{l.process}</span>
          </button>
        ) : (
          <span key={l.port} class="chip listening off" title={`${l.process || "Something"} listens on :${l.port} only where previews can't reach. Bind 0.0.0.0 or 127.0.0.1 to publish it.`}>
            :{l.port} <span translate={false}>{l.process}</span>
          </span>
        ),
      )}
    </>
  );
}

function Strip({ ws }: { ws: WorkspaceView }) {
  const columns = columnsOf(ws);
  const active = activeOf(ws);
  const threads = bySession(ws.threads);
  // A chip dragged onto another's left or right half goes before or after it.
  // The keys and the palette move columns too, for those who don't drag.
  const [dragging, setDragging] = useState<string | null>(null);
  const [drop, setDrop] = useState<{ name: string; after: boolean } | null>(null);
  const done = () => {
    setDragging(null);
    setDrop(null);
  };
  return (
    <div class="w-strip">
      <nav class="minimap" aria-label="Columns">
        {columns.map((c) => (
          <button
            type="button"
            aria-current={c.name === active ? "true" : undefined}
            key={c.name}
            class={`sc${c.state === "ended" ? " ended" : ""}${onScreen.value.has(c.name) ? " vis" : ""}${dragging === c.name ? " dragging" : ""}`}
            data-drop={drop?.name === c.name ? (drop.after ? "after" : "before") : undefined}
            draggable={arrangeable(c.state)}
            onDragStart={(e) => {
              e.dataTransfer?.setData("text/plain", titleOf(c));
              if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
              setDragging(c.name);
            }}
            onDragOver={(e) => {
              if (!dragging || dragging === c.name || !arrangeable(c.state)) return;
              e.preventDefault();
              const box = e.currentTarget.getBoundingClientRect();
              const after = e.clientX > box.left + box.width / 2;
              if (drop?.name !== c.name || drop.after !== after) setDrop({ name: c.name, after });
            }}
            onDragLeave={() => drop?.name === c.name && setDrop(null)}
            onDrop={(e) => {
              e.preventDefault();
              if (dragging && drop) void placeColumn(ws, dragging, drop.name, drop.after);
              done();
            }}
            onDragEnd={done}
            onClick={() => focusColumn(ws, c.name)}
          >
            <Glyph kind={attentionGlyph(threads.get(c.name)?.[0] ?? null)} />
            <span translate={false} title={c.label ? `${c.label} (session ${c.name})` : undefined}>
              {titleOf(c)}
            </span>
          </button>
        ))}
      </nav>
      <div class="sc-adder">
        <button
          type="button"
          class="sc-add"
          aria-label="Add a column"
          aria-expanded={isAsking("add-column")}
          title={`Add a column (${keysFor("add-column")})`}
          onClick={() => (isAsking("add-column") ? settle(ws) : ask(ws, { kind: "add-column" }))}
        >
          <Icon name="plus" size={13} />
        </button>
        {question.value?.kind === "add-column" ? <AddMenu ws={ws} server={question.value.server === true} /> : null}
      </div>
    </div>
  );
}

function AddMenu({ ws, server }: { ws: WorkspaceView; server: boolean }) {
  const [command, setCommand] = useState(server);
  // What's opening: the menu stays, says so, and takes no second click until
  // the column is there (which puts the menu away) or iglu refuses it.
  const [opening, setOpening] = useState<string | null>(null);
  const add = (label: string, kind: ColumnKind) => {
    setOpening(label);
    void addColumn(ws, kind).finally(() => setOpening(null));
  };
  const first = useGrab<HTMLButtonElement>();
  useEffect(() => {
    const away = (e: PointerEvent) => {
      if (!(e.target instanceof Element && e.target.closest(".sc-adder"))) settle(ws, "add-column");
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, []);
  const run = useForm(async (data) => {
    const text = data.get("command");
    // The person's own command in their own workspace, so a shell may read it.
    if (typeof text === "string" && text.trim()) await openColumn(ws, { kind: "server", command: ["sh", "-c", text.trim()] });
  });
  if (command) {
    return (
      <form class="add-menu" onSubmit={run.onSubmit} onKeyDown={(e) => e.key === "Escape" && settle(ws)}>
        <CommandInput form={run} />
        <button type="submit" class="btn" disabled={run.busy}>
          Start
        </button>
        <p class="about">It keeps running, and starts again with the workspace. For a one-off command, use a shell.</p>
        <FieldError form={run} input="command" />
        <FormError form={run} />
      </form>
    );
  }
  return (
    <div class="add-menu" role="group" aria-label="Add a column" aria-busy={opening !== null} onKeyDown={(e) => e.key === "Escape" && settle(ws)}>
      <button type="button" ref={first} disabled={opening !== null} onClick={() => add("Shell", { kind: "shell" })}>
        {opening === "Shell" ? "Opening a shell…" : "Shell"}
      </button>
      {ws.agents.map((agent) => (
        <button type="button" key={agent} translate={false} disabled={opening !== null} onClick={() => add(agent, { kind: "agent", agent })}>
          {opening === agent ? `Opening ${agent}…` : agent}
        </button>
      ))}
      <button type="button" disabled={opening !== null} onClick={() => setCommand(true)}>
        Server…
      </button>
    </div>
  );
}

function RenameInput({ ws, form }: { ws: WorkspaceView; form: Form }) {
  const ref = useGrab<HTMLInputElement>(true);
  return <input ref={ref} name="name" aria-label="Workspace name" defaultValue={ws.name} autocomplete="off" spellcheck={false} {...invalid(form, "name")} />;
}

function PortInput({ form }: { form: Form }) {
  const ref = useGrab<HTMLInputElement>();
  return <input ref={ref} name="port" inputMode="numeric" placeholder="3000…" aria-label="Port to publish" autocomplete="off" {...invalid(form, "port")} />;
}

function CommandInput({ form }: { form: Form }) {
  const ref = useGrab<HTMLInputElement>();
  return <input ref={ref} name="command" aria-label="Server command" placeholder="npm run dev…" autocomplete="off" spellcheck={false} {...invalid(form, "command")} />;
}

/** Naming a column: Enter, or leaving the field, keeps the name, as renaming
 * a file or a tab does; an empty one goes back to the session's, and Escape
 * leaves it as it was. Leaving matters on a phone, which has no Enter to see. */
function Naming({ ws, column }: { ws: WorkspaceView; column: Shown }) {
  const ref = useGrab<HTMLInputElement>(true);
  const naming = useForm(async (data) => labelColumn(ws, column.name, textOf(data)("label") ?? ""));
  // Once kept or given up, leaving the field mustn't keep it again.
  const done = useRef(false);
  return (
    <form
      class="col-rename"
      onSubmit={(e) => {
        done.current = true;
        void naming.onSubmit(e).finally(() => (done.current = false));
      }}
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        done.current = true;
        settle(ws);
      }}
    >
      <input
        onBlur={(e) => {
          if (!done.current) e.currentTarget.form?.requestSubmit();
        }}
        ref={ref}
        name="label"
        aria-label={`Name of ${titleOf(column)}`}
        placeholder={column.name}
        defaultValue={titleOf(column)}
        maxLength={40}
        autocomplete="off"
        spellcheck={false}
        disabled={naming.busy}
        {...invalid(naming, "label")}
      />
      <FieldError form={naming} input="label" />
      <FormError form={naming} />
    </form>
  );
}

/** Finding in a column's output, history included. It starts from the newest
 * match, as terminals do: ↩ goes further back and ⇧↩ comes forward. Escape,
 * or closing it, puts the search away and gives the column the keyboard. */
function Finding({ ws, name, title }: { ws: WorkspaceView; name: string; title: string }) {
  const field = useGrab<HTMLInputElement>();
  const pane = panes.get(`${ws.id}/${name}`);
  const found = pane?.found.value ?? null;
  // However it's put away, its highlights go with it.
  useEffect(() => () => pane?.find(""), [pane]);
  const close = () => {
    settle(ws, "find");
    focusColumn(ws, name);
  };
  const again = (back: boolean) => pane?.findAgain(back);
  return (
    <div class="col-find" role="search" aria-label={`Find in ${title}`}>
      <Icon name="search" size={12} />
      <input
        ref={field}
        type="search"
        aria-label={`Find in ${title}`}
        placeholder="Find"
        autocomplete="off"
        spellcheck={false}
        onInput={(e) => pane?.find(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            again(!e.shiftKey);
          } else if (e.key === "Escape") {
            e.preventDefault();
            close();
          }
        }}
      />
      <span class="n" aria-live="polite">
        {foundText(found)}
      </span>
      <button type="button" aria-label="Earlier match" title="Earlier (↩)" disabled={!found?.count} onClick={() => again(true)}>
        <Icon name="up" size={11} />
      </button>
      <button type="button" aria-label="Later match" title="Later (⇧↩)" disabled={!found?.count} onClick={() => again(false)}>
        <Icon name="down" size={11} />
      </button>
      <button type="button" aria-label="Close find" title="Close (esc)" onClick={close}>
        <Icon name="close" size={11} />
      </button>
    </div>
  );
}

/** Where finding has got to: which match of how many, counted from the oldest. */
function foundText(found: { count: number; activeIndex: number; searching: boolean; limited: boolean } | null): string {
  if (!found) return "";
  if (found.count === 0) return found.searching ? "Finding…" : "No matches";
  const of = `${found.count.toLocaleString()}${found.limited ? "+" : ""}`;
  return found.activeIndex === -1 ? of : `${(found.activeIndex + 1).toLocaleString()} of ${of}`;
}

/** Ending a column asks first; the question takes the keyboard, and Escape keeps the column. */
function Ending({ ws, name, title }: { ws: WorkspaceView; name: string; title: string }) {
  const end = useGrab<HTMLButtonElement>();
  return (
    <span class="col-ctl" style={{ opacity: 1 }} onKeyDown={(e) => e.key === "Escape" && settle(ws)}>
      <button type="button" class="end" ref={end} onClick={() => void closeColumn(ws, name)}>
        End {title}
      </button>
      <button type="button" onClick={() => settle(ws)}>
        Keep
      </button>
    </span>
  );
}

/** Which columns are wholly on screen, for the strip's chips. */
const onScreen = signal<ReadonlySet<string>>(new Set());

/** How much of the next column stays in view beside the active one. */
const PEEK = 48;

function Columns({ ws }: { ws: WorkspaceView }) {
  const columns = columnsOf(ws);
  const active = activeOf(ws);
  const strip = useRef<HTMLDivElement>(null);
  const activeNow = useRef(active);
  activeNow.current = active;
  const zoom = zoomedIn(ws);
  const arrangement = `${columns.map((c) => `${c.name}:${c.width}`).join(" ")} ${zoom ?? ""}`;
  // The strip scrolls so the active column is wholly in view, whenever it,
  // the order, the widths or the window change.
  const settle = (smooth: boolean) => {
    const el = strip.current;
    if (!el) return;
    const cols = [...el.querySelectorAll<HTMLElement>("[data-column]")];
    const spans = cols.map((c) => ({ left: c.offsetLeft, width: c.offsetWidth }));
    const index = cols.findIndex((c) => c.dataset.column === activeNow.current);
    const left = scrollTarget(spans, index, el.clientWidth, el.scrollLeft, PEEK);
    if (Math.abs(left - el.scrollLeft) > 1) {
      el.scrollTo({ left, behavior: smooth && !matchMedia("(prefers-reduced-motion: reduce)").matches ? "smooth" : "auto" });
    }
  };
  useEffect(() => {
    const frame = requestAnimationFrame(() => settle(true));
    return () => cancelAnimationFrame(frame);
  }, [active, arrangement]);
  useEffect(() => {
    const el = strip.current;
    if (!el) return;
    const look = () => {
      const cols = [...el.querySelectorAll<HTMLElement>("[data-column]")];
      const shown = inView(cols.map((c) => ({ left: c.offsetLeft, width: c.offsetWidth })), el.clientWidth, el.scrollLeft);
      onScreen.value = new Set(cols.filter((_, i) => shown.has(i)).map((c) => c.dataset.column ?? ""));
    };
    const resized = new ResizeObserver(() => {
      settle(false);
      look();
    });
    resized.observe(el);
    el.addEventListener("scroll", look, { passive: true });
    return () => {
      resized.disconnect();
      el.removeEventListener("scroll", look);
    };
  }, [ws.id]);
  if (columns.length === 0) {
    return (
      <div class="cols-note">
        <button type="button" class="btn" onClick={() => void addColumn(ws, { kind: "shell" })}>
          Open a shell
        </button>
      </div>
    );
  }
  const threads = bySession(ws.threads);
  return (
    <div class="w-cols" ref={strip}>
      {columns.map((c) => (
        <Column key={`${ws.id}/${c.name}`} ws={ws} column={c} on={c.name === active} zoomed={c.name === zoom} threads={threads.get(c.name) ?? []} />
      ))}
    </div>
  );
}

function Column(props: { ws: WorkspaceView; column: Shown; on: boolean; zoomed: boolean; threads: AttentionView[] }) {
  const { ws, column, threads } = props;
  const attention = threads[0] ?? null;
  const [listing, setListing] = useState(false);
  const { name } = column;
  const title = titleOf(column);
  const state = attention ? attentionText(attention) : null;
  const arranged = arrangeable(column.state);
  const [status, setStatus] = useState("");
  const asked = question.value;
  const ending = asked?.kind === "end" && asked.column === name;
  const naming = asked?.kind === "label" && asked.column === name;
  const finding = asked?.kind === "find" && asked.column === name;
  return (
    <section class={`col${props.on ? " on" : ""}`} style={{ "--cw": String(FRACTION[props.zoomed ? "full" : column.width]) }} data-column={name} aria-label={`Column ${title}`}>
      <header
        class={`col-h${attention?.state === "waiting" ? " asks" : ""}`}
        // Whatever is pressed in a column's header acts on that column, so it
        // takes the keyboard first: what's typed next goes to the column shown.
        onMouseDown={(e) => {
          if (e.target instanceof Element && e.target.closest("input")) return;
          e.preventDefault();
          focusColumn(ws, name);
        }}
      >
        <Glyph kind={attentionGlyph(attention)} />
        {naming ? (
          <Naming ws={ws} column={column} />
        ) : (
          <b
            translate={false}
            title={column.state === "adopted" ? undefined : `${column.label ? `Session ${name}. ` : ""}Double-click to rename (${keysFor("label-column")})`}
            onDblClick={() => labelOrSay(ws, name)}
          >
            {title}
          </b>
        )}
        {attention?.summary ? <span class="ct">{attention.summary}</span> : null}
        {state && state.text !== "idle" ? <span class={`state ${state.tone}`}>{state.text}</span> : null}
        {threads.length > 1 ? (
          <button type="button" class="threads-btn" aria-expanded={listing} onClick={() => setListing(!listing)}>
            {threads.length} threads
          </button>
        ) : null}
        {status && column.state !== "ended" ? <span class="status">{status}</span> : null}
        {ending ? (
          <Ending ws={ws} name={name} title={title} />
        ) : (
          <span class="col-ctl">
            {arranged ? (
              <>
                <button
                  type="button"
                  aria-pressed={props.zoomed}
                  aria-label={props.zoomed ? `Unzoom ${title}` : `Zoom ${title}`}
                  title={`${props.zoomed ? "Unzoom: put it back" : "Zoom to fill the page"} (${keysFor("zoom")})`}
                  onClick={() => (focusColumn(ws, name), toggleZoom(ws))}
                >
                  <Icon name={props.zoomed ? "unzoom" : "zoom"} size={11} />
                </button>
                <button type="button" class="wbtn" title={`Width (${keysFor("width")})`} aria-label={`Width of ${title}: ${LABEL[column.width]}`} onClick={() => (focusColumn(ws, name), void cycleWidth(ws, name))}>
                  {LABEL[column.width]}
                </button>
                <button type="button" aria-label={`Move ${title} left`} onClick={() => (focusColumn(ws, name), void moveColumn(ws, -1))}>
                  <Icon name="back" size={11} />
                </button>
                <button type="button" aria-label={`Move ${title} right`} onClick={() => (focusColumn(ws, name), void moveColumn(ws, 1))}>
                  <Icon name="chevron" size={11} />
                </button>
              </>
            ) : null}
            <button type="button" aria-label={`End ${title}`} title={`End this column (${keysFor("close-column")})`} onClick={() => (focusColumn(ws, name), ask(ws, { kind: "end", column: name }))}>
              <Icon name="close" size={11} />
            </button>
          </span>
        )}
      </header>
      {finding ? <Finding ws={ws} name={name} title={title} /> : null}
      {listing && threads.length > 1 ? (
        <ul class="threads" aria-label={`Threads in ${title}`}>
          {threads.map((t) => {
            const text = attentionText(t);
            return (
              <li key={t.thread}>
                <Glyph kind={attentionGlyph(t)} />
                <span class="tt">{t.title || t.summary || "New thread"}</span>
                <span class={`state ${text.tone}`}>{text.text}</span>
              </li>
            );
          })}
        </ul>
      ) : null}
      <ColumnBody ws={ws} column={column} on={props.on} onStatus={setStatus} />
    </section>
  );
}

/** Whether a column is one iglu keeps a layout for; adopted ones aren't. */
function arrangeable(state: ColumnState | null): boolean {
  switch (state) {
    case null:
    case "open":
    case "ended":
      return true;
    case "adopted":
      return false;
    default:
      return unreachable(state);
  }
}

function ColumnBody(props: { ws: WorkspaceView; column: Shown; on: boolean; onStatus: (status: string) => void }): VNode {
  const { ws, column } = props;
  switch (column.state) {
    case null:
      return <div class="col-ended" />;
    case "ended":
      return (
        <div class="col-ended">
          <span>Ended.</span>
          <button type="button" class="btn" onClick={() => void restartColumn(ws, column.name)}>
            Restart
          </button>
        </div>
      );
    case "open":
    case "adopted":
      return <Pane ws={ws} name={column.name} title={titleOf(column)} on={props.on} onStatus={props.onStatus} />;
    default:
      return unreachable(column.state);
  }
}

function Pane(props: { ws: WorkspaceView; name: string; title: string; on: boolean; onStatus: (status: string) => void }) {
  const { ws, name, onStatus } = props;
  const host = useRef<HTMLDivElement>(null);
  const opened = useRef<TerminalPane | null>(null);
  // Opening a terminal is asynchronous; by the time it's done another column may be active.
  const on = useRef(props.on);
  on.current = props.on;
  const prefix = chordLabel(keyboard.value.prefix, mac);
  const description = useRef({ title: props.title, prefix });
  description.current = { title: props.title, prefix };
  useEffect(() => opened.current?.describe(props.title, prefix), [props.title, prefix]);
  useEffect(() => {
    let pane: TerminalPane | null = null;
    let cancelled = false;
    void newCore().then((core) => {
      if (cancelled || !host.current) return core.dispose();
      pane = opened.current = new TerminalPane({
        container: host.current,
        core,
        workspace: ws.id,
        session: name,
        shortcuts: terminalKey,
        wantsFocus: () => on.current,
        onFocus: () => markActive(ws, name),
        onStatus,
        onDrop: () => void loadColumns(ws),
      });
      pane.describe(description.current.title, description.current.prefix);
    });
    return () => {
      cancelled = true;
      opened.current = null;
      pane?.dispose();
    };
  }, [ws.id, name]);
  return <div class="term-host" ref={host} />;
}

function Resting({ ws }: { ws: WorkspaceView }): VNode | null {
  const now = situation(ws);
  switch (now.kind) {
    case "building":
      return (
        <div class="cols-note">
          <div class="building">
            <Igloo rows={buildRows(ws.phase)} />
            <span class="what">{now.label}</span>
          </div>
        </div>
      );
    case "stuck":
    case "broken":
    case "held": {
      const project = projects.value.find((p) => p.id === ws.project);
      const cloning = now.kind === "stuck" && ws.phase === "creating" ? ws.checkout : null;
      return (
        <div class={`cols-note problem ${now.kind === "held" ? "calm" : "trouble"}`} role="status">
          <b>{now.title}</b>
          <p translate={false}>{now.detail}</p>
          {/* A workspace keeps the repository it was made with: fixing the project's helps only the next one. */}
          {cloning && project ? (
            <p>
              It clones <code translate={false}>{cloning.repo}</code>, as {project.name} had it when this workspace was made. If that's wrong, fix the project's
              repository, then delete this workspace and make a new one.
            </p>
          ) : null}
          <div class="acts">
            {cloning && project ? (
              <a class="btn" href={`/p/${project.name}`} onClick={(e) => (e.preventDefault(), navigate({ view: "project", name: project.name }))}>
                Open {project.name}
              </a>
            ) : null}
            {now.kind === "broken" ? (
              <button type="button" class="btn" onClick={() => void setState(ws, "stopped")}>
                Stop
              </button>
            ) : null}
            <button
              type="button"
              class="btn danger"
              onClick={() => {
                details.value = true;
                ask(ws, { kind: "delete" });
              }}
            >
              Delete…
            </button>
          </div>
        </div>
      );
    }
    case "frozen":
      return (
        <div class="cols-note is-frozen">
          <Frost label={now.label} hint={`${keysFor("freeze")} to thaw`} />
        </div>
      );
    case "stopped":
      return (
        <div class="cols-note">
          <span>{now.label}</span>
          <button type="button" class="btn primary" onClick={() => void setState(ws, "running")}>
            Start
          </button>
        </div>
      );
    case "leaving":
      return <div class="cols-note">{now.label}</div>;
    case "running":
      return null;
    default:
      return unreachable(now);
  }
}

/** What the delete confirmation warns about, as precisely as is known. */
function lossText(ws: WorkspaceView): string {
  if (!ws.checkout) return "Its files are lost.";
  if (ws.phase !== "running") return `It's ${ws.phase}, so unpushed work can't be checked. Its files are lost.`;
  const live = inside.value[ws.id];
  if (!live) return "Its files are lost; push your work first.";
  return live.unsaved ? `It has ${unsavedText(live.unsaved)}, which would be lost.` : "Everything is committed and pushed.";
}

function Details({ ws }: { ws: WorkspaceView }) {
  return (
    <aside class="info" aria-label="Details">
      <div>
        <h2>Workspace</h2>
        <dl>
          <dt>Project</dt>
          <dd translate={false}>{projects.value.find((p) => p.id === ws.project)?.name ?? "—"}</dd>
          {ws.checkout ? (
            <>
              <dt>Repository</dt>
              <dd translate={false}>{ws.checkout.repo}</dd>
              <dt>Branch</dt>
              <dd translate={false}>{ws.checkout.branch}</dd>
            </>
          ) : null}
          <dt>Environment</dt>
          <dd translate={false}>{ws.environment}</dd>
          <dt>Memory</dt>
          <dd>{ws.memory === null ? "—" : `${(ws.memory / 1024 ** 3).toFixed(1)} GB${ws.phase === "frozen" ? " in swap" : ""}`}</dd>
          <dt>Created</dt>
          <dd>{new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(ws.created_at)}</dd>
        </dl>
        {ws.columns.length ? (
          <button type="button" class="btn" onClick={() => void saveOpening(ws)}>
            Open new workspaces like this
          </button>
        ) : null}
      </div>
      <History ws={ws} />
      {ws.routes.length ? (
        <div>
          <h2>Previews</h2>
          {ws.routes.map((r) => (
            <RouteRow key={r.id} ws={ws} route={r} />
          ))}
        </div>
      ) : null}
      <div class="w-actions">
        {isAsking("delete") ? (
          <div class="confirm">
            <span>
              Delete {ws.name}? {lossText(ws)}
            </span>
            <button type="button" class="btn danger" onClick={() => void remove(ws)}>
              Delete
            </button>
            <button type="button" class="btn" onClick={() => settle(ws)}>
              Keep it
            </button>
          </div>
        ) : (
          <>
            {ws.phase === "running" || ws.phase === "frozen" ? (
              <button type="button" class="btn" onClick={() => void setState(ws, "stopped")}>
                Stop
              </button>
            ) : null}
            <button type="button" class="btn danger" onClick={() => ask(ws, { kind: "delete" })}>
              Delete
            </button>
          </>
        )}
      </div>
    </aside>
  );
}

const stamp = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });

/** What happened to the workspace, newest first. Loaded when shown, and again when it changes. */
function History({ ws }: { ws: WorkspaceView }) {
  const [entries, setEntries] = useState<ActivityEntry[] | null>(null);
  useEffect(() => {
    let current = true;
    void api.activity(ws.id).then(
      (list) => current && setEntries(list),
      () => current && setEntries([]),
    );
    return () => {
      current = false;
    };
  }, [ws.id, ws.revision, ws.phase, ws.condition?.kind]);
  if (!entries?.length) return null;
  return (
    <div>
      <h2>History</h2>
      <ol class="history">
        {entries.map((entry, i) => (
          <li key={`${entry.at}-${i}`}>
            <time dateTime={new Date(entry.at).toISOString()}>{stamp.format(entry.at)}</time>
            <span class="k">{entry.kind}</span>
            {entry.detail ? <span class="d">{entry.detail}</span> : null}
          </li>
        ))}
      </ol>
    </div>
  );
}

function RouteRow({ ws, route: r }: { ws: WorkspaceView; route: RouteView }) {
  const [sure, setSure] = useState(false);
  return (
    <div class="row">
      <a href={r.url} target="_blank" rel="noopener" translate={false} title={ONLY_YOU}>
        {r.name} :{r.port}
      </a>
      <CopyLink url={r.url} name={r.name} />
      {sure ? (
        <button type="button" class="btn danger" onClick={() => void unpublish(ws, r)}>
          Unpublish for good
        </button>
      ) : (
        <button type="button" class="btn" title="Its name is never reused" onClick={() => setSure(true)}>
          Unpublish
        </button>
      )}
    </div>
  );
}
