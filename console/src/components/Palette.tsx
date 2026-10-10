// Search and commands: everything in one list.

import { useEffect, useId, useMemo, useRef, useState } from "preact/hooks";

import { api } from "../api/client.ts";
import {
  activeOf,
  addColumn,
  columnsOf,
  cycleWidth,
  focusColumn,
  labelOrSay,
  moveColumn,
  nextWaiting,
  open,
  restartColumn,
  toggleFreeze,
  toggleZoom,
  zoomedIn,
} from "../actions.ts";
import { afterPrefix, keysFor, touchOnly } from "../keyboard.ts";
import { enableNotifications } from "../notify.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { LABEL, titleOf } from "../state/layout.ts";
import { search } from "../state/search.ts";
import { ask, current, details, listed, look, navigate, overlay, previous, projects, say, workspaces } from "../state/store.ts";
import { copyLink, ONLY_YOU } from "./CopyLink.tsx";
import { Modal } from "./Modal.tsx";

interface Item {
  label: string;
  sub?: string | undefined;
  hint?: string | undefined;
  run: () => void;
}

/** What can be done with the open workspace's columns: go to one, or act on
 * the one that has the keyboard, with the keys for each. */
function columnItems(ws: WorkspaceView): Item[] {
  const list: Item[] = [];
  const columns = columnsOf(ws);
  const active = activeOf(ws);
  columns.forEach((c, i) => {
    if (c.name !== active) list.push({ label: `Go to column ${titleOf(c)}`, sub: c.label ? `session ${c.name}` : undefined, hint: i < 9 ? afterPrefix(String(i + 1)) : undefined, run: () => focusColumn(ws, c.name) });
  });
  const here = columns.findIndex((c) => c.name === active);
  const column = columns[here];
  if (!column) return list;
  const title = titleOf(column);
  if (column.state === "adopted") {
    list.push({ label: `End ${title}`, hint: keysFor("close-column"), run: () => ask(ws, { kind: "end", column: column.name }) });
    return list;
  }
  const zoom = zoomedIn(ws) === column.name;
  list.push(
    zoom
      ? { label: `Unzoom ${title}`, sub: "Put it back as it was", hint: keysFor("zoom"), run: () => toggleZoom(ws) }
      : { label: `Zoom ${title}`, sub: "Fill the page with it", hint: keysFor("zoom"), run: () => toggleZoom(ws) },
    { label: `Change ${title}'s width`, sub: `It's ${LABEL[column.width]} of the strip`, hint: keysFor("width"), run: () => void cycleWidth(ws) },
  );
  if (here > 0) list.push({ label: `Move ${title} left`, hint: keysFor({ kind: "move-column", step: -1 }), run: () => void moveColumn(ws, -1) });
  if (here < columns.length - 1) list.push({ label: `Move ${title} right`, hint: keysFor({ kind: "move-column", step: 1 }), run: () => void moveColumn(ws, 1) });
  list.push({ label: `Rename column ${title}`, sub: column.label ? `session ${column.name}` : undefined, hint: keysFor("label-column"), run: () => labelOrSay(ws, column.name) });
  if (column.state !== "ended" && column.state !== null) list.push({ label: `Find in ${title}`, sub: "Search its output", hint: keysFor("find"), run: () => ask(ws, { kind: "find", column: column.name }) });
  if (column.state === "ended") list.push({ label: `Restart ${title}`, run: () => void restartColumn(ws, column.name) });
  list.push({ label: `End ${title}`, hint: keysFor("close-column"), run: () => ask(ws, { kind: "end", column: column.name }) });
  return list;
}

