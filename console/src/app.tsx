// The console's frame: the top bar, the current view, and overlays.

import { nextWaiting } from "./actions.ts";
import { Glyph, Icon, Igloo } from "./components/bits.tsx";
import { Keys, PrefixHint } from "./components/Keys.tsx";
import { keysFor, mac } from "./keyboard.ts";
import { onKeyboard } from "./state/keys.ts";
import { NewProject } from "./components/NewProject.tsx";
import { NewWorkspace } from "./components/NewWorkspace.tsx";
import { Palette } from "./components/Palette.tsx";
import { Overview } from "./views/Overview.tsx";
import { PreviewsView } from "./views/Previews.tsx";
import { ProjectPage } from "./views/Project.tsx";
import { Settings } from "./views/Settings.tsx";
import { Workspace } from "./views/Workspace.tsx";
import { current, flash, live, me, navigate, outdated, overlay, route, toasts, waiting, workspaces } from "./state/store.ts";

function Topbar() {
  const view = route.value.view;
  const count = waiting.value.length;
  const who = me.value?.email ?? me.value?.name ?? "";
  return (
    <header class="topbar">
      <div class="tb-left">
        <a
          class="tb-mark"
          href="/"
          translate={false}
          aria-label="iglu, overview"
          aria-current={view === "overview" ? "page" : undefined}
          onClick={(e) => {
            if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
            e.preventDefault();
            navigate({ view: "overview" });
          }}
        >
          iglu
        </a>
        {count ? (
          <button type="button" class="needs-badge" title={`Next workspace that needs you (${keysFor("next-waiting")})`} aria-label={`${count} waiting; go to the next`} onClick={nextWaiting}>
            <span aria-hidden="true">●</span>
            <span>
              {count}
              <span class="wide"> waiting</span>
            </span>
          </button>
        ) : null}
        {outdated.value ? (
          <span class="offline" role="status">
            iglu was updated.{" "}
            <button type="button" class="btn" onClick={() => location.reload()}>
              Reload
            </button>
          </span>
        ) : live.value ? null : (
          <span class="offline">Reconnecting…</span>
        )}
      </div>
      <div class="tb-right">
        {/* The mark is the way to the overview, so only previews needs a way here. */}
        <a
          class="tb-link"
          href="/previews"
          aria-current={view === "previews" ? "page" : undefined}
          title={`Previews (${keysFor("previews")})`}
          onClick={(e) => {
            if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
            e.preventDefault();
            navigate({ view: "previews" });
          }}
        >
          Previews
        </a>
        <button type="button" class="search" aria-label="Search and commands" onClick={() => (overlay.value = "palette")}>
          <span class="narrow">
            <Icon name="search" size={15} />
          </span>
          <span class="wide">Search…</span>
          <kbd>{onKeyboard("⌘ K", mac)}</kbd>
        </button>
        <button type="button" class="btn icon primary" aria-label="New workspace" title={`New workspace (${keysFor("new")})`} onClick={() => (overlay.value = "new")}>
          <Icon name="plus" size={16} />
        </button>
        <a
          class="who"
          href="/settings"
          aria-label={`Settings, signed in as ${who}`}
          title={who}
          aria-current={view === "settings" ? "page" : undefined}
          onClick={(e) => {
            if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
            e.preventDefault();
            navigate({ view: "settings" });
          }}
        >
          {who.slice(0, 1).toUpperCase()}
        </a>
      </div>
    </header>
  );
}

function Toasts() {
  return (
    <div class="toasts" aria-live="polite">
      {toasts.value.map((t) => (
        <a
          key={t.id}
          class="toast"
          href={`/w/${t.workspace}`}
          onClick={(e) => {
            toasts.value = toasts.value.filter((x) => x.id !== t.id);
            if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
            e.preventDefault();
            navigate({ view: "workspace", name: t.workspace });
          }}
        >
          <Igloo />
          <b>{t.title}</b>
          <span>{t.detail}</span>
        </a>
      ))}
    </div>
  );
}

function Flash() {
  const f = flash.value;
  return (
    <p class="flash" role="status" aria-live="polite" hidden={!f}>
      {f?.message}
      {f?.undo ? (
        <button
          type="button"
          class="lnk"
          onClick={() => {
            const undo = f.undo;
            flash.value = null;
            undo?.();
          }}
        >
          Undo
        </button>
      ) : null}
    </p>
  );
}

export function App() {
  const r = route.value;
  const loading = me.value === null;
  return (
    <>
      <a class="skip" href="#stage">
        Skip to content
      </a>
      <Topbar />
      <main class="stage" id="stage">
        {loading ? (
          <div class="empty-state">
            <Glyph kind="build" />
          </div>
        ) : r.view === "workspace" ? (
          <Workspace ws={current.value} />
        ) : r.view === "previews" ? (
          <PreviewsView />
        ) : r.view === "settings" ? (
          <Settings />
        ) : r.view === "project" ? (
          <ProjectPage />
        ) : workspaces.value ? (
          <Overview />
        ) : null}
      </main>
      {overlay.value === "palette" ? <Palette /> : null}
      {overlay.value === "new" ? <NewWorkspace /> : null}
      {overlay.value === "project" ? <NewProject /> : null}
      {overlay.value === "keys" ? <Keys /> : null}
      <PrefixHint />
      <Toasts />
      <Flash />
    </>
  );
}
