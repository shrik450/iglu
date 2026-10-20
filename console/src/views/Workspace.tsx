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
  openInBrowser,
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
  update,
  zoomedIn,
} from "../actions.ts";
import { api, failure } from "../api/client.ts";
import { FieldError, type Form, FormError, InputError, invalid, textOf, useForm, useGrab } from "../components/forms.tsx";
import { Browser } from "../components/Browser.tsx";
import { CopyLink } from "../components/CopyLink.tsx";
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
import type { AccessGrant } from "../generated/AccessGrant.ts";
import type { ActivityEntry } from "../generated/ActivityEntry.ts";
import type { AttentionView } from "../generated/AttentionView.ts";
import type { ColumnKind } from "../generated/ColumnKind.ts";
import type { ColumnState } from "../generated/ColumnState.ts";
import type { LabelColumn } from "../generated/LabelColumn.ts";
import type { LabelWorkspace } from "../generated/LabelWorkspace.ts";
import type { PublishPort } from "../generated/PublishPort.ts";
import type { RenameWorkspace } from "../generated/RenameWorkspace.ts";
import type { RouteView } from "../generated/RouteView.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { keysFor, mac, terminalKey, touchOnly } from "../keyboard.ts";
import { m } from "../messages.tsx";
import { keyboard } from "../preferences.ts";
import { chordLabel } from "../state/keys.ts";
import { FRACTION, inView, LABEL, scrollTarget, type Shown, titleOf } from "../state/layout.ts";
import { bySession } from "../state/threads.ts";
import { unreachable } from "../state/unsaved.ts";
import { situation } from "../state/situation.ts";
import { holds, named, PERMISSIONS, toggled } from "../state/access.ts";
import { ask, collapsed, details, groups, inside, isAsking, navigate, projects, question, route, say, settle, workspaces } from "../state/store.ts";
import { ctrlHeld, newCore, offers, panes, rang, setOffer, TerminalPane, titles } from "../terminal.ts";
import { called } from "../state/naming.ts";

