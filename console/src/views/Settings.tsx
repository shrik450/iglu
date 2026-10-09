// Everything set up once: environments, projects, secrets and the account.

import { useEffect, useState } from "preact/hooks";

import { attempt } from "../actions.ts";
import { api } from "../api/client.ts";
import { AddEnvironment } from "../components/AddEnvironment.tsx";
import { type Form, FieldError, FormError, invalid, textOf, useForm } from "../components/forms.tsx";
import { keyboard, mac, setKeyboard } from "../keyboard.ts";
import { enableNotifications } from "../notify.ts";
import type { EnvironmentView } from "../generated/EnvironmentView.ts";
import type { ProjectView } from "../generated/ProjectView.ts";
import type { SecretTarget } from "../generated/SecretTarget.ts";
import type { SecretView } from "../generated/SecretView.ts";
import { repoLabel } from "../state/groups.ts";
import { type Chord, chordLabel, usablePrefix } from "../state/keys.ts";
import type { OptionAsMeta } from "../state/prefs.ts";
import { environments, look, me, navigate, projects, say, workspaces } from "../state/store.ts";
import { oneOf, unreachable } from "../state/unsaved.ts";

const when = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

export function Settings() {
  return (
    <div class="settings">
      <h1 class="sr-only">Settings</h1>
      <Environments />
      <Projects />
      <Secrets />
      <Keyboard />
      <Account />
    </div>
  );
}

function buildText(env: EnvironmentView): string {
  const latest = env.latest;
  if (!latest) return "never built";
  switch (latest.status) {
    case "building":
      return "building…";
    case "ready":
      return `built ${when.format(latest.created_at)}`;
    case "failed":
      return "build failed";
  }
}

function Environments() {
  return (
    <section class="set" aria-labelledby="envs-h">
      <h2 id="envs-h">Environments</h2>
      <ul class="set-list">
        {environments.value.map((env) => (
          <li key={env.name}>
            <div class="set-row">
              <b translate={false}>{env.name}</b>
              <code translate={false}>{env.source}</code>
              <span class={`state${env.latest?.status === "failed" ? " s-trouble" : ""}`}>{buildText(env)}</span>
              <button type="button" class="btn" disabled={env.latest?.status === "building"} onClick={() => void attempt(() => api.buildEnvironment(env.name))}>
                Rebuild
              </button>
            </div>
            {env.latest?.status === "failed" ? (
              <details class="log">
                <summary>Build log</summary>
                <pre translate={false}>{env.latest.log_tail}</pre>
              </details>
            ) : null}
          </li>
        ))}
      </ul>
      <AddEnvironment />
    </section>
  );
}

function Projects() {
  return (
    <section class="set" aria-labelledby="projects-h">
      <h2 id="projects-h">Projects</h2>
      <ul class="set-list">
        {projects.value.map((p) => (
          <ProjectRow key={p.id} project={p} />
        ))}
      </ul>
    </section>
  );
}

function ProjectRow({ project }: { project: ProjectView }) {
  const live = workspaces.value.filter((ws) => ws.project === project.id).length;
  return (
    <li>
      <div class="set-row">
        <a
          href={`/p/${project.name}`}
          translate={false}
          onClick={(e) => {
            if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
            e.preventDefault();
            navigate({ view: "project", name: project.name });
          }}
        >
          <b>{project.name}</b>
        </a>
        <span class="muted" translate={false}>
          {project.repo ? repoLabel(project.repo) : "no repository"} · {project.environment}
        </span>
        <span class="state">{live === 1 ? "1 workspace" : `${live} workspaces`}</span>
      </div>
    </li>
  );
}

type TargetKind = SecretTarget["kind"];

const KINDS: Readonly<Record<TargetKind, string>> = {
  env: "Environment variable",
  file: "File in home",
  git_credential: "Git credential",
};

function targetText(target: SecretTarget): string {
  switch (target.kind) {
    case "env":
      return `$${target.name}`;
    case "file":
      return `~/${target.path}`;
    case "git_credential":
      return `git ${target.username}@${target.host}`;
  }
}

