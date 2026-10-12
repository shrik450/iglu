// Creating a workspace, in a project: a branch to work on, and what to call
// it. It opens with the project's columns.

import { useState } from "preact/hooks";

import { create } from "../actions.ts";
import type { CreateWorkspace } from "../generated/CreateWorkspace.ts";
import { nameFrom, nameFromBranch } from "../state/naming.ts";
import { opensWith } from "../state/project.ts";
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
  const opens = project ? opensWith(project) : [];
  const close = () => {
    overlay.value = null;
    newIn.value = null;
  };
  const start = useForm(async (data) => {
    if (!project) return;
    const text = textOf(data);
    const body: CreateWorkspace = { project: project.id };
    const [label, name, branch, base] = [text("label"), text("name"), text("branch"), text("base")];
    if (label) body.label = label;
    if (name) body.name = name;
    if (repo && branch) body.branch = branch;
    if (repo && base) body.base = base;
    await create(body);
  });
  return (
    <Modal labelledby="new-h" onClose={close}>
      <form class="nbox" aria-labelledby="new-h" onSubmit={start.onSubmit}>
        <DialogHead id="new-h" title="New workspace" onClose={close} />
        {projects.value.length === 0 ? <p class="field-err">Add an environment first, in <a href="/settings">Settings</a>.</p> : null}
        {projects.value.length > 1 ? (
          <label>
            Project
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
            Branch
            <input
              name="branch"
              placeholder="A new branch, or one the remote has…"
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
          Call it
          <input
            name="label"
            placeholder={repo ? "Anything; the branch names it otherwise…" : "Anything, such as Tidy my notes…"}
            autocomplete="off"
            autofocus={!repo}
            value={label}
            onInput={(e) => setLabel(e.currentTarget.value)}
            {...invalid(start, "label")}
          />
          <FieldError form={start} input="label" />
        </label>
        <details>
          <summary>More</summary>
          <div class="fine">
            <label>
              Name, in URLs and the CLI
              <input name="name" placeholder={made ? `${made}…` : "Picked for you…"} autocomplete="off" spellcheck={false} {...invalid(start, "name")} />
              <FieldError form={start} input="name" />
            </label>
            {repo ? (
              <label>
                A new branch starts from
                <input name="base" placeholder="The default branch…" autocomplete="off" spellcheck={false} {...invalid(start, "base")} />
                <FieldError form={start} input="base" />
              </label>
            ) : null}
          </div>
        </details>
        {opens.length ? <p class="fine-print">Opens with {opens.join(", ")}.</p> : null}
        <FormError form={start} />
        <div class="nbtns">
          {/* Once asked for, the workspace is made either way; closing only stops waiting for it. */}
          <button type="button" class="btn" onClick={close}>
            {start.busy ? "Close" : "Cancel"}
          </button>
          <button type="submit" class="btn primary" disabled={start.busy || !project}>
            {start.busy ? "Creating…" : "Create"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
