// Search and commands: everything in one list.

import { useEffect, useMemo, useState } from "preact/hooks";

import { api } from "../api/client.ts";
import { addColumn, nextWaiting, open, toggleFreeze } from "../actions.ts";
import { enableNotifications } from "../notify.ts";
import { current, details, listed, look, navigate, overlay, renaming, say } from "../state/store.ts";

interface Item {
  label: string;
  sub?: string;
  hint?: string;
  run: () => void;
}

function items(): Item[] {
  const ws = current.value;
  const list: Item[] = [
    { label: "New workspace", hint: "n", run: () => (overlay.value = "new") },
    { label: "New project", run: () => (overlay.value = "project") },
    { label: "Next waiting", hint: "⌥N", run: nextWaiting },
    { label: "Overview", run: () => navigate({ view: "overview" }) },
    { label: "Previews", hint: "p", run: () => navigate({ view: "previews" }) },
  ];
  if (ws) {
    list.push({ label: `Rename ${ws.name}`, hint: "e", run: () => (renaming.value = true) });
    list.push({ label: `Details of ${ws.name}`, hint: "i", run: () => (details.value = true) });
    if (ws.phase === "running") {
      list.push({ label: `Shell in ${ws.name}`, hint: "a", run: () => void addColumn(ws, { kind: "shell" }) });
      for (const agent of ws.agents) list.push({ label: `${agent} in ${ws.name}`, run: () => void addColumn(ws, { kind: "agent", agent }) });
      list.push({ label: `Freeze ${ws.name}`, hint: "f", run: () => void toggleFreeze(ws) });
    }
    if (ws.phase === "frozen") list.push({ label: `Thaw ${ws.name}`, hint: "f", run: () => void toggleFreeze(ws) });
  }
  for (const w of listed.value) list.push({ label: w.name, sub: w.attention?.summary || (w.checkout ? `⎇ ${w.checkout.branch}` : ""), run: () => open(w) });
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

function matches(query: string, text: string): boolean {
  let at = 0;
  for (const c of text.toLowerCase()) if (c === query[at]) at++;
  return at === query.length;
}

export function Palette() {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const all = useMemo(items, [current.value, listed.value]);
  const q = query.trim().toLowerCase();
  const shown = all.filter((item) => !q || matches(q, `${item.label} ${item.sub ?? ""}`));
  useEffect(() => setIndex(0), [query]);
  const run = (item: Item | undefined) => {
    overlay.value = null;
    item?.run();
  };
  return (
    <div class="overlay" onClick={(e) => e.target === e.currentTarget && (overlay.value = null)}>
      <div class="pbox" role="dialog" aria-label="Search and commands">
        <input
          type="text"
          name="q"
          aria-label="Search"
          placeholder="Workspaces, previews, commands…"
          autocomplete="off"
          spellcheck={false}
          autoFocus
          value={query}
          onInput={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") setIndex((i) => Math.min(shown.length - 1, i + 1));
            else if (e.key === "ArrowUp") setIndex((i) => Math.max(0, i - 1));
            else if (e.key === "Enter") run(shown[index]);
            else if (e.key === "Escape") overlay.value = null;
            else return;
            e.preventDefault();
          }}
        />
        <ul class="plist" role="listbox" aria-label="Results">
          {shown.length === 0 ? <li class="pempty">No matches.</li> : null}
          {shown.map((item, i) => (
            <li
              key={`${item.label}/${item.sub ?? ""}`}
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
    </div>
  );
}
