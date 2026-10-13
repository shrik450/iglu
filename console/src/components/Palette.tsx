// Search and commands: everything in one list.

import { useEffect, useId, useMemo, useRef, useState } from "preact/hooks";

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
  setState,
  signOut,
  toggleFreeze,
  toggleZoom,
  update,
  zoomedIn,
} from "../actions.ts";
import { afterPrefix, keysFor, touchOnly } from "../keyboard.ts";
import { m } from "../messages.tsx";
import { enableNotifications } from "../notify.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { LABEL, titleOf } from "../state/layout.ts";
import { search } from "../state/search.ts";
import { setLook } from "../preferences.ts";
import { ask, current, details, listed, navigate, overlay, previous, projects, say, workspaces } from "../state/store.ts";
import { copyLink } from "./CopyLink.tsx";
import { Modal } from "./Modal.tsx";
import { called } from "../state/naming.ts";

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
    if (c.name !== active) list.push({ label: m.palette.goTo(titleOf(c)), sub: c.label ? m.palette.session(c.name) : undefined, hint: i < 9 ? afterPrefix(String(i + 1)) : undefined, run: () => focusColumn(ws, c.name) });
  });
  const here = columns.findIndex((c) => c.name === active);
  const column = columns[here];
  if (!column) return list;
  const title = titleOf(column);
  if (column.state === "adopted") {
    list.push({ label: m.palette.end(title), hint: keysFor("close-column"), run: () => ask(ws, { kind: "end", column: column.name }) });
    return list;
  }
  const zoom = zoomedIn(ws) === column.name;
  list.push(
    zoom
      ? { label: m.palette.unzoom(title), sub: m.palette.unzoomAbout, hint: keysFor("zoom"), run: () => toggleZoom(ws) }
      : { label: m.palette.zoom(title), sub: m.palette.zoomAbout, hint: keysFor("zoom"), run: () => toggleZoom(ws) },
    { label: m.palette.width(title), sub: m.palette.widthAbout(LABEL[column.width]), hint: keysFor("width"), run: () => void cycleWidth(ws) },
  );
  if (here > 0) list.push({ label: m.palette.moveLeft(title), hint: keysFor({ kind: "move-column", step: -1 }), run: () => void moveColumn(ws, -1) });
  if (here < columns.length - 1) list.push({ label: m.palette.moveRight(title), hint: keysFor({ kind: "move-column", step: 1 }), run: () => void moveColumn(ws, 1) });
  list.push({ label: m.palette.renameColumn(title), sub: column.label ? m.palette.session(column.name) : undefined, hint: keysFor("label-column"), run: () => labelOrSay(ws, column.name) });
  if (column.state !== "ended" && column.state !== null) list.push({ label: m.palette.find(title), sub: m.palette.findAbout, hint: keysFor("find"), run: () => ask(ws, { kind: "find", column: column.name }) });
  if (column.state === "ended") list.push({ label: m.palette.restart(title), run: () => void restartColumn(ws, column.name) });
  list.push({ label: m.palette.end(title), hint: keysFor("close-column"), run: () => ask(ws, { kind: "end", column: column.name }) });
  return list;
}