function targetOf(kind: TargetKind, text: (key: string) => string | undefined): SecretTarget | null {
  switch (kind) {
    case "env": {
      const name = text("env");
      return name ? { kind, name } : null;
    }
    case "file": {
      const path = text("path");
      return path ? { kind, path } : null;
    }
    case "git_credential": {
      const host = text("host");
      const username = text("username");
      return host && username ? { kind, host, username } : null;
    }
  }
}

/** The inputs for where a secret goes. iglu names them all `target`, and so
 * does the form's kind select; the error shows by the last of them. */
function TargetFields({ kind, form }: { kind: TargetKind; form: Form }) {
  switch (kind) {
    case "env":
      return (
        <label>
          Variable
          <input name="env" required placeholder="CLAUDE_CODE_OAUTH_TOKEN…" autocomplete="off" spellcheck={false} {...invalid(form, "target")} />
          <FieldError form={form} input="target" />
        </label>
      );
    case "file":
      return (
        <label>
          Path
          <input name="path" required placeholder=".ssh/id_ed25519…" autocomplete="off" spellcheck={false} {...invalid(form, "target")} />
          <FieldError form={form} input="target" />
        </label>
      );
    case "git_credential":
      return (
        <>
          <label>
            Host
            <input name="host" required placeholder="github.com…" autocomplete="off" spellcheck={false} {...invalid(form, "target")} />
          </label>
          <label>
            Username
            <input name="username" required placeholder="x-access-token…" autocomplete="off" spellcheck={false} {...invalid(form, "target")} />
            <FieldError form={form} input="target" />
          </label>
        </>
      );
    default:
      return unreachable(kind);
  }
}

