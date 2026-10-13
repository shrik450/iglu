// Everything set up once: environments, projects, secrets and the account.

import { useEffect, useState } from "preact/hooks";

import { attempt, signOut } from "../actions.ts";
import { api, type SecretPath } from "../api/client.ts";
import { AddEnvironment } from "../components/AddEnvironment.tsx";
import { type Form, FieldError, FormError, invalid, textOf, useForm } from "../components/forms.tsx";
import { mac } from "../keyboard.ts";
import { m } from "../messages.tsx";
import { keyboard, look, setKeyboard, setLook, setTermStyle, termStyle } from "../preferences.ts";
import { enableNotifications } from "../notify.ts";
import type { EnvironmentView } from "../generated/EnvironmentView.ts";
import type { ProjectView } from "../generated/ProjectView.ts";
import type { PutSecret } from "../generated/PutSecret.ts";
import type { SecretTarget } from "../generated/SecretTarget.ts";
import type { SecretView } from "../generated/SecretView.ts";
import { repoLabel } from "../state/groups.ts";
import { type Chord, chordLabel, usablePrefix } from "../state/keys.ts";
import type { OptionAsMeta } from "../generated/OptionAsMeta.ts";
import type { TerminalStyle } from "../generated/TerminalStyle.ts";
import { environments, me, projects, say, workspaces } from "../state/store.ts";
import { type CellSize, THEMES, XTERM, cellSize, cellSizes, colorsOf, fontName, formatTheme, isLight, parseTheme } from "../state/themes.ts";
import { oneOf, unreachable } from "../state/unsaved.ts";
import { pageColors } from "../terminal.ts";
import { advance, fontTrouble } from "../termstyle.ts";

const when = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

export function Settings() {
  return (
    <div class="settings">
      <h1 class="sr-only">{m.settings.heading}</h1>
      <Environments />
      <Projects />
      <Secrets />
      <Keyboard />
      <Terminal />
      <Account />
    </div>
  );
}

function buildText(env: EnvironmentView): string {
  const latest = env.latest;
  if (!latest) return m.settings.build(null);
  switch (latest.status) {
    case "Building":
    case "Failed":
      return m.settings.build({ status: latest.status });
    case "Ready":
      return m.settings.build({ status: "Ready", at: when.format(latest.created_at) });
    default:
      return unreachable(latest);
  }
}

