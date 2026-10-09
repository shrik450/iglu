// Creating a workspace, in a project.

import { useState } from "preact/hooks";

import { create } from "../actions.ts";
import type { CreateWorkspace } from "../generated/CreateWorkspace.ts";
import { startingAgent } from "../state/project.ts";
import { environments, newIn, overlay, projects } from "../state/store.ts";
import { FieldError, FormError, invalid, textOf, useForm } from "./forms.tsx";
import { Modal } from "./Modal.tsx";

export function NewWorkspace() {
  const [chosen, setChosen] = useState(newIn.value ?? projects.value[0]?.id ?? "");
  const project = projects.value.find((p) => p.id === chosen) ?? null;
  const latest = environments.value.find((env) => env.name === project?.environment)?.latest;
  const agents = latest?.status === "ready" ? latest.image.agents.map((a) => a.name) : [];
  const preferred = project ? startingAgent(project, agents) : null;
  // What the person picked, while it's still one this project's environment has.
  const [picked, setPicked] = useState<string | null>(null);
  const agent = picked && agents.includes(picked) ? picked : preferred;
  const close = () => {
    overlay.value = null;
    newIn.value = null;
  };
  const start = useForm(async (data) => {
    if (!project) return;
    const text = textOf(data);
    const body: CreateWorkspace = { project: project.id };
    const prompt = text("prompt");
    if (prompt) body.prompt = prompt;
    // Left out unless picked, so iglu chooses as the project says.
    if (prompt && picked && agent) body.agent = agent;
    const name = text("name");
    const branch = text("branch");
    const base = text("base");
    if (name) body.name = name;
    if (branch) body.branch = branch;
    if (base) body.base = base;
    await create(body);
  });
  return (
    <Modal labelledby="new-h" onClose={close}>
      <form class="nbox" aria-labelledby="new-h" onSubmit={start.onSubmit}>
        <h2 id="new-h">New workspace</h2>
        {projects.value.length === 0 ? <p class="field-err">Add an environment first, in <a href="/settings">Settings</a>.</p> : null}
        <label>
          Prompt
          <textarea
            name="prompt"
            rows={3}
            placeholder={agent ? `What should ${agent} do? Optional…` : "No agents in this environment…"}
            disabled={agents.length === 0}
            autofocus
            {...invalid(start, "prompt")}
            onKeyDown={(e) => {
              // ⌘↩ submits from the prompt, which takes plain returns.
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) e.currentTarget.form?.requestSubmit();
            }}
          />
          <FieldError form={start} input="prompt" />
        </label>
        {agents.length > 1 ? (
          <label>
            Agent
            <select name="agent" value={agent ?? ""} onChange={(e) => setPicked(e.currentTarget.value)} {...invalid(start, "agent")}>
              {agents.map((a) => (
                <option key={a} value={a}>
                  {a}
                </option>
              ))}
            </select>
            <FieldError form={start} input="agent" />
          </label>
        ) : null}
        <div class="two">
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
          <label>
            Name
            <input name="name" placeholder="Picked for you…" autocomplete="off" spellcheck={false} {...invalid(start, "name")} />
            <FieldError form={start} input="name" />
          </label>
        </div>
        {project?.repo ? (
          <details>
            <summary>Branch</summary>
            <div class="fine">
              <label>
                Branch
                <input name="branch" placeholder="Named after the workspace…" autocomplete="off" spellcheck={false} {...invalid(start, "branch")} />
                <FieldError form={start} input="branch" />
              </label>
              <label>
                Start from
                <input name="base" placeholder="The default branch…" autocomplete="off" spellcheck={false} {...invalid(start, "base")} />
                <FieldError form={start} input="base" />
              </label>
            </div>
          </details>
        ) : null}
        <FormError form={start} />
        <div class="nbtns">
          <button type="button" class="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" class="btn primary" disabled={start.busy || !project}>
            {start.busy ? "Creating…" : "Create"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
