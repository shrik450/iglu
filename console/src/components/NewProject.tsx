// Adding a project, then starting its first workspace.

import { useState } from "preact/hooks";

import { api, failure } from "../api/client.ts";
import type { CreateProject } from "../generated/CreateProject.ts";
import { environments, newIn, overlay } from "../state/store.ts";
import { textOf } from "./forms.ts";

export function NewProject() {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const close = () => (overlay.value = null);
  return (
    <div class="overlay" onClick={(e) => e.target === e.currentTarget && close()}>
      <form
        class="nbox"
        aria-labelledby="project-h"
        onKeyDown={(e) => e.key === "Escape" && close()}
        onSubmit={async (e) => {
          e.preventDefault();
          const text = textOf(new FormData(e.currentTarget));
          const environment = text("environment");
          if (!environment) return;
          const body: CreateProject = { environment };
          const repo = text("repo");
          const name = text("name");
          if (repo) body.repo = repo;
          if (name) body.name = name;
          setBusy(true);
          try {
            const added = await api.createProject(body);
            newIn.value = added.id;
            overlay.value = "new";
          } catch (err) {
            setError(failure(err));
          }
          setBusy(false);
        }}
      >
        <h2 id="project-h">New project</h2>
        {environments.value.length === 0 ? <p class="field-err">Add an environment first, in <a href="/settings">Settings</a>.</p> : null}
        <label>
          Repository
          <input name="repo" inputMode="url" placeholder="https://github.com/you/app.git…" autocomplete="off" spellcheck={false} autoFocus />
        </label>
        <div class="two">
          <label>
            Name
            <input name="name" placeholder="Named after the repository…" autocomplete="off" spellcheck={false} />
          </label>
          <label>
            Environment
            <select name="environment" required>
              {environments.value.map((env) => (
                <option key={env.name} value={env.name}>
                  {env.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        {error ? (
          <p class="field-err" role="alert">
            {error}
          </p>
        ) : null}
        <div class="nbtns">
          <button type="button" class="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" class="btn primary" disabled={busy || environments.value.length === 0}>
            {busy ? "Adding…" : "Add"}
          </button>
        </div>
      </form>
    </div>
  );
}
