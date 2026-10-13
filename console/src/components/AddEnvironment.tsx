// Adding an environment: in Settings, and as the first thing anyone does.

import { api } from "../api/client.ts";
import { m } from "../messages.tsx";
import { say } from "../state/store.ts";
import { FieldError, FormError, invalid, textOf, useForm, useGrab } from "./forms.tsx";

/** `first` suggests a name, since a first environment needs no other. */
export function AddEnvironment({ first = false }: { first?: boolean }) {
  const source = useGrab<HTMLInputElement>(false, first);
  const add = useForm(async (data, form) => {
    const text = textOf(data);
    const name = text("name");
    const source = text("source");
    if (!name || !source) return;
    await api.createEnvironment({ name, source });
    form.reset();
    say(m.environment.building(name));
  }, m.environment.rules);
  return (
    <form class="set-form" aria-label={m.environment.add} onSubmit={add.onSubmit}>
      <label>
        {m.environment.name}
        <input name="name" required defaultValue={first ? "default" : undefined} placeholder={m.environment.nameHint} autocomplete="off" spellcheck={false} {...invalid(add, "name")} />
        <FieldError form={add} input="name" />
      </label>
      <label class="grow">
        {m.environment.source}
        <input name="source" required placeholder={m.environment.sourceHint} autocomplete="off" spellcheck={false} ref={source} {...invalid(add, "source")} />
        <FieldError form={add} input="source" />
      </label>
      <button type="submit" class={`btn${first ? " primary" : ""}`} disabled={add.busy}>
        {m.environment.submit(add.busy)}
      </button>
      <FormError form={add} />
    </form>
  );
}
