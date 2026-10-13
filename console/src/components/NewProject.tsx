// Adding a project, then starting its first workspace.

import { api } from "../api/client.ts";
import type { CreateProject } from "../generated/CreateProject.ts";
import { m } from "../messages.tsx";
import { environments, newIn, overlay } from "../state/store.ts";
import { FieldError, FormError, invalid, textOf, useForm } from "./forms.tsx";
import { DialogHead, Modal } from "./Modal.tsx";

export function NewProject() {
  const close = () => (overlay.value = null);
  const add = useForm(async (data) => {
    const text = textOf(data);
    const environment = text("environment");
    if (!environment) return;
    const body: CreateProject = { environment };
    const repo = text("repo");
    const name = text("name");
    if (repo) body.repo = repo;
    if (name) body.name = name;
    const added = await api.createProject(body);
    newIn.value = added.id;
    overlay.value = "new";
  }, m.newProject.rules);
  return (
    <Modal labelledby="project-h" onClose={close}>
      <form class="nbox" aria-labelledby="project-h" onSubmit={add.onSubmit}>
        <DialogHead id="project-h" title={m.newProject.title} onClose={close} />
        {environments.value.length === 0 ? <p class="field-err">{m.newProject.noEnvironment((text) => <a href="/settings">{text}</a>)}</p> : null}
        <label>
          {m.newProject.repo}
          <input name="repo" inputMode="url" placeholder={m.newProject.repoHint} autocomplete="off" spellcheck={false} autofocus {...invalid(add, "repo")} />
          <FieldError form={add} input="repo" />
        </label>
        <div class="two">
          <label>
            {m.newProject.name}
            <input name="name" placeholder={m.newProject.nameHint} autocomplete="off" spellcheck={false} {...invalid(add, "name")} />
            <FieldError form={add} input="name" />
          </label>
          <label>
            {m.newProject.environment}
            <select name="environment" required {...invalid(add, "environment")}>
              {environments.value.map((env) => (
                <option key={env.name} value={env.name}>
                  {env.name}
                </option>
              ))}
            </select>
            <FieldError form={add} input="environment" />
          </label>
        </div>
        <FormError form={add} />
        <div class="nbtns">
          <button type="button" class="btn" onClick={close}>
            {m.newProject.cancel}
          </button>
          <button type="submit" class="btn primary" disabled={add.busy || environments.value.length === 0}>
            {m.newProject.add(add.busy)}
          </button>
        </div>
      </form>
    </Modal>
  );
}