function Secrets() {
  const [secrets, setSecrets] = useState<SecretView[] | null>(null);
  const [kind, setKind] = useState<TargetKind>("env");
  const [removing, setRemoving] = useState<string | null>(null);
  const load = async () => setSecrets((await attempt(() => api.secrets())) ?? []);
  useEffect(() => void load(), []);
  const add = useForm(async (data, form) => {
    const text = textOf(data);
    const name = text("name");
    const value = data.get("value");
    const target = targetOf(kind, text);
    if (!name || !target || typeof value !== "string" || value === "") return;
    await api.putSecret(name, { target, value });
    form.reset();
    await load();
    say(`Saved ${name}. New terminals get it.`);
  });
  return (
    <section class="set" aria-labelledby="secrets-h">
      <h2 id="secrets-h">Secrets</h2>
      <p class="muted">Every workspace you own gets these. Values are never shown again.</p>
      <ul class="set-list">
        {(secrets ?? []).map((secret) => (
          <li key={secret.id}>
            <div class="set-row">
              <b translate={false}>{secret.name}</b>
              <code translate={false}>{targetText(secret.target)}</code>
              <span class="state">{when.format(secret.updated_at)}</span>
              <span class="set-actions">
                {removing === secret.name ? (
                  <>
                    <button
                      type="button"
                      class="btn danger"
                      onClick={() =>
                        void attempt(() => api.deleteSecret(secret.name)).then(() => {
                          setRemoving(null);
                          void load();
                        })
                      }
                    >
                      Delete {secret.name}
                    </button>
                    <button type="button" class="btn" onClick={() => setRemoving(null)}>
                      Keep
                    </button>
                  </>
                ) : (
                  <button type="button" class="btn" onClick={() => setRemoving(secret.name)}>
                    Delete
                  </button>
                )}
              </span>
            </div>
          </li>
        ))}
      </ul>
      <form class="set-form" aria-label="Add or replace a secret" onSubmit={add.onSubmit}>
        <label>
          Name
          <input name="name" required placeholder="anthropic…" autocomplete="off" spellcheck={false} {...invalid(add, "name")} />
          <FieldError form={add} input="name" />
        </label>
        <label>
          Goes to
          <select name="target" value={kind} onChange={(e) => setKind(oneOf(KINDS, e.currentTarget.value) ?? kind)}>
            {Object.entries(KINDS).map(([k, label]) => (
              <option key={k} value={k}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <TargetFields kind={kind} form={add} />
        <label class="grow">
          Value
          <input name="value" type="password" required autocomplete="new-password" spellcheck={false} {...invalid(add, "value")} />
          <FieldError form={add} input="value" />
        </label>
        <button type="submit" class="btn" disabled={add.busy}>
          {add.busy ? "Saving…" : "Save"}
        </button>
        <FormError form={add} />
      </form>
    </section>
  );
}

/** This browser's keys: a laptop and a phone can differ. */
function Keyboard() {
  const prefs = keyboard.value;
  const [recording, setRecording] = useState(false);
  const [refused, setRefused] = useState(false);
  const record = (e: KeyboardEvent) => {
    if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setRecording(false);
      return;
    }
    const chord: Chord = { code: e.code, ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, meta: e.metaKey };
    if (!usablePrefix(chord)) {
      setRefused(true);
      return;
    }
    setKeyboard({ ...prefs, prefix: chord });
    setRefused(false);
    setRecording(false);
  };
  const meta: Record<OptionAsMeta, string> = { left: "Left Option", both: "Both Options", off: "Neither" };
  return (
    <section class="set" aria-labelledby="keyboard-h">
      <h2 id="keyboard-h">Keyboard</h2>
      <div class="set-row">
        <span>Prefix, to reach iglu from a terminal</span>
        <button
          type="button"
          class="btn"
          aria-pressed={recording}
          onClick={() => {
            setRecording(true);
            setRefused(false);
          }}
          onKeyDown={recording ? record : undefined}
          onBlur={() => setRecording(false)}
        >
          {recording ? "Press a Ctrl chord…" : <kbd>{chordLabel(prefs.prefix, mac)}</kbd>}
        </button>
        {refused ? <span class="field-err" role="alert">Use Ctrl with a letter, Space, [, ] or \.</span> : null}
      </div>
      <label class="set-row">
        <input type="checkbox" checked={prefs.altMoves} onChange={(e) => setKeyboard({ ...prefs, altMoves: e.currentTarget.checked })} />
        <span>
          {mac ? "⌥H ⌥J ⌥K ⌥L" : "Alt+H Alt+J Alt+K Alt+L"} move between columns and workspaces, even in a terminal
        </span>
      </label>
      {mac ? (
        <label class="set-row">
          <span>Option as Meta in terminals</span>
          <select value={prefs.optionAsMeta} onChange={(e) => setKeyboard({ ...prefs, optionAsMeta: e.currentTarget.value as OptionAsMeta })}>
            {(["left", "both", "off"] as const).map((o) => (
              <option key={o} value={o}>
                {meta[o]}
              </option>
            ))}
          </select>
        </label>
      ) : null}
    </section>
  );
}

function Account() {
  return (
    <section class="set" aria-labelledby="account-h">
      <h2 id="account-h">Account</h2>
      <div class="set-row">
        <span translate={false}>{me.value?.email ?? me.value?.name ?? ""}</span>
        <div class="seg" role="group" aria-label="Look">
          {(["auto", "dark", "light"] as const).map((l) => (
            <button key={l} type="button" aria-pressed={look.value === l} onClick={() => (look.value = l)}>
              {l === "auto" ? "System" : l === "dark" ? "Polar night" : "Snowfield"}
            </button>
          ))}
        </div>
        <span class="set-actions">
          <button
            type="button"
            class="btn"
            onClick={() => void enableNotifications().then((p) => say(p === "granted" ? "Notifications are on." : "Notifications are blocked in this browser."))}
          >
            Notifications
          </button>
          <button type="button" class="btn" onClick={() =>
              void attempt(async () => {
                await api.logout();
                return true;
              }).then((done) => {
                if (done) location.assign("/");
              })
            }>
            Sign out
          </button>
        </span>
      </div>
    </section>
  );
}
