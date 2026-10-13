// Creating a workspace, in a project: a branch to work on, and what to call
// it. It opens with the project's columns.

import { useState } from "preact/hooks";

import { create } from "../actions.ts";
import type { CreateWorkspace } from "../generated/CreateWorkspace.ts";
import { m } from "../messages.tsx";
import { nameFrom, nameFromBranch } from "../state/naming.ts";
import { current, newIn, overlay, projects, route } from "../state/store.ts";
import { FieldError, FormError, invalid, textOf, useForm } from "./forms.tsx";
import { DialogHead, Modal } from "./Modal.tsx";

export function NewWorkspace() {
  // The project asked for, else the one being looked at, else the first.
  const r = route.value;
  const here = newIn.value ?? current.value?.project ?? (r.view === "project" ? projects.value.find((p) => p.name === r.name)?.id : undefined);
  const [chosen, setChosen] = useState(here ?? projects.value[0]?.id ?? "");
  const project = projects.value.find((p) => p.id === chosen) ?? null;
  const [label, setLabel] = useState("");
  const [branch, setBranch] = useState("");
  const repo = Boolean(project?.repo);
  // The name iglu will make, shown until one is typed.
  const made = nameFrom(label) ?? (repo ? nameFromBranch(branch) : null);
  const opens = project ? project.opening.map((t) => t.name ?? m.columnKind(t.kind)) : [];
  const close = () => {
    overlay.value = null;
    newIn.value = null;
  };
  const start = useForm<CreateWorkspace>(async (data) => {
    if (!project) return;
    const text = textOf(data);
    const body: CreateWorkspace = { project: project.id };
    const [label, name, branch, base] = [text("label"), text("name"), text("branch"), text("base")];
    if (label) body.label = label;
    if (name) body.name = name;
    if (repo && branch) body.branch = branch;
    if (repo && base) body.base = base;
    await create(body);
  }, m.newWorkspace.rules);
  return (
    <Modal labelledby="new-h" onClose={close}>
      <form class="nbox" aria-labelledby="new-h" onSubmit={start.onSubmit}>
        <DialogHead id="new-h" title={m.newWorkspace.title} onClose={close} />
        {projects.value.length === 0 ? <p class="field-err">{m.newWorkspace.noEnvironment((text) => <a href="/settings">{text}</a>)}</p> : null}
        {projects.value.length > 1 ? (
          <label>
            {m.newWorkspace.project}
            <select name="project" value={chosen} onChange={(e) => setChosen(e.currentTarget.value)} {...invalid(start, "project")}>
              {projects.value.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
            <FieldError form={start} input="project" />
          </label>
        ) : null}
        {repo ? (
          <label>
            {m.newWorkspace.branch}
            <input
              name="branch"
              placeholder={m.newWorkspace.branchHint}
              autocomplete="off"
              spellcheck={false}
              autofocus
              value={branch}
              onInput={(e) => setBranch(e.currentTarget.value)}
              {...invalid(start, "branch")}
            />
            <FieldError form={start} input="branch" />
          </label>
        ) : null}
        <label>
          {m.newWorkspace.label}
          <input
            name="label"
            placeholder={m.newWorkspace.labelHint(repo)}
            autocomplete="off"
            autofocus={!repo}
            value={label}
            onInput={(e) => setLabel(e.currentTarget.value)}
            {...invalid(start, "label")}
          />
          <FieldError form={start} input="label" />
        </label>
        <details>
          <summary>{m.newWorkspace.more}</summary>
          <div class="fine">
            <label>
              {m.newWorkspace.name}
              <input name="name" placeholder={m.newWorkspace.nameHint(made)} autocomplete="off" spellcheck={false} {...invalid(start, "name")} />
              <FieldError form={start} input="name" />
            </label>
            {repo ? (
              <label>
                {m.newWorkspace.base}
                <input name="base" placeholder={m.newWorkspace.baseHint} autocomplete="off" spellcheck={false} {...invalid(start, "base")} />
                <FieldError form={start} input="base" />
              </label>
            ) : null}
          </div>
        </details>
        {opens.length ? <p class="fine-print">{m.newWorkspace.opens(opens)}</p> : null}
        <FormError form={start} />
        <div class="nbtns">
          {/* Once asked for, the workspace is made either way; closing only stops waiting for it. */}
          <button type="button" class="btn" onClick={close}>
            {m.newWorkspace.cancel(start.busy)}
          </button>
          <button type="submit" class="btn primary" disabled={start.busy || !project}>
            {m.newWorkspace.create(start.busy)}
          </button>
        </div>
      </form>
    </Modal>
  );
}
