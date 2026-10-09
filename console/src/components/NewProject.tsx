// Adding a project, then starting its first workspace.

import { api } from "../api/client.ts";
import type { CreateProject } from "../generated/CreateProject.ts";
import { environments, newIn, overlay } from "../state/store.ts";
import { FieldError, FormError, invalid, textOf, useForm } from "./forms.tsx";

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
  });
  return (
    <div class="overlay" onClick={(e) => e.target === e.currentTarget && close()}>
      <form
        class="nbox"
        aria-labelledby="project-h"
        onKeyDown={(e) => e.key === "Escape" && close()}
        onSubmit={add.onSubmit}
      >
        <h2 id="project-h">New project</h2>
        {environments.value.length === 0 ? <p class="field-err">Add an environment first, in <a href="/settings">Settings</a>.</p> : null}
        <label>
          Repository
          <input name="repo" inputMode="url" placeholder="https://github.com/you/app.git…" autocomplete="off" spellcheck={false} autoFocus {...invalid(add, "repo")} />
          <FieldError form={add} input="repo" />
        </label>
        <div class="two">
          <label>
            Name
            <input name="name" placeholder="Named after the repository…" autocomplete="off" spellcheck={false} {...invalid(add, "name")} />
            <FieldError form={add} input="name" />
          </label>
          <label>
            Environment
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
            Cancel
          </button>
          <button type="submit" class="btn primary" disabled={add.busy || environments.value.length === 0}>
            {add.busy ? "Adding…" : "Add"}
          </button>
        </div>
      </form>
    </div>
  );
}
