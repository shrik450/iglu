// One project: its workspaces, and how new ones start.

import { useEffect, useState } from "preact/hooks";

import { attempt } from "../actions.ts";
import { api } from "../api/client.ts";
import { Icon } from "../components/bits.tsx";
import { Card } from "../components/Card.tsx";
import { FieldError, FormError, InputError, invalid, textOf, useForm } from "../components/forms.tsx";
import type { ChangeProject } from "../generated/ChangeProject.ts";
import type { ColumnKind } from "../generated/ColumnKind.ts";
import type { ColumnTemplate } from "../generated/ColumnTemplate.ts";
import type { IdleRule } from "../generated/IdleRule.ts";
import type { ProjectView } from "../generated/ProjectView.ts";
import { m } from "../messages.tsx";
import { repoLabel } from "../state/groups.ts";
import { LABEL } from "../state/layout.ts";
import { parsePorts } from "../state/project.ts";
import { environments, navigate, newIn, overlay, projects, route, say, workspaces } from "../state/store.ts";
import { oneOf, unreachable } from "../state/unsaved.ts";

const SETTINGS = "project-settings";

/** The settings form's inputs: the request's fields, and the minutes an idle rule is read from. */
type SettingsInputs = ChangeProject & { minutes: string };

const IDLE: Readonly<Record<IdleRule["kind"], string>> = {
  Default: m.project.idle("Default"),
  Never: m.project.idle("Never"),
  After: m.project.idle("After"),
};

export function ProjectPage() {
  const r = route.value;
  const project = r.view === "project" ? projects.value.find((p) => p.name === r.name) : undefined;
  if (!project) {
    return (
      <div class="empty-state">
        <p class="empty">{m.project.missing(r.view === "project" ? r.name : null)}</p>
      </div>
    );
  }
  return <Project key={project.id} project={project} settings={r.view === "project" && r.settings === true} />;
}

/** A project's workspaces, then its settings; at its settings' address, the
 * page starts at them, below however many workspaces there are. */
function Project({ project, settings }: { project: ProjectView; settings: boolean }) {
  const inside = workspaces.value.filter((ws) => ws.project === project.id);
  useEffect(() => {
    if (!settings) return;
    const form = document.getElementById(SETTINGS);
    form?.scrollIntoView({ block: "start" });
    form?.querySelector<HTMLElement>("input, select")?.focus({ preventScroll: true });
  }, [settings]);
  return (
    <div class="project">
      <header class="p-head">
        <h1 translate={false}>{project.name}</h1>
        <span class="muted" translate={false}>
          {m.project.source(project.repo ? repoLabel(project.repo) : null, project.environment)}
        </span>
        <a class="btn" href={`/p/${project.name}/settings`}>
          {m.project.settings}
        </a>
        <button
          type="button"
          class="btn primary"
          onClick={() => {
            newIn.value = project.id;
            overlay.value = "new";
          }}
        >
          {m.project.newWorkspace}
        </button>
      </header>
      {inside.length ? (
        <div class="grid">
          {inside.map((ws) => (
            <Card key={ws.id} ws={ws} />
          ))}
        </div>
      ) : (
        <p class="muted">{m.project.empty}</p>
      )}
      <Draft project={project} />
      {project.origin === "added" ? <Remove project={project} live={inside.length} /> : null}
    </div>
  );
}

function idleChoice(rule: IdleRule): IdleRule["kind"] {
  return rule.kind;
}

/** The settings form edits the revision it opened on. A save elsewhere
 * replaces an untouched form; an edited one keeps the edits and says so,
 * since saving them would overwrite what was saved. */
function Draft({ project }: { project: ProjectView }) {
  const [base, setBase] = useState(project);
  const [edited, setEdited] = useState(false);
  const moved = base.revision !== project.revision;
  useEffect(() => {
    if (moved && !edited) setBase(project);
  }, [moved, edited, project]);
  const restart = () => {
    setBase(project);
    setEdited(false);
  };
  return <Settings key={base.revision} project={base} onEdit={() => setEdited(true)} onSaved={() => setEdited(false)} stale={moved && edited ? restart : null} />;
}

