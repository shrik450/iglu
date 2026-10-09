// One project: its workspaces, and how new ones start.

import { useState } from "preact/hooks";

import { attempt } from "../actions.ts";
import { api, failure } from "../api/client.ts";
import { Card } from "../components/Card.tsx";
import { textOf } from "../components/forms.ts";
import type { ColumnKind } from "../generated/ColumnKind.ts";
import type { ColumnTemplate } from "../generated/ColumnTemplate.ts";
import type { IdleRule } from "../generated/IdleRule.ts";
import type { ProjectView } from "../generated/ProjectView.ts";
import { repoLabel } from "../state/groups.ts";
import { LABEL } from "../state/layout.ts";
import { parsePorts } from "../state/project.ts";
import { environments, navigate, newIn, overlay, projects, route, say, workspaces } from "../state/store.ts";
import { oneOf, unreachable } from "../state/unsaved.ts";

const IDLE: Readonly<Record<IdleRule["kind"], string>> = {
  default: "Freeze as usual",
  never: "Stay awake",
  after: "Freeze after…",
};

export function ProjectPage() {
  const r = route.value;
  const project = r.view === "project" ? projects.value.find((p) => p.name === r.name) : undefined;
  if (!project) {
    return (
      <div class="empty-state">
        <p class="empty">No project called {r.view === "project" ? r.name : "that"}.</p>
      </div>
    );
  }
  // Keyed so the form starts over when the project changes underneath it.
  return <Project key={`${project.id}/${project.revision}`} project={project} />;
}

function Project({ project }: { project: ProjectView }) {
  const inside = workspaces.value.filter((ws) => ws.project === project.id);
  return (
    <div class="project">
      <header class="p-head">
        <h1 translate={false}>{project.name}</h1>
        <span class="muted" translate={false}>
          {project.repo ? repoLabel(project.repo) : "no repository"} · {project.environment}
        </span>
        <button
          type="button"
          class="btn primary"
          onClick={() => {
            newIn.value = project.id;
            overlay.value = "new";
          }}
        >
          New workspace
        </button>
      </header>
      {inside.length ? (
        <div class="grid">
          {inside.map((ws) => (
            <Card key={ws.id} ws={ws} />
          ))}
        </div>
      ) : (
        <p class="muted">No workspaces yet.</p>
      )}
      <Settings project={project} />
      {project.origin === "added" ? <Remove project={project} live={inside.length} /> : null}
    </div>
  );
}

function kindText(kind: ColumnKind): string {
  switch (kind.kind) {
    case "shell":
      return "Shell";
    case "agent":
      return kind.agent;
    case "server":
      return kind.command.join(" ");
    default:
      return unreachable(kind);
  }
}

function idleChoice(rule: IdleRule): IdleRule["kind"] {
  return rule.kind;
}

