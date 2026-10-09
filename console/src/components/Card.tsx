// A workspace's tile in the overview.

import type { WorkspaceView } from "../generated/WorkspaceView.ts";
import { cursor } from "../state/store.ts";
import { Previews } from "./Previews.tsx";
import { attentionGlyph, attentionText, buildRows, conditionText, Frost, Glyph, Igloo, inTrouble, lookOf, phaseText, WorkspaceLink, workspaceGlyph } from "./bits.tsx";

export function Card({ ws }: { ws: WorkspaceView }) {
  const look = lookOf(ws.phase);
  const frozen = look === "frozen";
  const asleep = look === "asleep";
  const building = look === "building";
  const phase = phaseText(ws.phase);
  // The top thread is the first; one more gives a sense of what else is going on.
  const others = ws.threads.slice(1, 2);
  const classes = ["card", ws.needs_you ? "needs" : "", frozen ? "is-frozen" : "", asleep ? "asleep" : "", cursor.value === ws.id ? "sel" : ""];
  return (
    <article class={classes.filter(Boolean).join(" ")} data-name={ws.name}>
      <header class="c-head">
        <Glyph kind={workspaceGlyph(ws)} />
        <WorkspaceLink ws={ws} class="name">
          <span translate={false}>{ws.name}</span>
        </WorkspaceLink>
        {ws.checkout ? (
          <span class="branch" translate={false}>
            ⎇ {ws.checkout.branch}
          </span>
        ) : null}
        {phase ? <span class="state">{phase}</span> : null}
      </header>
      <div class="c-lines">
        {ws.condition ? <div class={inTrouble(ws.condition) ? "trouble" : "note"}>{conditionText(ws.condition)}</div> : null}
        {building ? (
          <div class="building">
            <Igloo rows={buildRows(ws.phase)} />
          </div>
        ) : (
          [ws.attention, ...others].flatMap((a) => {
            if (!a) return [];
            const state = attentionText(a);
            return [
              <div class="tl" key={`${a.session}/${a.thread}`}>
                <Glyph kind={attentionGlyph(a)} />
                <span>{a.summary || a.title || a.session}</span>
                <span class={`state ${state.tone}`}>{state.text}</span>
              </div>,
            ];
          })
        )}
      </div>
      <div />
      <footer class="c-ports">
        <Previews ws={ws} />
      </footer>
      {frozen ? <Frost label={ws.phase === "freezing" ? "Freezing…" : "Frozen"} /> : null}
    </article>
  );
}