function items(): Item[] {
  const ws = current.value;
  const list: Item[] = [
    { label: "New workspace", hint: keysFor("new"), run: () => (overlay.value = "new") },
    { label: "New project", run: () => (overlay.value = "project") },
    { label: "Next workspace that needs you", hint: keysFor("next-waiting"), run: nextWaiting },
    { label: "Overview", run: () => navigate({ view: "overview" }) },
    { label: "Previews", hint: keysFor("previews"), run: () => navigate({ view: "previews" }) },
    { label: "Settings", sub: "Environments, secrets, keyboard", run: () => navigate({ view: "settings" }) },
    // Keys are no help on a touch screen.
    ...(touchOnly ? [] : [{ label: "Keyboard shortcuts", hint: keysFor("keys"), run: () => (overlay.value = "keys") }]),
  ];
  const before = workspaces.value.find((w) => w.id === previous.value && w.id !== ws?.id);
  if (before) list.push({ label: `Back to ${before.name}`, sub: "The workspace you were in before", hint: keysFor("last-workspace"), run: () => open(before) });
  if (ws) {
    list.push({ label: `Rename ${ws.name}`, hint: keysFor("rename"), run: () => ask(ws, { kind: "rename" }) });
    list.push({ label: `Details of ${ws.name}`, hint: keysFor("details"), run: () => (details.value = true) });
    list.push(...columnItems(ws));
    for (const r of ws.routes) list.push({ label: `Copy the link to ${r.name}`, sub: ONLY_YOU, run: () => copyLink(r.url) });
    if (ws.phase === "running") {
      list.push({ label: `Shell in ${ws.name}`, run: () => void addColumn(ws, { kind: "shell" }) });
      for (const agent of ws.agents) list.push({ label: `${agent} in ${ws.name}`, run: () => void addColumn(ws, { kind: "agent", agent }) });
      list.push({ label: `Server in ${ws.name}`, sub: "A command that keeps running, such as a dev server", run: () => ask(ws, { kind: "add-column", server: true }) });
      list.push({ label: `Freeze ${ws.name}`, hint: keysFor("freeze"), run: () => void toggleFreeze(ws) });
    }
    if (ws.phase === "frozen") list.push({ label: `Thaw ${ws.name}`, hint: keysFor("freeze"), run: () => void toggleFreeze(ws) });
  }
  for (const w of listed.value) list.push({ label: w.name, sub: w.attention?.summary || (w.checkout ? `⎇ ${w.checkout.branch}` : ""), run: () => open(w) });
  for (const p of projects.value) list.push({ label: p.name, sub: "Project", run: () => navigate({ view: "project", name: p.name }) });
  for (const w of listed.value) for (const r of w.routes) list.push({ label: `Open ${r.name}`, sub: `${w.name} :${r.port}`, run: () => window.open(r.url, "_blank", "noopener") });
  list.push(
    { label: "Look: match the system", run: () => (look.value = "auto") },
    { label: "Look: Polar night", run: () => (look.value = "dark") },
    { label: "Look: Snowfield", run: () => (look.value = "light") },
    {
      label: "Turn on notifications",
      run: () => void enableNotifications().then((p) => say(p === "granted" ? "Notifications are on." : "Notifications are blocked in this browser.")),
    },
    { label: "Sign out", run: () => void api.logout().then(() => location.assign("/")) },
  );
  return list;
}

export function Palette() {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const all = useMemo(items, [current.value, listed.value, projects.value]);
  const shown = search(query, all);
  useEffect(() => setIndex(0), [query]);
  // The command runs once the palette has closed and given focus back, so a
  // command that moves focus, to a column or a question, keeps it there.
  const next = useRef<Item | null>(null);
  const run = (item: Item) => {
    next.current = item;
    overlay.value = null;
  };
  const list = useId();
  return (
    <Modal label="Search and commands" onClose={() => (overlay.value = null)} onClosed={() => next.current?.run()}>
      <div class="pbox">
        <input
          autofocus
          type="text"
          name="q"
          aria-label="Search"
          role="combobox"
          aria-expanded="true"
          aria-controls={list}
          aria-autocomplete="list"
          aria-activedescendant={shown[index] ? `${list}-${index}` : undefined}
          placeholder="Workspaces, previews, commands…"
          autocomplete="off"
          spellcheck={false}
          value={query}
          onInput={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") setIndex((i) => Math.min(shown.length - 1, i + 1));
            else if (e.key === "ArrowUp") setIndex((i) => Math.max(0, i - 1));
            else if (e.key === "Enter") {
              // With nothing found, there's nothing to do: the search stays to be fixed.
              const item = shown[index];
              if (item) run(item);
            }
            else return;
            e.preventDefault();
          }}
        />
        <ul class="plist" id={list} role="listbox" aria-label="Results">
          {shown.length === 0 ? <li class="pempty">No matches.</li> : null}
          {shown.map((item, i) => (
            <li
              key={`${item.label}/${item.sub ?? ""}`}
              id={`${list}-${i}`}
              class="pitem"
              role="option"
              aria-selected={i === index}
              ref={(el) => {
                if (i === index) el?.scrollIntoView({ block: "nearest" });
              }}
              onClick={() => run(item)}
            >
              <span translate={false}>{item.label}</span>
              {item.hint ? <span class="h">{item.hint}</span> : null}
              {item.sub ? <span class="sub">{item.sub}</span> : null}
            </li>
          ))}
        </ul>
      </div>
    </Modal>
  );
}