function Settings({ project }: { project: ProjectView }) {
  const [opening, setOpening] = useState<ColumnTemplate[]>(project.opening);
  const [idle, setIdle] = useState<IdleRule["kind"]>(idleChoice(project.idle));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [envName, setEnvName] = useState(project.environment);
  const latest = environments.value.find((env) => env.name === envName)?.latest;
  const agents = latest?.status === "ready" ? latest.image.agents.map((a) => a.name) : [];
  const move = (index: number, step: -1 | 1) => {
    const next = [...opening];
    const [item] = next.splice(index, 1);
    if (item) next.splice(Math.max(0, Math.min(next.length, index + step)), 0, item);
    setOpening(next);
  };
  const add = (kind: ColumnKind) => setOpening([...opening, { kind, width: "half" }]);
  return (
    <form
      class="set-form p-settings"
      aria-label={`Settings for ${project.name}`}
      onSubmit={async (e) => {
        e.preventDefault();
        const data = new FormData(e.currentTarget);
        const text = textOf(data);
        const name = text("name");
        if (!name) return;
        const ports = parsePorts(text("ports") ?? "");
        if ("error" in ports) {
          setError(ports.error);
          return;
        }
        const minutes = Number(text("minutes"));
        let rule: IdleRule;
        switch (idle) {
          case "default":
            rule = { kind: "default" };
            break;
          case "never":
            rule = { kind: "never" };
            break;
          case "after":
            if (!Number.isInteger(minutes) || minutes < 1) {
              setError("Freeze after a whole number of minutes.");
              return;
            }
            rule = { kind: "after", minutes };
            break;
          default:
            return unreachable(idle);
        }
        setBusy(true);
        setError(null);
        try {
          await api.changeProject(project.id, {
            expected_revision: project.revision,
            name,
            repo: text("repo") ?? null,
            environment: envName,
            opening,
            agent: text("agent") ?? null,
            ports: ports.ports,
            idle: rule,
          });
          say("Saved.");
          if (name !== project.name) navigate({ view: "project", name }, "replace");
        } catch (err) {
          setError(failure(err));
        }
        setBusy(false);
      }}
    >
      <h2>Settings</h2>
      <label>
        Name
        <input name="name" required defaultValue={project.name} autocomplete="off" spellcheck={false} />
      </label>
      <label class="grow">
        Repository
        <input name="repo" defaultValue={project.repo ?? ""} placeholder="None…" autocomplete="off" spellcheck={false} />
      </label>
      <label>
        Environment
        <select name="environment" value={envName} onChange={(e) => setEnvName(e.currentTarget.value)}>
          {environments.value.map((env) => (
            <option key={env.name} value={env.name}>
              {env.name}
            </option>
          ))}
        </select>
      </label>
      <label>
        Starts
        <select name="agent" defaultValue={project.agent ?? ""}>
          <option value="">No agent</option>
          {agents.map((a) => (
            <option key={a} value={a}>
              {a}
            </option>
          ))}
        </select>
      </label>
      <label>
        Previews
        <input name="ports" defaultValue={project.ports.join(", ")} placeholder="3000, 5173…" inputMode="numeric" autocomplete="off" spellcheck={false} />
      </label>
      <label>
        When unused
        <select name="idle" value={idle} onChange={(e) => setIdle(oneOf(IDLE, e.currentTarget.value) ?? idle)}>
          {Object.entries(IDLE).map(([kind, label]) => (
            <option key={kind} value={kind}>
              {label}
            </option>
          ))}
        </select>
      </label>
      {idle === "after" ? (
        <label>
          Minutes
          <input
            name="minutes"
            type="number"
            min="1"
            required
            defaultValue={project.idle.kind === "after" ? String(project.idle.minutes) : "60"}
            autocomplete="off"
          />
        </label>
      ) : null}
      <fieldset class="opening">
        <legend>New workspaces open with</legend>
        <ol>
          {opening.map((t, i) => (
            <li key={`${i}-${t.name ?? ""}`}>
              <span translate={false}>{t.name ?? kindText(t.kind)}</span>
              <select
                aria-label={`Width of ${t.name ?? kindText(t.kind)}`}
                value={t.width}
                onChange={(e) => {
                  const width = oneOf(LABEL, e.currentTarget.value);
                  if (width) setOpening(opening.map((o, j) => (j === i ? { ...o, width } : o)));
                }}
              >
                {Object.entries(LABEL).map(([w, label]) => (
                  <option key={w} value={w}>
                    {label}
                  </option>
                ))}
              </select>
              <button type="button" class="btn icon" aria-label="Move left" disabled={i === 0} onClick={() => move(i, -1)}>
                ‹
              </button>
              <button type="button" class="btn icon" aria-label="Move right" disabled={i === opening.length - 1} onClick={() => move(i, 1)}>
                ›
              </button>
              <button type="button" class="btn icon" aria-label={`Remove ${t.name ?? kindText(t.kind)}`} onClick={() => setOpening(opening.filter((_, j) => j !== i))}>
                ×
              </button>
            </li>
          ))}
        </ol>
        <div class="opening-add">
          <button type="button" class="btn" onClick={() => add({ kind: "shell" })}>
            + Shell
          </button>
          {agents.map((agent) => (
            <button key={agent} type="button" class="btn" onClick={() => add({ kind: "agent", agent })}>
              + {agent}
            </button>
          ))}
        </div>
      </fieldset>
      <button type="submit" class="btn primary" disabled={busy}>
        {busy ? "Saving…" : "Save"}
      </button>
      {error ? (
        <p class="field-err" role="alert">
          {error}
        </p>
      ) : null}
    </form>
  );
}

function Remove({ project, live }: { project: ProjectView; live: number }) {
  const [sure, setSure] = useState(false);
  if (live > 0) return <p class="muted">Delete its workspaces to remove {project.name}.</p>;
  return sure ? (
    <div class="confirm">
      <span>Remove {project.name}? Its settings go; nothing else does.</span>
      <button
        type="button"
        class="btn danger"
        onClick={() =>
          void attempt(async () => {
            await api.removeProject(project.id);
            return true;
          }).then((done) => {
            if (done) navigate({ view: "overview" });
          })
        }
      >
        Remove
      </button>
      <button type="button" class="btn" onClick={() => setSure(false)}>
        Keep it
      </button>
    </div>
  ) : (
    <button type="button" class="btn danger" onClick={() => setSure(true)}>
      Remove project
    </button>
  );
}