function Settings(props: { project: ProjectView; onEdit: () => void; onSaved: () => void; stale: (() => void) | null }) {
  const { project } = props;
  const [opening, setOpeningState] = useState<ColumnTemplate[]>(project.opening);
  const setOpening = (next: ColumnTemplate[]) => {
    setOpeningState(next);
    props.onEdit();
  };
  const [idle, setIdle] = useState<IdleRule["kind"]>(idleChoice(project.idle));
  const [envName, setEnvName] = useState(project.environment);
  const latest = environments.value.find((env) => env.name === envName)?.latest;
  // A workspace has one browser, and only from an image that has one.
  const browser = latest?.status === "Ready" && latest.image.browser && !opening.some((t) => t.kind.kind === "Browser");
  const move = (index: number, step: -1 | 1) => {
    const next = [...opening];
    const [item] = next.splice(index, 1);
    if (item) next.splice(Math.max(0, Math.min(next.length, index + step)), 0, item);
    setOpening(next);
  };
  const add = (kind: ColumnKind) => setOpening([...opening, { kind, width: "half" }]);
  const save = useForm<SettingsInputs>(async (data) => {
    const text = textOf(data);
    const name = text("name");
    if (!name) return;
    const ports = parsePorts(text("ports") ?? "");
    if ("notANumber" in ports) throw new InputError("ports", m.project.notAPort(ports.notANumber));
    let rule: IdleRule;
    switch (idle) {
      case "Default":
        rule = { kind: "Default" };
        break;
      case "Never":
        rule = { kind: "Never" };
        break;
      case "After": {
        const minutes = text("minutes") ?? "";
        if (!/^\d+$/.test(minutes)) throw new InputError("minutes", m.project.wholeMinutes);
        rule = { kind: "After", minutes: Number(minutes) };
        break;
      }
      default:
        return unreachable(idle);
    }
    await api.changeProject(project.id, {
      expected_revision: project.revision,
      name,
      repo: text("repo") ?? null,
      environment: envName,
      opening,
      ports: ports.ports,
      idle: rule,
    });
    props.onSaved();
    say(m.project.saved);
    if (name !== project.name) navigate({ view: "project", name }, "replace");
  }, m.project.rules);
  return (
    <form id={SETTINGS} class="set-form p-settings" aria-label={m.project.settingsFor(project.name)} onSubmit={save.onSubmit} onInput={props.onEdit} onChange={props.onEdit}>
      <h2>{m.project.settings}</h2>
      {/* While saving, a new revision may be this save's own, arriving before its answer. */}
      {props.stale && !save.busy ? (
        <p class="field-err stale" role="status">
          {m.project.stale}{" "}
          <button type="button" class="btn" onClick={props.stale}>
            {m.project.showSaved}
          </button>
        </p>
      ) : null}
      <label>
        {m.project.name}
        <input name="name" required defaultValue={project.name} autocomplete="off" spellcheck={false} {...invalid(save, "name")} />
        <FieldError form={save} input="name" />
      </label>
      <label class="grow">
        {m.project.repo}
        <input name="repo" defaultValue={project.repo ?? ""} placeholder={m.project.repoHint} autocomplete="off" spellcheck={false} {...invalid(save, "repo")} />
        <FieldError form={save} input="repo" />
      </label>
      <label>
        {m.project.environment}
        <select name="environment" value={envName} onChange={(e) => setEnvName(e.currentTarget.value)} {...invalid(save, "environment")}>
          {environments.value.map((env) => (
            <option key={env.name} value={env.name}>
              {env.name}
            </option>
          ))}
        </select>
        <FieldError form={save} input="environment" />
      </label>
      <label>
        {m.project.ports}
        <input name="ports" defaultValue={project.ports.join(", ")} placeholder={m.project.portsHint} inputMode="numeric" autocomplete="off" spellcheck={false} {...invalid(save, "ports")} />
        <FieldError form={save} input="ports" />
      </label>
      <label>
        {m.project.whenUnused}
        <select name="idle" value={idle} onChange={(e) => setIdle(oneOf(IDLE, e.currentTarget.value) ?? idle)} {...invalid(save, "idle")}>
          {Object.entries(IDLE).map(([kind, label]) => (
            <option key={kind} value={kind}>
              {label}
            </option>
          ))}
        </select>
        <FieldError form={save} input="idle" />
      </label>
      {idle === "After" ? (
        <label>
          {m.project.minutes}
          <input
            name="minutes"
            inputMode="numeric"
            required
            defaultValue={project.idle.kind === "After" ? String(project.idle.minutes) : "60"}
            autocomplete="off"
            {...invalid(save, "minutes")}
          />
          <FieldError form={save} input="minutes" />
        </label>
      ) : null}
      <fieldset class="opening" name="opening" {...invalid(save, "opening")}>
        <legend>{m.project.opening}</legend>
        <ol>
          {opening.map((t, i) => (
            <li key={`${i}-${t.name ?? ""}`}>
              <span translate={t.name ? false : undefined}>{t.name ?? m.columnKind(t.kind)}</span>
              <select
                aria-label={m.project.widthOf(t.name ?? m.columnKind(t.kind))}
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
              <button type="button" class="btn icon" aria-label={m.project.moveLeft} disabled={i === 0} onClick={() => move(i, -1)}>
                <Icon name="back" size={12} />
              </button>
              <button type="button" class="btn icon" aria-label={m.project.moveRight} disabled={i === opening.length - 1} onClick={() => move(i, 1)}>
                <Icon name="chevron" size={12} />
              </button>
              <button type="button" class="btn icon" aria-label={m.project.remove(t.name ?? m.columnKind(t.kind))} onClick={() => setOpening(opening.filter((_, j) => j !== i))}>
                <Icon name="close" size={12} />
              </button>
            </li>
          ))}
        </ol>
        <div class="opening-add">
          <button type="button" class="btn" onClick={() => add({ kind: "Shell" })}>
            <Icon name="plus" size={12} />
            {m.columnKind({ kind: "Shell" })}
          </button>
          {browser ? (
            <button type="button" class="btn" onClick={() => add({ kind: "Browser" })}>
              <Icon name="plus" size={12} />
              {m.columnKind({ kind: "Browser" })}
            </button>
          ) : null}
        </div>
        <FieldError form={save} input="opening" />
      </fieldset>
      <button type="submit" class="btn primary" disabled={save.busy}>
        {m.project.save(save.busy)}
      </button>
      <FormError form={save} />
    </form>
  );
}

function Remove({ project, live }: { project: ProjectView; live: number }) {
  const [sure, setSure] = useState(false);
  if (live > 0) return <p class="muted">{m.project.deleteFirst(project.name)}</p>;
  return sure ? (
    <div class="confirm">
      <span>{m.project.confirmRemove(project.name)}</span>
      <button
        type="button"
        class="btn danger"
        onClick={() =>
          void attempt(() => api.removeProject(project.id)).then((done) => {
            if (done) navigate({ view: "overview" });
          })
        }
      >
        {m.project.removeSure}
      </button>
      <button type="button" class="btn" onClick={() => setSure(false)}>
        {m.project.keepIt}
      </button>
    </div>
  ) : (
    <button type="button" class="btn danger" onClick={() => setSure(true)}>
      {m.project.removeProject}
    </button>
  );
}
