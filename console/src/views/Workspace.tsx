// One workspace: the tree beside it, its header, and its strip of columns.

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
  loadColumns,
  loadLive,
  markActive,
  moveColumn,
  openColumn,
  publish,
  rename,
  restartColumn,
  saveOpening,
  setState,
  toggleFreeze,
  unpublish,
} from "../actions.ts";
import { api } from "../api/client.ts";
import { FieldError, FormError, InputError, invalid, useForm } from "../components/forms.tsx";
import { Previews } from "../components/Previews.tsx";
import {
  attentionGlyph,
  attentionText,
  buildRows,
  Frost,
  Glyph,
  Igloo,
  phaseText,
  workspaceGlyph,
} from "../components/bits.tsx";
import type { ActivityEntry } from "../generated/ActivityEntry.ts";
import type { AttentionView } from "../generated/AttentionView.ts";
import type { ColumnState } from "../generated/ColumnState.ts";
import type { RouteView } from "../generated/RouteView.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { terminalShortcut } from "../keyboard.ts";
import { FRACTION, LABEL, type Shown } from "../state/layout.ts";
import { bySession } from "../state/threads.ts";
import { unreachable, unsavedText } from "../state/unsaved.ts";
import { situation } from "../state/situation.ts";
import { addingColumn, addingPort, closing, inside, projects, collapsed, confirming, details, filter, groups, navigate, renaming, route } from "../state/store.ts";
import { loadGhostty, TerminalPane } from "../terminal.ts";

