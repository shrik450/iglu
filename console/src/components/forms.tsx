// Reading submitted forms, and showing why one was refused.
//
// Every form submits through useForm. An error about a field (from iglu, or
// an InputError from reading the form) shows by the input its first segment
// names, through that input's FieldError; any other error shows at the
// form's FormError. The server is the one parser of what inputs mean, so
// forms only turn text into the request's shape. iglu says which input it
// couldn't take, not why: each form gives, from messages.tsx, what each of
// its inputs takes, and that's said by the one refused. A form is typed by
// its request, so an input's name is one of the request's fields.

import { type RefObject } from "preact";
import { useId, useLayoutEffect, useRef, useState } from "preact/hooks";

import { ApiError, failure } from "../api/client.ts";
import { inputOf } from "../state/fields.ts";

/** A field's trimmed text, or undefined when it's empty or missing. */
export function textOf(data: FormData): (key: string) => string | undefined {
  return (key) => {
    const value = data.get(key);
    return typeof value === "string" && value.trim() !== "" ? value.trim() : undefined;
  };
}

/** Text in an input that can't take the request's shape, such as a port that isn't a number. */
export class InputError extends Error {
  readonly field: string;

  constructor(field: string, message: string) {
    super(message);
    this.field = field;
  }
}

/** A form's inputs: the fields of the request it makes, which are what iglu
 * names when it refuses one. */
type Input<Request> = keyof Request & string;

interface Problem<Request> {
  message: string;
  /** The input it's shown by, or null for the form's own error. */
  input: Input<Request> | null;
}

export interface Form<Request> {
  readonly id: string;
  readonly busy: boolean;
  readonly problem: Problem<Request> | null;
  readonly onSubmit: (e: Event) => Promise<void>;
}

/** What each of a request's inputs takes. */
export type Rules<Request> = { readonly [Field in keyof Request]?: string };

/** Submits a form with `run`, showing what it throws by the input it's
 * about: for an input iglu couldn't take, what it takes, from `rules`. */
export function useForm<Request>(run: (data: FormData, form: HTMLFormElement) => Promise<void>, rules: Rules<Request>): Form<Request> {
  const id = useId();
  const [problem, setProblem] = useState<Problem<Request> | null>(null);
  const [busy, setBusy] = useState(false);
  const onSubmit = async (e: Event) => {
    e.preventDefault();
    const form = e.currentTarget as HTMLFormElement;
    setBusy(true);
    setProblem(null);
    try {
      await run(new FormData(form), form);
    } catch (error) {
      const field = error instanceof ApiError || error instanceof InputError ? error.field : null;
      const input = field === null ? null : inputOf(field);
      const named = input === null ? null : form.elements.namedItem(input);
      const invalid = error instanceof ApiError && error.refusal.from === "iglu" && error.refusal.reason.kind === "Invalid";
      // iglu names the request's fields, and an InputError the form's inputs.
      const known = named ? (input as Input<Request>) : null;
      const rule = invalid && known !== null ? rules[known] : undefined;
      setProblem({ message: rule ?? failure(error), input: known });
      if (named instanceof HTMLElement) {
        const folded = named.closest("details");
        if (folded) folded.open = true;
        named.focus();
      }
    }
    setBusy(false);
  };
  return { id, busy, problem, onSubmit };
}

function errorId<Request>(form: Form<Request>, input: Input<Request>): string {
  return `${form.id}-${input}-error`;
}

/** Marks an input invalid while the form's error is about `input`. */
export function invalid<Request>(form: Form<Request>, input: NoInfer<Input<Request>>): { "aria-invalid"?: true; "aria-describedby"?: string } {
  return form.problem?.input === input ? { "aria-invalid": true, "aria-describedby": errorId(form, input) } : {};
}

/** The form's error about `input`, if that's what it's about. It's hidden
 * from the label it sits in; the input reads it as its description. */
export function FieldError<Request>({ form, input }: { form: Form<Request>; input: NoInfer<Input<Request>> }) {
  return form.problem?.input === input ? (
    <span class="field-err" id={errorId(form, input)} aria-hidden="true">
      {form.problem.message}
    </span>
  ) : null;
}

/** The form's error that isn't about one of its inputs. */
export function FormError<Request>({ form }: { form: Form<Request> }) {
  return form.problem && form.problem.input === null ? (
    <p class="field-err" role="alert">
      {form.problem.message}
    </p>
  ) : null;
}

/** Takes focus as it opens, and again a moment later: opened from a
 * terminal, the terminal takes focus back once in Chromium, and autoFocus
 * loses to it. */
export function useGrab<T extends HTMLElement>(select = false, when = true): RefObject<T | null> {
  const ref = useRef<T>(null);
  useLayoutEffect(() => {
    if (!when) return;
    const take = () => {
      ref.current?.focus();
      if (select && ref.current instanceof HTMLInputElement) ref.current.select();
    };
    take();
    const again = window.setTimeout(take);
    return () => window.clearTimeout(again);
  }, []);
  return ref;
}