export function Workspace({ ws }: { ws: WorkspaceView | null }) {
  const r = route.value;
  return (
    <div class={`wsv${ws && zoomedIn(ws) ? " zoomed" : ""}`} data-phase={ws?.phase}>
      <Tree current={ws} />
      {ws ? (
        <Main ws={ws} />
      ) : (
        <div class="empty-state">
          <p class="empty">{m.workspace.missing(r.view === "workspace" ? r.name : null)}</p>
          <button type="button" class="btn" onClick={() => navigate({ view: "overview" })}>
            {m.workspace.overview}
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
    <nav class="side" aria-label={m.workspace.list}>
      <div class="tree">
        {groups.value
          .filter((group) => group.workspaces.length > 0)
          .map((group) => {
            const closed = collapsed.value.has(group.key);
            const need = group.workspaces.filter((ws) => ws.needs_you).length;
            return (
              <div key={group.key} role="group" aria-label={group.label}>
                <div class="t-group">
                  <button type="button" class="chev" aria-expanded={!closed} aria-label={closed ? m.overview.expand(group.label) : m.overview.collapse(group.label)} onClick={() => toggle(group.key)}>
                    <Icon name="chevron" size={12} />
                  </button>
                  <span class="label" translate={false}>
                    {group.label}
                  </span>
                  <span class={`cnt${need ? " hot" : ""}`}>
                    {need ? (
                      <>
                        <Glyph kind="waiting" /> {m.overview.count(need)}
                      </>
                    ) : (
                      m.overview.count(group.workspaces.length)
                    )}
                  </span>
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
                  >
                    <Glyph kind={workspaceGlyph(ws)} />
                    <span class="n" translate={false}>
                      {called(ws)}
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
    if (ws.attention?.seen === "Unseen" && document.visibilityState === "visible") void api.seen(ws.id).catch(() => undefined);
  }, [ws.id, ws.attention?.updated_at]);
  return (
    <div class="main">
      <Header ws={ws} />
      {running ? <Strip ws={ws} /> : null}
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
  { label: "esc", name: m.workspace.keys.escape, key: "Escape" },
  { label: "tab", name: m.workspace.keys.tab, key: "Tab" },
  { label: "←", name: m.workspace.keys.left, key: "ArrowLeft" },
  { label: "↓", name: m.workspace.keys.down, key: "ArrowDown" },
  { label: "↑", name: m.workspace.keys.up, key: "ArrowUp" },
  { label: "→", name: m.workspace.keys.right, key: "ArrowRight" },
  { label: "^C", name: m.workspace.keys.interrupt, text: "\x03" },
];

function KeyRow({ ws }: { ws: WorkspaceView }) {
  const pane = () => panes.get(`${ws.id}/${activeOf(ws) ?? ""}`);
  return (
    <div class="keyrow" role="toolbar" aria-label={m.workspace.keyRow} onPointerDown={(e) => e.preventDefault()}>
      <button type="button" aria-pressed={ctrlHeld.value} aria-label={m.workspace.ctrl} onClick={() => (ctrlHeld.value = !ctrlHeld.value)}>
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
  const naming = useForm<RenameWorkspace & LabelWorkspace>(async (data) => {
    const text = textOf(data);
    await rename(ws, text("label") ?? "", text("name") ?? ws.name);
  }, m.workspace.renameRules);
  const publishing = useForm<PublishPort>(async (data) => {
    const text = String(data.get("port") ?? "").trim();
    const port = Number(text);
    if (!/^\d+$/.test(text)) throw new InputError("port", m.workspace.notAPort(text));
    await publish(ws, port);
  }, m.workspace.portRules);
  return (
    <>
      <header class="w-head">
        {isAsking("rename") ? (
          <form class="rename" onSubmit={naming.onSubmit} onKeyDown={(e) => e.key === "Escape" && settle(ws)}>
            <RenameInput ws={ws} form={naming} />
            <input
              name="name"
              class="slug"
              aria-label={m.workspace.name}
              title={m.workspace.name}
              defaultValue={ws.name}
              autocomplete="off"
              spellcheck={false}
              {...invalid(naming, "name")}
            />
            <button type="submit" class="btn">
              {m.workspace.renameSubmit}
            </button>
            <button type="button" class="btn" onClick={() => settle(ws)}>
              {m.workspace.cancel}
            </button>
            <FieldError form={naming} input="label" />
            <FieldError form={naming} input="name" />
            <FormError form={naming} />
          </form>
        ) : (
          <div class="w-title">
            <Glyph kind={workspaceGlyph(ws)} />
            <button type="button" class="nm" title={m.workspace.renameTitle(keysFor("rename"))} translate={false} onClick={() => ask(ws, { kind: "rename" })}>
              <h1>{called(ws)}</h1>
            </button>
            {ws.label ? (
              <span class="slug" translate={false} title={m.workspace.name}>
                {ws.name}
              </span>
            ) : null}
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
                  {m.workspace.publish}
                </button>
                <FieldError form={publishing} input="port" />
                <FormError form={publishing} />
              </form>
            ) : (
              <button type="button" class="chip" aria-label={m.workspace.publishPort} onClick={() => ask(ws, { kind: "port" })}>
                <Icon name="plus" size={11} />
                {m.workspace.port}
              </button>
            )
          ) : null}
        </div>
        <div class="w-actions">
          {ws.phase === "running" ? (
            <button type="button" class="btn icon" aria-label={m.workspace.freeze} title={m.workspace.freezeTitle(keysFor("freeze"))} onClick={() => void toggleFreeze(ws)}>
              <Icon name="snow" />
            </button>
          ) : ws.phase === "frozen" ? (
            <button type="button" class="btn" title={m.workspace.thawTitle(keysFor("freeze"))} onClick={() => void toggleFreeze(ws)}>
              {m.workspace.thaw}
            </button>
          ) : ws.phase === "stopped" ? (
            <button type="button" class="btn primary" onClick={() => void setState(ws, "running")}>
              {m.workspace.start}
            </button>
          ) : null}
          <button type="button" class="btn icon" aria-label={m.workspace.details} title={m.workspace.detailsTitle(keysFor("details"))} aria-pressed={details.value} onClick={() => (details.value = !details.value)}>
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
    case "held": {
      const { title, detail } = m.situation.trouble(now);
      return <p class={`condition${now.kind === "held" ? " calm" : ""}`}>{m.workspace.troubleLine(title, detail)}</p>;
    }
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
        git.ahead ? m.workspace.ahead(git.ahead) : "",
        git.behind ? m.workspace.behind(git.behind) : "",
        git.changed + git.untracked ? m.workspace.changed(git.changed + git.untracked) : "",
        git.conflicted ? m.workspace.conflicted(git.conflicted) : "",
      ].filter(Boolean)
    : [];
  // A branch named after the workspace goes without saying.
  const named = branch && branch !== ws.name;
  if (!named && !marks.length) return null;
  return (
    <span class="sub" title={branch ? m.workspace.branchTitle(branch) : undefined}>
      <span translate={false}>⎇{named ? ` ${branch}` : ""}</span>
      {marks.length ? <span class="git"> {m.workspace.gitMarks(marks)}</span> : null}
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
            aria-label={m.workspace.listening.label(l.port, l.process || null)}
            title={m.workspace.listening.title(l.process || null, l.column || null)}
            onClick={() => void attempt(() => publish(ws, l.port))}
          >
            + :{l.port} <span translate={false}>{l.process}</span>
          </button>
        ) : (
          <span key={l.port} class="chip listening off" title={m.workspace.listening.unreachable(l.process || null, l.port)}>
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
      <nav class="minimap" aria-label={m.workspace.columns}>
        {columns.map((c) => (
          <button
            type="button"
            aria-current={c.name === active ? "true" : undefined}
            key={c.name}
            class={`sc${c.state === "Ended" ? " ended" : ""}${onScreen.value.has(c.name) ? " vis" : ""}${dragging === c.name ? " dragging" : ""}${rang.value.has(`${ws.id}/${c.name}`) ? " rang" : ""}`}
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
            <span translate={false} title={c.label ? m.workspace.sessionTitle(c.label, c.name) : undefined}>
              {titleOf(c)}
            </span>
            {rang.value.has(`${ws.id}/${c.name}`) ? (
              <span class="bell" title={m.workspace.rang}>
                <Icon name="bell" size={12} />
                <span class="sr-only">{m.workspace.rangLabel}</span>
              </span>
            ) : null}
          </button>
        ))}
      </nav>
      <div class="sc-adder">
        <button
          type="button"
          class="sc-add"
          aria-label={m.workspace.addColumn}
          aria-expanded={isAsking("add-column")}
          title={m.workspace.addColumnTitle(keysFor("add-column"))}
          onClick={() => (isAsking("add-column") ? settle(ws) : ask(ws, { kind: "add-column" }))}
        >
          <Icon name="plus" size={13} />
        </button>
        {question.value?.kind === "add-column" ? <AddMenu ws={ws} /> : null}
      </div>
    </div>
  );
}

function AddMenu({ ws }: { ws: WorkspaceView }) {
  // What's opening: the menu stays, says so, and takes no second click until
  // the column is there (which puts the menu away) or iglu refuses it.
  const [opening, setOpening] = useState<ColumnKind["kind"] | null>(null);
  // A workspace has one browser; asking for it again goes to it.
  const browser = columnsOf(ws).find((c) => c.kind.kind === "Browser");
  const add = (kind: ColumnKind) => {
    setOpening(kind.kind);
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
  return (
    <div class="add-menu" role="group" aria-label={m.workspace.addColumn} aria-busy={opening !== null} onKeyDown={(e) => e.key === "Escape" && settle(ws)}>
      <button type="button" ref={first} disabled={opening !== null} onClick={() => add({ kind: "Shell" })}>
        {opening === "Shell" ? m.workspace.openingShell : m.workspace.shell}
      </button>
      {browser ? (
        <button type="button" disabled={opening !== null} onClick={() => (settle(ws, "add-column"), focusColumn(ws, browser.name))}>
          {m.workspace.browser} <span class="about">{m.workspace.browserOpen}</span>
        </button>
      ) : ws.browser ? (
        <button type="button" disabled={opening !== null} onClick={() => add({ kind: "Browser" })}>
          {opening === "Browser" ? m.workspace.openingBrowser : m.workspace.browser}
        </button>
      ) : null}
    </div>
  );
}

function RenameInput({ ws, form }: { ws: WorkspaceView; form: Form<RenameWorkspace & LabelWorkspace> }) {
  const ref = useGrab<HTMLInputElement>(true);
  return <input ref={ref} name="label" aria-label={m.workspace.label} defaultValue={ws.label ?? ""} placeholder={ws.name} autocomplete="off" {...invalid(form, "label")} />;
}

function PortInput({ form }: { form: Form<PublishPort> }) {
  const ref = useGrab<HTMLInputElement>();
  return <input ref={ref} name="port" inputMode="numeric" placeholder="3000…" aria-label={m.workspace.portLabel} autocomplete="off" {...invalid(form, "port")} />;
}

/** Naming a column: Enter, or leaving the field, keeps the name, as renaming
 * a file or a tab does; an empty one goes back to the session's, and Escape
 * leaves it as it was. Leaving matters on a phone, which has no Enter to see. */
function Naming({ ws, column }: { ws: WorkspaceView; column: Shown }) {
  const ref = useGrab<HTMLInputElement>(true);
  const naming = useForm<LabelColumn>(async (data) => labelColumn(ws, column.name, textOf(data)("label") ?? ""), m.workspace.columnRules);
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
        aria-label={m.workspace.columnName(titleOf(column))}
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
  const mounted = panes.get(`${ws.id}/${name}`);
  // Finding is in a terminal's output; a browser has its own.
  const pane = mounted instanceof TerminalPane ? mounted : undefined;
  const found = pane?.found.value ?? null;
  // However it's put away, its highlights go with it.
  useEffect(() => () => pane?.find(""), [pane]);
  const close = () => {
    settle(ws, "find");
    focusColumn(ws, name);
  };
  const again = (back: boolean) => pane?.findAgain(back);
  return (
    <div class="col-find" role="search" aria-label={m.workspace.find.label(title)}>
      <Icon name="search" size={12} />
      <input
        ref={field}
        type="search"
        aria-label={m.workspace.find.label(title)}
        placeholder={m.workspace.find.placeholder}
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
      <button type="button" aria-label={m.workspace.find.earlier} title={m.workspace.find.earlierTitle} disabled={!found?.count} onClick={() => again(true)}>
        <Icon name="up" size={11} />
      </button>
      <button type="button" aria-label={m.workspace.find.later} title={m.workspace.find.laterTitle} disabled={!found?.count} onClick={() => again(false)}>
        <Icon name="down" size={11} />
      </button>
      <button type="button" aria-label={m.workspace.find.close} title={m.workspace.find.closeTitle} onClick={close}>
        <Icon name="close" size={11} />
      </button>
    </div>
  );
}

/** What a program in the column asked to open, offered where it can go:
 * a page opens on the person's click, never on a program's say-so. The
 * person's browser is where they're signed in; the workspace's reaches what
 * listens in the workspace, such as a sign-in's callback. */
function Offering({ ws, name }: { ws: WorkspaceView; name: string }) {
  const key = `${ws.id}/${name}`;
  const asked = offers.value.get(key);
  if (!asked) return null;
  const places = asked.places.filter((place) => place !== "workspace" || ws.browser);
  const done = () => setOffer(key, null);
  return (
    <div class="col-offer" role="group" aria-label={m.workspace.offer.label}>
      <span class="what" translate={false} title={asked.url}>
        {asked.url}
      </span>
      {places.includes("here") ? (
        <a class="btn" href={asked.url} target="_blank" rel="noopener noreferrer" onClick={done}>
          {m.workspace.offer.open}
        </a>
      ) : null}
      {places.includes("workspace") ? (
        <button
          type="button"
          class="btn"
          onClick={() => {
            done();
            void openInBrowser(ws, asked.url);
          }}
        >
          {m.workspace.offer.inWorkspace}
        </button>
      ) : null}
      {places.length === 0 ? <span class="why">{m.workspace.offer.nowhere}</span> : null}
      <button
        type="button"
        class="x"
        aria-label={m.workspace.offer.dismiss}
        title={m.workspace.offer.dismiss}
        onClick={() => {
          done();
          focusColumn(ws, name);
        }}
      >
        <Icon name="close" size={11} />
      </button>
    </div>
  );
}

/** Where finding has got to: which match of how many, counted from the oldest. */
function foundText(found: { count: number; activeIndex: number; searching: boolean; limited: boolean } | null): string {
  if (!found) return "";
  if (found.count === 0) return found.searching ? m.workspace.find.finding : m.workspace.find.none;
  return found.activeIndex === -1 ? m.workspace.find.count(found.count, found.limited) : m.workspace.find.at(found.activeIndex + 1, found.count, found.limited);
}

/** Ending a column asks first; the question takes the keyboard, and Escape keeps the column. */
function Ending({ ws, name, title }: { ws: WorkspaceView; name: string; title: string }) {
  const end = useGrab<HTMLButtonElement>();
  return (
    <span class="col-ctl" style={{ opacity: 1 }} onKeyDown={(e) => e.key === "Escape" && settle(ws)}>
      <button type="button" class="end" ref={end} onClick={() => void closeColumn(ws, name)}>
        {m.workspace.end(title)}
      </button>
      <button type="button" onClick={() => settle(ws)}>
        {m.workspace.keep}
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
        <button type="button" class="btn" onClick={() => void addColumn(ws, { kind: "Shell" })}>
          {m.workspace.openShell}
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
  const program = titles.value.get(`${ws.id}/${name}`);
  const state = attention ? attentionText(attention) : null;
  const arranged = arrangeable(column.state);
  const [status, setStatus] = useState("");
  const asked = question.value;
  const ending = asked?.kind === "end" && asked.column === name;
  const naming = asked?.kind === "label" && asked.column === name;
  const finding = asked?.kind === "find" && asked.column === name;
  return (
    <section class={`col${props.on ? " on" : ""}`} style={{ "--cw": String(FRACTION[props.zoomed ? "full" : column.width]) }} data-column={name} aria-label={m.workspace.column(title)}>
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
            title={column.state === "Adopted" ? undefined : m.workspace.renameColumnTitle(column.label ? name : null, keysFor("label-column"))}
            onDblClick={() => labelOrSay(ws, name)}
          >
            {title}
          </b>
        )}
        {attention?.summary ? (
          <span class="ct">{attention.summary}</span>
        ) : program ? (
          <span class="ct" translate={false} title={m.workspace.programTitle}>
            {program}
          </span>
        ) : null}
        {state && attention?.state !== "idle" ? <span class={`state ${state.tone}`}>{state.text}</span> : null}
        {threads.length > 1 ? (
          <button type="button" class="threads-btn" aria-expanded={listing} onClick={() => setListing(!listing)}>
            {m.workspace.threads(threads.length)}
          </button>
        ) : null}
        {status && column.state !== "Ended" ? <span class="status">{status}</span> : null}
        {ending ? (
          <Ending ws={ws} name={name} title={title} />
        ) : (
          <span class="col-ctl">
            {arranged ? (
              <>
                <button
                  type="button"
                  aria-pressed={props.zoomed}
                  aria-label={props.zoomed ? m.workspace.unzoom(title) : m.workspace.zoom(title)}
                  title={props.zoomed ? m.workspace.unzoomTitle(keysFor("zoom")) : m.workspace.zoomTitle(keysFor("zoom"))}
                  onClick={() => (focusColumn(ws, name), toggleZoom(ws))}
                >
                  <Icon name={props.zoomed ? "unzoom" : "zoom"} size={11} />
                </button>
                <button type="button" class="wbtn" title={m.workspace.widthTitle(keysFor("width"))} aria-label={m.workspace.widthLabel(title, LABEL[column.width])} onClick={() => (focusColumn(ws, name), void cycleWidth(ws, name))}>
                  {LABEL[column.width]}
                </button>
                <button type="button" aria-label={m.workspace.moveLeft(title)} onClick={() => (focusColumn(ws, name), void moveColumn(ws, -1))}>
                  <Icon name="back" size={11} />
                </button>
                <button type="button" aria-label={m.workspace.moveRight(title)} onClick={() => (focusColumn(ws, name), void moveColumn(ws, 1))}>
                  <Icon name="chevron" size={11} />
                </button>
              </>
            ) : null}
            <button type="button" aria-label={m.workspace.end(title)} title={m.workspace.endTitle(keysFor("close-column"))} onClick={() => (focusColumn(ws, name), ask(ws, { kind: "end", column: name }))}>
              <Icon name="close" size={11} />
            </button>
          </span>
        )}
      </header>
      {finding ? <Finding ws={ws} name={name} title={title} /> : null}
      <Offering ws={ws} name={name} />
      {listing && threads.length > 1 ? (
        <ul class="threads" aria-label={m.workspace.threadsIn(title)}>
          {threads.map((t) => {
            const text = attentionText(t);
            return (
              <li key={t.thread}>
                <Glyph kind={attentionGlyph(t)} />
                <span class="tt">{t.title || t.summary || m.workspace.newThread}</span>
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
    case "Open":
    case "Ended":
      return true;
    case "Adopted":
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
    case "Ended":
      return (
        <div class="col-ended">
          <span>{m.workspace.ended}</span>
          <button type="button" class="btn" onClick={() => void restartColumn(ws, column.name)}>
            {m.workspace.restart}
          </button>
        </div>
      );
    case "Open":
    case "Adopted":
      return column.kind.kind === "Browser" ? (
        <Browser ws={ws} name={column.name} title={titleOf(column)} on={props.on} />
      ) : (
        <Pane ws={ws} name={column.name} title={titleOf(column)} on={props.on} onStatus={props.onStatus} />
      );
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
            <span class="what">{m.situation.resting(now)}</span>
          </div>
        </div>
      );
    case "stuck":
    case "broken":
    case "held": {
      const project = projects.value.find((p) => p.id === ws.project);
      const cloning = now.kind === "stuck" && ws.phase === "creating" ? ws.checkout : null;
      const { title, detail } = m.situation.trouble(now);
      return (
        <div class={`cols-note problem ${now.kind === "held" ? "calm" : "trouble"}`} role="status">
          <b>{title}</b>
          <p>{detail}</p>
          {/* A workspace keeps the repository it was made with: fixing the project's helps only the next one. */}
          {cloning && project ? <p>{m.workspace.cloning(<code translate={false}>{cloning.repo}</code>, project.name)}</p> : null}
          <div class="acts">
            {/* A failure of the build it's on, such as one an older iglu made, may be over on the newest. */}
            {ws.newer_build ? (
              <button type="button" class="btn primary" onClick={() => void update(ws)}>
                {m.workspace.updateNewest}
              </button>
            ) : null}
            {cloning && project ? (
              <a class="btn" href={`/p/${project.name}`}>
                {m.workspace.openProject(project.name)}
              </a>
            ) : null}
            {now.kind === "broken" ? (
              <button type="button" class="btn" onClick={() => void setState(ws, "stopped")}>
                {m.workspace.stop}
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
              {m.workspace.deleteAsk}
            </button>
          </div>
        </div>
      );
    }
    case "frozen":
      return (
        <div class="cols-note is-frozen">
          <Frost label={m.situation.resting(now)} hint={m.workspace.thawHint(keysFor("freeze"))} />
        </div>
      );
    case "stopped":
      return (
        <div class="cols-note">
          <span>{m.situation.resting(now)}</span>
          <button type="button" class="btn primary" onClick={() => void setState(ws, "running")}>
            {m.workspace.start}
          </button>
        </div>
      );
    case "leaving":
      return <div class="cols-note">{m.situation.resting(now)}</div>;
    case "running":
      return null;
    default:
      return unreachable(now);
  }
}

/** What the delete confirmation warns about, as precisely as is known. */
function lossText(ws: WorkspaceView): string {
  if (!ws.checkout) return m.workspace.loss.noCheckout;
  if (ws.phase !== "running") return m.workspace.loss.unchecked(m.phase(ws.phase));
  const live = inside.value[ws.id];
  if (!live) return m.workspace.loss.unknown;
  if (live.unsaved) return m.workspace.loss.unsaved(m.unsaved(live.unsaved));
  // Its checkout should be there; without it, nothing says what's pushed.
  return live.git ? m.workspace.loss.none : m.workspace.loss.unknown;
}

function Details({ ws }: { ws: WorkspaceView }) {
  const project = projects.value.find((p) => p.id === ws.project);
  return (
    <aside class="info" aria-label={m.workspace.info.label}>
      <div>
        <div class="info-head">
          <h2>{m.workspace.info.heading}</h2>
          <button type="button" class="btn icon" aria-label={m.workspace.info.close} onClick={() => (details.value = false)}>
            <Icon name="close" />
          </button>
        </div>
        <dl>
          <dt>{m.workspace.info.project}</dt>
          <dd translate={false}>
            {project ? (
              <a href={`/p/${project.name}`}>
                {project.name}
              </a>
            ) : (
              m.workspace.info.none
            )}
          </dd>
          {ws.checkout ? (
            <>
              <dt>{m.workspace.info.repo}</dt>
              <dd translate={false}>{ws.checkout.repo}</dd>
              <dt>{m.workspace.info.branch}</dt>
              <dd translate={false}>{ws.checkout.branch}</dd>
            </>
          ) : null}
          <dt>{m.workspace.info.environment}</dt>
          <dd translate={false}>{ws.environment}</dd>
          <dt>{m.workspace.info.memory}</dt>
          <dd>{ws.memory === null ? m.workspace.info.none : m.workspace.info.memoryAmount(ws.memory)}</dd>
          <dt>{m.workspace.info.created}</dt>
          <dd>{new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(ws.created_at)}</dd>
        </dl>
        {ws.columns.length ? (
          <button type="button" class="btn" onClick={() => void saveOpening(ws)}>
            {m.workspace.info.saveOpening}
          </button>
        ) : null}
      </div>
      {ws.newer_build ? (
        <div class="update">
          <h2>{m.workspace.info.newer(ws.environment)}</h2>
          <p class="muted">{m.workspace.info.newerAbout}</p>
          <button type="button" class="btn primary" onClick={() => void update(ws)}>
            {m.workspace.info.update}
          </button>
        </div>
      ) : null}
      <Access ws={ws} />
      <History ws={ws} />
      {ws.routes.length ? (
        <div>
          <h2>{m.workspace.info.previews}</h2>
          {ws.routes.map((r) => (
            <RouteRow key={r.id} ws={ws} route={r} />
          ))}
        </div>
      ) : null}
      <div class="w-actions">
        {isAsking("delete") ? (
          <div class="confirm">
            <span>{m.workspace.confirmDelete(called(ws), lossText(ws))}</span>
            <button type="button" class="btn danger" onClick={() => void remove(ws)}>
              {m.workspace.delete}
            </button>
            <button type="button" class="btn" onClick={() => settle(ws)}>
              {m.workspace.keepIt}
            </button>
          </div>
        ) : (
          <>
            {ws.phase === "running" || ws.phase === "frozen" ? (
              <button type="button" class="btn" onClick={() => void setState(ws, "stopped")}>
                {m.workspace.stop}
              </button>
            ) : null}
            <button type="button" class="btn danger" onClick={() => ask(ws, { kind: "delete" })}>
              {m.workspace.delete}
            </button>
          </>
        )}
      </div>
    </aside>
  );
}

/** What the workspace may do from inside, with `iglu`: to itself, and to
 * the other workspaces it's been given. Any program in it can do as much. */
function Access({ ws }: { ws: WorkspaceView }) {
  const [saving, setSaving] = useState(false);
  const nameOf = (id: string) => (id === ws.id ? m.workspace.access.self : (workspaces.value.find((w) => w.id === id)?.name ?? m.workspace.access.gone));
  const save = (grants: AccessGrant[]) => {
    setSaving(true);
    api.putAccess(ws.id, grants).then(
      () => setSaving(false),
      (error: unknown) => {
        setSaving(false);
        say(m.workspace.access.refused(failure(error)));
      },
    );
  };
  const addable = workspaces.value.filter((w) => w.id !== ws.id && !ws.access.some((grant) => grant.workspace === w.id));
  return (
    <div>
      <h2>{m.workspace.access.heading}</h2>
      <p class="muted access-note">{m.workspace.access.about}</p>
      <ul class="access" aria-busy={saving}>
        {named(ws.access, ws.id).map((id) => (
          <li key={id}>
            <span class="access-on" translate={id !== ws.id ? false : undefined}>
              {nameOf(id)}
            </span>
            <div class="access-perms" role="group" aria-label={m.workspace.access.may(nameOf(id))}>
              {PERMISSIONS.map((permission) => (
                <button
                  key={permission}
                  type="button"
                  title={m.permission(permission).title}
                  disabled={saving}
                  aria-pressed={holds(ws.access, id, permission)}
                  onClick={() => save(toggled(ws.access, id, permission))}
                >
                  {m.permission(permission).label}
                </button>
              ))}
            </div>
          </li>
        ))}
      </ul>
      {addable.length ? (
        <select
          class="access-add"
          aria-label={m.workspace.access.add}
          value=""
          disabled={saving}
          onChange={(e) => {
            const id = e.currentTarget.value;
            if (id) save(toggled(ws.access, id, "view"));
          }}
        >
          <option value="">{m.workspace.access.addHint}</option>
          {addable.map((w) => (
            <option key={w.id} value={w.id} translate={false}>
              {called(w)}
            </option>
          ))}
        </select>
      ) : null}
    </div>
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
      <h2>{m.history.heading}</h2>
      <ol class="history">
        {entries.map((entry, i) => {
          const { what, detail } = m.history.happened(entry);
          const via = entry.via === null ? null : entry.via === ws.name ? { self: true as const } : { name: entry.via };
          return (
            <li key={`${entry.at}-${i}`}>
              <time dateTime={new Date(entry.at).toISOString()}>{stamp.format(entry.at)}</time>
              <span class="k">{m.history.by(what, via)}</span>
              {detail ? <span class="d">{detail}</span> : null}
            </li>
          );
        })}
      </ol>
    </div>
  );
}

function RouteRow({ ws, route: r }: { ws: WorkspaceView; route: RouteView }) {
  const [sure, setSure] = useState(false);
  return (
    <div class="row">
      <a href={r.url} target="_blank" rel="noopener" translate={false} title={m.preview.onlyYou}>
        {r.name} :{r.port}
      </a>
      <CopyLink url={r.url} name={r.name} />
      {sure ? (
        <button type="button" class="btn danger" onClick={() => void unpublish(ws, r)}>
          {m.workspace.unpublishSure}
        </button>
      ) : (
        <button type="button" class="btn" title={m.workspace.unpublishTitle} onClick={() => setSure(true)}>
          {m.workspace.unpublish}
        </button>
      )}
    </div>
  );
}