function Environments() {
  return (
    <section class="set" aria-labelledby="envs-h">
      <h2 id="envs-h">{m.settings.environments}</h2>
      <ul class="set-list">
        {environments.value.map((env) => (
          <li key={env.name}>
            <div class="set-row">
              <b translate={false}>{env.name}</b>
              <code translate={false}>{env.source}</code>
              <span class={`state${env.latest?.status === "Failed" ? " s-trouble" : ""}`}>{buildText(env)}</span>
              <button type="button" class="btn" disabled={env.latest?.status === "Building"} onClick={() => void attempt(() => api.buildEnvironment(env.name))}>
                {m.settings.rebuild}
              </button>
            </div>
            {env.latest?.status === "Failed" ? (
              <details class="log">
                <summary>{m.settings.log}</summary>
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
      <h2 id="projects-h">{m.settings.projects}</h2>
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
        >
          <b>{project.name}</b>
        </a>
        <span class="muted" translate={false}>
          {m.project.source(project.repo ? repoLabel(project.repo) : null, project.environment)}
        </span>
        <span class="state">{m.settings.workspaces(live)}</span>
      </div>
    </li>
  );
}

type TargetKind = SecretTarget["kind"];

const KINDS: Readonly<Record<TargetKind, string>> = {
  env: m.settings.target("env"),
  file: m.settings.target("file"),
  git_credential: m.settings.target("git_credential"),
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
function TargetFields({ kind, form }: { kind: TargetKind; form: Form<PutSecret & SecretPath> }) {
  switch (kind) {
    case "env":
      return (
        <label>
          {m.settings.variable}
          <input name="env" required placeholder={m.settings.variableHint} autocomplete="off" spellcheck={false} {...invalid(form, "target")} />
          <FieldError form={form} input="target" />
        </label>
      );
    case "file":
      return (
        <label>
          {m.settings.path}
          <input name="path" required placeholder={m.settings.pathHint} autocomplete="off" spellcheck={false} {...invalid(form, "target")} />
          <FieldError form={form} input="target" />
        </label>
      );
    case "git_credential":
      return (
        <>
          <label>
            {m.settings.host}
            <input name="host" required placeholder={m.settings.hostHint} autocomplete="off" spellcheck={false} {...invalid(form, "target")} />
          </label>
          <label>
            {m.settings.username}
            <input name="username" required placeholder={m.settings.usernameHint} autocomplete="off" spellcheck={false} {...invalid(form, "target")} />
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
  const load = async () => {
    const got = await attempt(() => api.secrets());
    if (got) setSecrets(got.value);
  };
  useEffect(() => void load(), []);
  const add = useForm<PutSecret & SecretPath>(async (data, form) => {
    const text = textOf(data);
    const name = text("name");
    const value = data.get("value");
    const target = targetOf(kind, text);
    if (!name || !target || typeof value !== "string" || value === "") return;
    await api.putSecret(name, { target, value });
    form.reset();
    await load();
    say(m.settings.secretSaved(name));
  }, m.settings.secretRules);
  return (
    <section class="set" aria-labelledby="secrets-h">
      <h2 id="secrets-h">{m.settings.secrets}</h2>
      <p class="muted">{m.settings.secretsAbout}</p>
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
                        void attempt(() => api.deleteSecret(secret.name)).then((done) => {
                          if (!done) return;
                          setRemoving(null);
                          void load();
                        })
                      }
                    >
                      {m.settings.deleteSecret(secret.name)}
                    </button>
                    <button type="button" class="btn" onClick={() => setRemoving(null)}>
                      {m.settings.keep}
                    </button>
                  </>
                ) : (
                  <button type="button" class="btn" onClick={() => setRemoving(secret.name)}>
                    {m.settings.delete}
                  </button>
                )}
              </span>
            </div>
          </li>
        ))}
      </ul>
      <form class="set-form" aria-label={m.settings.addSecret} onSubmit={add.onSubmit}>
        <label>
          {m.settings.secretName}
          <input name="name" required placeholder={m.settings.secretNameHint} autocomplete="off" spellcheck={false} {...invalid(add, "name")} />
          <FieldError form={add} input="name" />
        </label>
        <label>
          {m.settings.goesTo}
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
          {m.settings.value}
          <input name="value" type="password" required autocomplete="new-password" spellcheck={false} {...invalid(add, "value")} />
          <FieldError form={add} input="value" />
        </label>
        <button type="submit" class="btn" disabled={add.busy}>
          {m.settings.save(add.busy)}
        </button>
        <FormError form={add} />
      </form>
    </section>
  );
}

/** The keyboard: the prefix, moving with Alt, and Option on a Mac. */
function Keyboard() {
  const prefs = keyboard.value;
  const [recording, setRecording] = useState(false);
  const [refused, setRefused] = useState(false);
  // An abandoned recording takes its complaint with it.
  const cancel = () => {
    setRecording(false);
    setRefused(false);
  };
  const record = (e: KeyboardEvent) => {
    if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      cancel();
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
  return (
    <section class="set" aria-labelledby="keyboard-h">
      <h2 id="keyboard-h">{m.settings.keyboard}</h2>
      <div class="set-row">
        <span>{m.settings.prefix}</span>
        <button
          type="button"
          class="btn"
          aria-pressed={recording}
          aria-label={recording ? m.settings.prefixRecording : m.settings.prefixLabel(chordLabel(prefs.prefix, mac))}
          onClick={() => {
            setRecording(true);
            setRefused(false);
          }}
          onKeyDown={recording ? record : undefined}
          onBlur={cancel}
        >
          {recording ? m.settings.prefixPress : <kbd>{chordLabel(prefs.prefix, mac)}</kbd>}
        </button>
        {refused ? (
          <span class="field-err" role="alert">
            {m.settings.prefixRefused}
          </span>
        ) : null}
      </div>
      <label class="set-row">
        <input type="checkbox" checked={prefs.alt_moves} onChange={(e) => setKeyboard({ ...prefs, alt_moves: e.currentTarget.checked })} />
        <span>{m.settings.altMoves(mac ? "⌥H ⌥J ⌥K ⌥L" : "Alt+H Alt+J Alt+K Alt+L")}</span>
      </label>
      {mac ? (
        <label class="set-row">
          <span>{m.settings.optionAsMeta}</span>
          <select value={prefs.option_as_meta} onChange={(e) => setKeyboard({ ...prefs, option_as_meta: e.currentTarget.value as OptionAsMeta })}>
            {(["Left", "Both", "Off"] as const).map((o) => (
              <option key={o} value={o}>
                {m.settings.meta(o)}
              </option>
            ))}
          </select>
        </label>
      ) : null}
    </section>
  );
}

/** The terminal's type and colours. */
function Terminal() {
  const prefs = termStyle.value;
  const set = (change: Partial<TerminalStyle>, typing = false) => setTermStyle({ ...prefs, ...change }, typing);
  const [font, setFont] = useState(prefs.font ?? "");
  const [fontRefused, setFontRefused] = useState(false);
  useEffect(() => setFont(prefs.font ?? ""), [prefs.font]);
  const keepFont = () => {
    const name = fontName(font);
    setFontRefused(Boolean(font.trim()) && !name);
    if (!font.trim() || name) set({ font: name });
  };
  const names = Object.keys(THEMES);
  const size = cellSize(advance.value, prefs.size);
  // A size kept from another font may fall outside this one's.
  const sizes = cellSizes(advance.value);
  if (!sizes.some((s) => s.width === size.width)) sizes.push(size), sizes.sort((a, b) => a.width - b.width);
  const pasted = prefs.theme === "pasted" ? parseTheme(prefs.pasted) : null;
  const choose = (theme: string) => {
    // A theme of one's own starts from the one it takes over from.
    if (theme === "pasted" && !prefs.pasted.trim()) set({ theme, pasted: formatTheme(colorsOf(prefs.theme, "") ?? pageColors()) });
    else set({ theme });
  };
  return (
    <section class="set" aria-labelledby="terminal-h">
      <h2 id="terminal-h">{m.settings.terminal}</h2>
      <label class="set-row">
        <span>{m.settings.colours}</span>
        <select value={prefs.theme} onChange={(e) => choose(e.currentTarget.value)}>
          <option value="iglu">{m.settings.ownColours}</option>
          {[false, true].map((light) => (
            <optgroup key={String(light)} label={light ? m.settings.light : m.settings.dark}>
              {names
                .filter((name) => isLight(THEMES[name]!) === light)
                .map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
            </optgroup>
          ))}
          <option value="pasted">{m.settings.pastedColours}</option>
        </select>
      </label>
      {pasted ? (
        <div class="set-form">
          <label class="grow">
            <span>{m.settings.pasteAbout}</span>
            <textarea
              class="theme-paste"
              rows={8}
              value={prefs.pasted}
              spellcheck={false}
              autocomplete="off"
              aria-invalid={"error" in pasted}
              aria-describedby={"error" in pasted ? "theme-err" : undefined}
              onInput={(e) => set({ pasted: e.currentTarget.value }, true)}
            />
          </label>
          {"error" in pasted ? (
            <span id="theme-err" class="field-err">
              {m.settings.themeProblem(pasted.error)}
            </span>
          ) : null}
        </div>
      ) : null}
      <label class="set-row">
        <span>{m.settings.font}</span>
        <input
          value={font}
          placeholder="JetBrains Mono"
          autocomplete="off"
          spellcheck={false}
          aria-invalid={fontRefused}
          onInput={(e) => setFont(e.currentTarget.value)}
          onChange={keepFont}
          onKeyDown={(e) => e.key === "Enter" && keepFont()}
        />
        {fontRefused ? (
          <span class="field-err" role="alert">
            {m.settings.fontRefused}
          </span>
        ) : fontTrouble.value && prefs.font ? (
          <span class="field-err" role="status">
            {m.settings.fontTrouble(prefs.font, fontTrouble.value === "missing")}
          </span>
        ) : null}
      </label>
      <label class="set-row">
        <span>{m.settings.size}</span>
        <select value={String(pixels(size))} onChange={(e) => set({ size: Number(e.currentTarget.value) })}>
          {sizes.map((s) => (
            <option key={s.width} value={String(pixels(s))}>
              {m.settings.pixels(pixels(s))}
            </option>
          ))}
        </select>
      </label>
      <Sample />
    </section>
  );
}

/** A size as it's shown and kept: whole pixels, within what iglud keeps. */
const pixels = (size: CellSize) => Math.min(40, Math.max(6, Math.round(size.font)));

/** A few lines as a terminal would draw them, in the type and colours chosen. */
function Sample() {
  const ink = (index: number) => ({ color: `var(--term-ansi-${index})` });
  return (
    <div class="term-sample" aria-hidden="true" translate={false}>
      <div>
        <span style={ink(2)}>~/iglu</span> <span style={ink(4)}>main</span> <span style={ink(5)}>❯</span> git status --short
      </div>
      <div>
        <span style={ink(1)}> M</span> console/src/<span class="term-sample-pick">terminal.ts</span>
      </div>
      <div>
        <span style={ink(3)}>??</span> console/src/termstyle.ts
      </div>
      <div>
        <span style={ink(2)}>~/iglu</span> <span style={ink(4)}>main</span> <span style={ink(5)}>❯</span> <span class="term-sample-caret"> </span>
      </div>
      <div class="term-sample-colours">
        {XTERM.map((_, index) => (
          <span key={index} style={{ background: `var(--term-ansi-${index})` }} />
        ))}
      </div>
    </div>
  );
}

function Account() {
  return (
    <section class="set" aria-labelledby="account-h">
      <h2 id="account-h">{m.settings.account}</h2>
      <div class="set-row">
        <span translate={false}>{me.value?.email ?? me.value?.name ?? ""}</span>
        <div class="seg" role="group" aria-label={m.settings.look}>
          {(["Auto", "Dark", "Light"] as const).map((l) => (
            <button key={l} type="button" aria-pressed={look.value === l} onClick={() => setLook(l)}>
              {m.settings.lookName(l)}
            </button>
          ))}
        </div>
        <span class="set-actions">
          <button
            type="button"
            class="btn"
            onClick={() => void enableNotifications().then((p) => say(m.notifications.answer(p)))}
          >
            {m.notifications.button}
          </button>
          <button type="button" class="btn" onClick={() => void signOut()}>
            {m.settings.signOut}
          </button>
        </span>
      </div>
    </section>
  );
}