export function Workspace({ ws }: { ws: WorkspaceView | null }) {
  const r = route.value;
  return (
    <div class="wsv" data-phase={ws?.phase}>
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
  const query = filter.value.trim().toLowerCase();
  const shown = groups.value
    .map((group) => ({
      ...group,
      workspaces: group.workspaces.filter((ws) => !query || [ws.name, ws.checkout?.branch ?? "", group.label, ws.attention?.summary ?? ""].some((t) => t.toLowerCase().includes(query))),
    }))
    .filter((group) => group.workspaces.length > 0);
  const toggle = (key: string) => {
    const next = new Set(collapsed.value);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    collapsed.value = next;
  };
  return (
    <nav class="side" aria-label="Workspaces">
      <div class="tree-tools">
        <input
          type="search"
          name="filter"
          placeholder="Filter…"
          aria-label="Filter workspaces"
          autocomplete="off"
          spellcheck={false}
          value={filter.value}
          onInput={(e) => (filter.value = e.currentTarget.value)}
        />
        <button type="button" aria-label="Collapse all" title="Collapse all" onClick={() => (collapsed.value = new Set(groups.value.map((g) => g.key)))}>
          ⊟
        </button>
        <button type="button" aria-label="Expand all" title="Expand all" onClick={() => (collapsed.value = new Set())}>
          ⊞
        </button>
      </div>
      <div class="tree">
        {shown.map((group) => {
          const closed = collapsed.value.has(group.key) && !query;
          const need = group.workspaces.filter((ws) => ws.needs_you).length;
          return (
            <div key={group.key} role="group" aria-label={group.label}>
              <div class="t-group">
                <button type="button" class="chev" aria-expanded={!closed} aria-label={`${closed ? "Expand" : "Collapse"} ${group.label}`} onClick={() => toggle(group.key)}>
                  {closed ? "▸" : "▾"}
                </button>
                <span class="label" translate={false}>
                  {group.label}
                </span>
                <span class={`cnt${need ? " hot" : ""}`}>{need ? `● ${need}` : group.workspaces.length}</span>
              </div>
              {closed
                ? null
                : group.workspaces.map((ws) => (
                    <a
                      key={ws.id}
                      href={`/w/${ws.name}`}
                      class={`t-ws${ws.needs_you ? " needs" : ""}${ws.phase === "stopped" || ws.phase === "frozen" ? " asleep" : ""}`}
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
    </div>
  );
}

function Header({ ws }: { ws: WorkspaceView }) {
  const phase = phaseText(ws.phase);
  const naming = useForm(async (data) => {
    const name = data.get("name");
    if (typeof name !== "string" || name.trim() === ws.name) {
      renaming.value = false;
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
        {renaming.value ? (
          <form class="rename" onSubmit={naming.onSubmit}>
            <input
              name="name"
              aria-label="Workspace name"
              defaultValue={ws.name}
              autocomplete="off"
              spellcheck={false}
              autoFocus
              onFocus={(e) => e.currentTarget.select()}
              {...invalid(naming, "name")}
            />
            <button type="submit" class="btn">
              Rename
            </button>
            <button type="button" class="btn" onClick={() => (renaming.value = false)}>
              Cancel
            </button>
            <FieldError form={naming} input="name" />
            <FormError form={naming} />
          </form>
        ) : (
          <div class="w-title">
            <Glyph kind={workspaceGlyph(ws)} />
            <button type="button" class="nm" title="Rename (e)" translate={false} onClick={() => (renaming.value = true)}>
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
            addingPort.value ? (
              <form class="addport" onSubmit={publishing.onSubmit}>
                <input name="port" inputMode="numeric" placeholder="3000…" aria-label="Port to publish" autocomplete="off" autoFocus {...invalid(publishing, "port")} />
                <button type="submit" class="btn" disabled={publishing.busy}>
                  Publish
                </button>
                <FieldError form={publishing} input="port" />
                <FormError form={publishing} />
              </form>
            ) : (
              <button type="button" class="chip" aria-label="Publish a port" onClick={() => (addingPort.value = true)}>
                + port
              </button>
            )
          ) : null}
        </div>
        <div class="w-actions">
          {ws.phase === "running" ? (
            <button type="button" class="btn icon" aria-label="Freeze" title="Freeze (f)" onClick={() => void toggleFreeze(ws)}>
              ❄
            </button>
          ) : ws.phase === "frozen" ? (
            <button type="button" class="btn" title="Thaw (f)" onClick={() => void toggleFreeze(ws)}>
              Thaw
            </button>
          ) : ws.phase === "stopped" ? (
            <button type="button" class="btn primary" onClick={() => void setState(ws, "running")}>
              Start
            </button>
          ) : null}
          <button type="button" class="btn icon" aria-label="Details" title="Details (i)" aria-pressed={details.value} onClick={() => (details.value = !details.value)}>
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
  return (
    <div class="w-strip">
      <nav class="minimap" aria-label="Columns">
        {columns.map((c) => (
          <button
            type="button"
            aria-current={c.name === active ? "true" : undefined}
            key={c.name}
            class={`sc${c.state === "ended" ? " ended" : ""}`}
            onClick={() => focusColumn(ws, c.name)}
          >
            <Glyph kind={attentionGlyph(threads.get(c.name)?.[0] ?? null)} />
            <span translate={false}>{c.name}</span>
          </button>
        ))}
      </nav>
      <div class="sc-adder">
        <button
          type="button"
          class="sc-add"
          aria-label="Add a column"
          aria-expanded={addingColumn.value}
          title="Add a column (a)"
          onClick={() => (addingColumn.value = !addingColumn.value)}
        >
          +
        </button>
        {addingColumn.value ? <AddMenu ws={ws} /> : null}
      </div>
    </div>
  );
}

function AddMenu({ ws }: { ws: WorkspaceView }) {
  const [command, setCommand] = useState(false);
  useEffect(() => {
    const away = (e: PointerEvent) => {
      if (!(e.target instanceof Element && e.target.closest(".sc-adder"))) addingColumn.value = false;
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
      <form class="add-menu" onSubmit={run.onSubmit}>
        <input name="command" aria-label="Command to run" placeholder="npm run dev…" autocomplete="off" spellcheck={false} autoFocus {...invalid(run, "command")} />
        <button type="submit" class="btn" disabled={run.busy}>
          Run
        </button>
        <FieldError form={run} input="command" />
        <FormError form={run} />
      </form>
    );
  }
  return (
    <div class="add-menu" role="group" aria-label="Add a column">
      <button type="button" autoFocus onClick={() => void addColumn(ws, { kind: "shell" })}>
        Shell
      </button>
      {ws.agents.map((agent) => (
        <button type="button" key={agent} translate={false} onClick={() => void addColumn(ws, { kind: "agent", agent })}>
          {agent}
        </button>
      ))}
      <button type="button" onClick={() => setCommand(true)}>
        Command…
      </button>
    </div>
  );
}

function Columns({ ws }: { ws: WorkspaceView }) {
  const columns = columnsOf(ws);
  const active = activeOf(ws);
  const strip = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = strip.current?.querySelector<HTMLElement>(`[data-column="${CSS.escape(active ?? "")}"]`);
    el?.scrollIntoView({ inline: "nearest", block: "nearest", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }, [active, columns.length]);
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
        <Column key={`${ws.id}/${c.name}`} ws={ws} column={c} on={c.name === active} threads={threads.get(c.name) ?? []} />
      ))}
    </div>
  );
}

function Column(props: { ws: WorkspaceView; column: Shown; on: boolean; threads: AttentionView[] }) {
  const { ws, column, threads } = props;
  const attention = threads[0] ?? null;
  const [listing, setListing] = useState(false);
  const { name } = column;
  const state = attention ? attentionText(attention) : null;
  const arranged = arrangeable(column.state);
  const [status, setStatus] = useState("");
  return (
    <section class={`col${props.on ? " on" : ""}`} style={{ "--cw": String(FRACTION[column.width]) }} data-column={name} aria-label={`Column ${name}`}>
      <header class={`col-h${attention?.state === "waiting" ? " asks" : ""}`}>
        <Glyph kind={attentionGlyph(attention)} />
        <b translate={false}>{name}</b>
        {attention?.summary ? <span class="ct">{attention.summary}</span> : null}
        {state && state.text !== "idle" ? <span class={`state ${state.tone}`}>{state.text}</span> : null}
        {threads.length > 1 ? (
          <button type="button" class="threads-btn" aria-expanded={listing} onClick={() => setListing(!listing)}>
            {threads.length} threads
          </button>
        ) : null}
        {status && column.state !== "ended" ? <span class="status">{status}</span> : null}
        {closing.value === name ? (
          <span class="col-ctl" style={{ opacity: 1 }}>
            <button type="button" class="end" onClick={() => void closeColumn(ws, name)}>
              End {name}
            </button>
            <button type="button" onClick={() => (closing.value = null)}>
              Keep
            </button>
          </span>
        ) : (
          <span class="col-ctl">
            {arranged ? (
              <>
                <button type="button" class="wbtn" title="Width (w)" aria-label={`Width of ${name}: ${LABEL[column.width]}`} onClick={() => void cycleWidth(ws, name)}>
                  {LABEL[column.width]}
                </button>
                <button type="button" aria-label={`Move ${name} left`} onClick={() => (markActive(ws, name), void moveColumn(ws, -1))}>
                  ‹
                </button>
                <button type="button" aria-label={`Move ${name} right`} onClick={() => (markActive(ws, name), void moveColumn(ws, 1))}>
                  ›
                </button>
              </>
            ) : null}
            <button type="button" aria-label={`End ${name}`} title="End this column (x)" onClick={() => (closing.value = name)}>
              ×
            </button>
          </span>
        )}
      </header>
      {listing && threads.length > 1 ? (
        <ul class="threads" aria-label={`Threads in ${name}`}>
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
      return <Pane ws={ws} name={column.name} on={props.on} onStatus={props.onStatus} />;
    default:
      return unreachable(column.state);
  }
}

function Pane(props: { ws: WorkspaceView; name: string; on: boolean; onStatus: (status: string) => void }) {
  const { ws, name, onStatus } = props;
  const host = useRef<HTMLDivElement>(null);
  // Loading ghostty is asynchronous; by the time it's done another column may be active.
  const on = useRef(props.on);
  on.current = props.on;
  useEffect(() => {
    let pane: TerminalPane | null = null;
    let cancelled = false;
    void loadGhostty().then((ghostty) => {
      if (cancelled || !host.current) return;
      pane = new TerminalPane({
        container: host.current,
        ghostty,
        workspace: ws.id,
        session: name,
        shortcuts: terminalShortcut,
        wantsFocus: () => on.current,
        onFocus: () => markActive(ws, name),
        onStatus,
        onDrop: () => void loadColumns(ws),
      });
    });
    return () => {
      cancelled = true;
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
      return (
        <div class={`cols-note problem ${now.kind === "held" ? "calm" : "trouble"}`} role="status">
          <b>{now.title}</b>
          <p translate={false}>{now.detail}</p>
          <div class="acts">
            {now.kind === "stuck" && ws.phase === "creating" && project?.repo ? (
              <a class="btn" href={`/p/${project.name}`} onClick={(e) => (e.preventDefault(), navigate({ view: "project", name: project.name }))}>
                Check {project.name}'s repository
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
                confirming.value = "delete";
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
          <Frost label={now.label} hint="f to thaw" />
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
        {confirming.value === "delete" ? (
          <div class="confirm">
            <span>
              Delete {ws.name}? {lossText(ws)}
            </span>
            <button type="button" class="btn danger" onClick={() => void setState(ws, "deleted").then(() => navigate({ view: "overview" }))}>
              Delete
            </button>
            <button type="button" class="btn" onClick={() => (confirming.value = null)}>
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
            <button type="button" class="btn danger" onClick={() => (confirming.value = "delete")}>
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
      (list) => current && setEntries([...list].reverse()),
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
      <a href={r.url} target="_blank" rel="noopener" translate={false}>
        {r.name} :{r.port}
      </a>
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
