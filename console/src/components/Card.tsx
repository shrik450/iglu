// A workspace's tile in the overview.

import type { VNode } from "preact";
import { useEffect, useRef } from "preact/hooks";

import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { m } from "../messages.tsx";
import { situation } from "../state/situation.ts";
import { cursor } from "../state/store.ts";
import { unreachable } from "../state/unsaved.ts";
import { Previews } from "./Previews.tsx";
import { attentionGlyph, attentionText, buildRows, Frost, Glyph, Igloo, lookOf, phaseText, WorkspaceLink, workspaceGlyph } from "./bits.tsx";
import { called } from "../state/naming.ts";

/** The top thread, and one more for a sense of what else is going on. */
function Threads({ ws }: { ws: WorkspaceView }) {
  const shown = ws.threads.slice(0, 2);
  if (shown.length === 0) {
    return ws.columns.length ? (
      <div class="runs" translate={false}>
        {m.card.runs(ws.columns.map((c) => c.name))}
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
            <span>{a.summary || a.title || m.card.inSession(a.session)}</span>
            <span class={`state ${state.tone}`}>{state.text}</span>
          </div>
        );
      })}
      {more > 0 ? <div class="note">{m.card.more(more)}</div> : null}
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
          <span>{m.situation.resting(now)}</span>
        </div>
      );
    case "stuck":
    case "broken":
    case "held": {
      const { title, detail } = m.situation.trouble(now);
      return (
        <div class={`c-alert ${now.kind === "held" ? "calm" : "trouble"}`}>
          <b>{title}</b>
          <span title={detail}>{detail}</span>
        </div>
      );
    }
    case "stopped":
      return <div class="note">{m.card.stopped}</div>;
    case "leaving":
      return <div class="note">{m.situation.resting(now)}</div>;
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
  const selected = cursor.value === ws.id;
  const ref = useRef<HTMLElement>(null);
  // The cursor stays in sight as j and k move it.
  useEffect(() => {
    if (selected) ref.current?.scrollIntoView({ block: "nearest" });
  }, [selected]);
  const classes = ["card", ws.needs_you ? "needs" : "", frozen ? "is-frozen" : "", look === "asleep" ? "asleep" : "", selected ? "sel" : ""];
  return (
    <article ref={ref} class={classes.filter(Boolean).join(" ")} data-name={ws.name}>
      <header class="c-head">
        <Glyph kind={workspaceGlyph(ws)} />
        <WorkspaceLink ws={ws} class="name">
          <span translate={false}>{called(ws)}</span>
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
      {frozen ? <Frost label={m.situation.resting({ kind: "frozen", freezing: ws.phase === "freezing" })} hint={m.card.thaws} /> : null}
    </article>
  );
}