function items(): Item[] {
  const ws = current.value;
  const list: Item[] = [
    { label: m.palette.newWorkspace, hint: keysFor("new"), run: () => (overlay.value = "new") },
    { label: m.palette.newProject, run: () => (overlay.value = "project") },
    { label: m.palette.nextWaiting, hint: keysFor("next-waiting"), run: nextWaiting },
    { label: m.palette.overview, run: () => navigate({ view: "overview" }) },
    { label: m.palette.previews, hint: keysFor("previews"), run: () => navigate({ view: "previews" }) },
    { label: m.palette.settings, sub: m.palette.settingsAbout, run: () => navigate({ view: "settings" }) },
    // Keys are no help on a touch screen.
    ...(touchOnly ? [] : [{ label: m.palette.keys, hint: keysFor("keys"), run: () => (overlay.value = "keys") }]),
  ];
  const before = workspaces.value.find((w) => w.id === previous.value && w.id !== ws?.id);
  if (before) list.push({ label: m.palette.back(before.name), sub: m.palette.backAbout, hint: keysFor("last-workspace"), run: () => open(before) });
  if (ws) {
    list.push({ label: m.palette.rename(called(ws)), hint: keysFor("rename"), run: () => ask(ws, { kind: "rename" }) });
    list.push({ label: m.palette.details(called(ws)), hint: keysFor("details"), run: () => (details.value = true) });
    const project = projects.value.find((p) => p.id === ws.project);
    if (project) list.push({ label: m.palette.project(project.name), sub: m.palette.projectAbout(called(ws)), run: () => navigate({ view: "project", name: project.name }) });
    list.push(...columnItems(ws));
    for (const r of ws.routes) list.push({ label: m.palette.copyLink(r.name), sub: m.preview.onlyYou, run: () => copyLink(r.url) });
    if (ws.phase === "running") {
      list.push({ label: m.palette.shell(called(ws)), run: () => void addColumn(ws, { kind: "shell" }) });
      if (ws.browser && !ws.columns.some((c) => c.kind.kind === "browser")) list.push({ label: m.palette.browser(called(ws)), run: () => void addColumn(ws, { kind: "browser" }) });
      list.push({ label: m.palette.publish(called(ws)), sub: m.preview.onlyYou, run: () => ask(ws, { kind: "port" }) });
      list.push({ label: m.palette.freeze(called(ws)), hint: keysFor("freeze"), run: () => void toggleFreeze(ws) });
    }
    if (ws.phase === "frozen") list.push({ label: m.palette.thaw(called(ws)), hint: keysFor("freeze"), run: () => void toggleFreeze(ws) });
    if (ws.phase === "running" || ws.phase === "frozen") list.push({ label: m.palette.stop(called(ws)), sub: m.palette.stopAbout, run: () => void setState(ws, "stopped") });
    if (ws.phase === "stopped") list.push({ label: m.palette.start(called(ws)), run: () => void setState(ws, "running") });
    if (ws.newer_build) list.push({ label: m.palette.update(called(ws)), sub: m.palette.updateAbout(ws.environment), run: () => void update(ws) });
    list.push({
      label: m.palette.delete(called(ws)),
      sub: m.palette.deleteAbout,
      run: () => {
        details.value = true;
        ask(ws, { kind: "delete" });
      },
    });
  }
  for (const w of listed.value) list.push({ label: called(w), sub: w.attention?.summary || (w.checkout ? m.palette.branch(w.checkout.branch) : ""), run: () => open(w) });
  for (const p of projects.value) {
    list.push({ label: p.name, sub: m.palette.projectKind, run: () => navigate({ view: "project", name: p.name }) });
    list.push({ label: m.palette.projectSettings(p.name), sub: m.palette.projectSettingsAbout, run: () => navigate({ view: "project", name: p.name, settings: true }) });
  }
  for (const w of listed.value) for (const r of w.routes) list.push({ label: m.palette.openPreview(r.name), sub: m.palette.previewAbout(called(w), r.port), run: () => window.open(r.url, "_blank", "noopener") });
  list.push(
    ...(["auto", "dark", "light"] as const).map((look) => ({ label: m.palette.look(look), run: () => setLook(look) })),
    {
      label: m.notifications.turnOn,
      run: () => void enableNotifications().then((p) => say(m.notifications.answer(p))),
    },
    { label: m.palette.signOut, run: () => void signOut() },
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
    <Modal label={m.palette.label} onClose={() => (overlay.value = null)} onClosed={() => next.current?.run()}>
      <div class="pbox">
        <input
          autofocus
          type="text"
          name="q"
          aria-label={m.palette.search}
          role="combobox"
          aria-expanded="true"
          aria-controls={list}
          aria-autocomplete="list"
          aria-activedescendant={shown[index] ? `${list}-${index}` : undefined}
          placeholder={m.palette.placeholder}
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
        <ul class="plist" id={list} role="listbox" aria-label={m.palette.results}>
          {shown.length === 0 ? <li class="pempty">{m.palette.none}</li> : null}
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
