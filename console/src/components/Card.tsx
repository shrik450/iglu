// A workspace's tile in the overview.

import type { VNode } from "preact";

import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { runs, situation } from "../state/situation.ts";
import { cursor } from "../state/store.ts";
import { unreachable } from "../state/unsaved.ts";
import { Previews } from "./Previews.tsx";
import { attentionGlyph, attentionText, buildRows, Frost, Glyph, Igloo, lookOf, phaseText, WorkspaceLink, workspaceGlyph } from "./bits.tsx";

/** The top thread, and one more for a sense of what else is going on. */
function Threads({ ws }: { ws: WorkspaceView }) {
  const shown = ws.threads.slice(0, 2);
  if (shown.length === 0) {
    return ws.columns.length ? (
      <div class="runs" translate={false}>
        {runs(ws.columns)}
      </div>
    ) : null;
  }
  const more = ws.threads.length - shown.length;
  return (
    <>
      {shown.map((a) => {
        const state = attentionText(a);
        return (
          <div class="tl" key={`${a.session}/${a.thread}`}>
            <Glyph kind={attentionGlyph(a)} />
            <span>{a.summary || a.title || a.session}</span>
            <span class={`state ${state.tone}`}>{state.text}</span>
          </div>
        );
      })}
      {more > 0 ? <div class="note">+{more} more</div> : null}
    </>
  );
}

function Body({ ws }: { ws: WorkspaceView }): VNode {
  const now = situation(ws);
  switch (now.kind) {
    case "building":
      return (
        <div class="c-build">
          <Igloo rows={buildRows(ws.phase)} />
          <span>{now.label}</span>
        </div>
      );
    case "stuck":
    case "broken":
    case "held":
      return (
        <div class={`c-alert ${now.kind === "held" ? "calm" : "trouble"}`}>
          <b>{now.title}</b>
          <span title={now.detail}>{now.detail}</span>
        </div>
      );
    case "stopped":
      return <div class="note">Its files are kept; its processes ended.</div>;
    case "leaving":
      return <div class="note">{now.label}</div>;
    case "frozen":
    case "running":
      return <Threads ws={ws} />;
    default:
      return unreachable(now);
  }
}

export function Card({ ws }: { ws: WorkspaceView }) {
  const look = lookOf(ws.phase);
  const frozen = look === "frozen";
  const phase = phaseText(ws.phase);
  // The branch is worth showing only when it isn't the workspace's own name.
  const branch = ws.checkout && ws.checkout.branch !== ws.name ? ws.checkout.branch : null;
  const classes = ["card", ws.needs_you ? "needs" : "", frozen ? "is-frozen" : "", look === "asleep" ? "asleep" : "", cursor.value === ws.id ? "sel" : ""];
  return (
    <article class={classes.filter(Boolean).join(" ")} data-name={ws.name}>
      <header class="c-head">
        <Glyph kind={workspaceGlyph(ws)} />
        <WorkspaceLink ws={ws} class="name">
          <span translate={false}>{ws.name}</span>
        </WorkspaceLink>
        {branch ? (
          <span class="branch" translate={false}>
            ⎇ {branch}
          </span>
        ) : null}
        {phase ? <span class="state">{phase}</span> : null}
      </header>
      <div class="c-lines">
        <Body ws={ws} />
      </div>
      {ws.routes.length ? (
        <footer class="c-ports">
          <Previews ws={ws} />
        </footer>
      ) : null}
      {frozen ? <Frost label={ws.phase === "freezing" ? "Freezing…" : "Frozen"} hint="Opening it thaws it" /> : null}
    </article>
  );
}
