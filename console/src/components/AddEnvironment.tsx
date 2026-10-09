// Adding an environment: in Settings, and as the first thing anyone does.

import { api } from "../api/client.ts";
import { say } from "../state/store.ts";
import { FieldError, FormError, invalid, textOf, useForm } from "./forms.tsx";

/** `first` suggests a name, since a first environment needs no other. */
export function AddEnvironment({ first = false }: { first?: boolean }) {
  const add = useForm(async (data, form) => {
    const text = textOf(data);
    const name = text("name");
    const source = text("source");
    if (!name || !source) return;
    await api.createEnvironment({ name, source });
    form.reset();
    say(`Building ${name}.`);
  });
  return (
    <form class="set-form" aria-label="Add an environment" onSubmit={add.onSubmit}>
      <label>
        Name
        <input name="name" required defaultValue={first ? "default" : undefined} placeholder="default…" autocomplete="off" spellcheck={false} {...invalid(add, "name")} />
        <FieldError form={add} input="name" />
      </label>
      <label class="grow">
        Flake
        <input name="source" required placeholder="github:you/iglu-env#default…" autocomplete="off" spellcheck={false} autoFocus={first} {...invalid(add, "source")} />
        <FieldError form={add} input="source" />
      </label>
      <button type="submit" class={`btn${first ? " primary" : ""}`} disabled={add.busy}>
        {add.busy ? "Adding…" : "Add"}
      </button>
      <FormError form={add} />
    </form>
  );
}
