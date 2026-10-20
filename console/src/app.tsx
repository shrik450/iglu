// The console's frame: the top bar, the current view, and overlays.

import { nextWaiting } from "./actions.ts";
import { Glyph, Icon, Igloo } from "./components/bits.tsx";
import { Keys, PrefixHint } from "./components/Keys.tsx";
import { keysFor, mac } from "./keyboard.ts";
import { m } from "./messages.tsx";
import { onKeyboard } from "./state/keys.ts";
import { NewProject } from "./components/NewProject.tsx";
import { NewWorkspace } from "./components/NewWorkspace.tsx";
import { Palette } from "./components/Palette.tsx";
import { Overview } from "./views/Overview.tsx";
import { PreviewsView } from "./views/Previews.tsx";
import { ProjectPage } from "./views/Project.tsx";
import { Settings } from "./views/Settings.tsx";
import { Workspace } from "./views/Workspace.tsx";
import { current, flash, live, loaded, me, outdated, overlay, route, toasts, waiting, workspaces } from "./state/store.ts";

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
          aria-label={m.frame.home}
          aria-current={view === "overview" ? "page" : undefined}
        >
          iglu
        </a>
        {count ? (
          <button type="button" class="needs-badge" title={m.frame.waitingTitle(keysFor("next-waiting"))} aria-label={m.frame.waitingLabel(count)} onClick={nextWaiting}>
            <Glyph kind="waiting" />
            <span>{m.frame.waiting(count)}</span>
          </button>
        ) : null}
        {outdated.value ? (
          <span class="offline" role="status">
            {m.frame.updated}{" "}
            <button type="button" class="btn" onClick={() => location.reload()}>
              {m.frame.reload}
            </button>
          </span>
        ) : live.value ? null : (
          <span class="offline">{m.frame.reconnecting}</span>
        )}
      </div>
      <div class="tb-right">
        {/* The mark is the way to the overview, so only previews needs a way here. */}
        <a
          class="tb-link"
          href="/previews"
          aria-current={view === "previews" ? "page" : undefined}
          title={m.frame.previewsTitle(keysFor("previews"))}
        >
          {m.frame.previews}
        </a>
        <button type="button" class="search" aria-label={m.frame.search} onClick={() => (overlay.value = "palette")}>
          <span class="narrow">
            <Icon name="search" size={15} />
          </span>
          <span class="wide">{m.frame.searchWide}</span>
          <kbd>{onKeyboard("⌘ K", mac)}</kbd>
        </button>
        <button type="button" class="btn icon primary" aria-label={m.frame.newWorkspace} title={m.frame.newWorkspaceTitle(keysFor("new"))} onClick={() => (overlay.value = "new")}>
          <Icon name="plus" size={16} />
        </button>
        <a
          class="who"
          href="/settings"
          aria-label={m.frame.settings(who)}
          title={who}
          aria-current={view === "settings" ? "page" : undefined}
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
      {toasts.value.map((t) => {
        // A workspace deleted since has nothing to show; one renamed is
        // reached by its new name.
        const ws = workspaces.value.find((w) => w.id === t.workspace);
        if (!ws) return null;
        return (
          <a
            key={t.id}
            class="toast"
            href={`/w/${ws.name}`}
            onClick={() => (toasts.value = toasts.value.filter((x) => x.id !== t.id))}
          >
            <Igloo />
            <b>{t.title}</b>
            <span>{t.detail}</span>
          </a>
        );
      })}
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
          {m.frame.undo}
        </button>
      ) : null}
    </p>
  );
}

export function App() {
  const r = route.value;
  const loading = me.value === null || !loaded.value;
  return (
    <>
      <a class="skip" href="#stage">
        {m.frame.skip}
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
        ) : (
          <Overview />
        )}
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
