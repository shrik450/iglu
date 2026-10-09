// Reading submitted forms, and showing why one was refused.
//
// Every form submits through useForm. An error about a field (from iglu, or
// an InputError from reading the form) shows by the input its first segment
// names, through that input's FieldError; any other error shows at the
// form's FormError. The server is the one parser of what inputs mean, so
// forms only turn text into the request's shape.

import { useId, useState } from "preact/hooks";

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

interface Problem {
  message: string;
  /** The input it's shown by, or null for the form's own error. */
  input: string | null;
}

export interface Form {
  readonly id: string;
  readonly busy: boolean;
  readonly problem: Problem | null;
  readonly onSubmit: (e: Event) => Promise<void>;
}

/** Submits a form with `run`, showing what it throws by the input it's about. */
export function useForm(run: (data: FormData, form: HTMLFormElement) => Promise<void>): Form {
  const id = useId();
  const [problem, setProblem] = useState<Problem | null>(null);
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
      const named = field === null ? null : form.elements.namedItem(inputOf(field));
      setProblem({ message: failure(error), input: named && field !== null ? inputOf(field) : null });
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

function errorId(form: Form, input: string): string {
  return `${form.id}-${input}-error`;
}

/** Marks an input invalid while the form's error is about `input`. */
export function invalid(form: Form, input: string): { "aria-invalid"?: true; "aria-describedby"?: string } {
  return form.problem?.input === input ? { "aria-invalid": true, "aria-describedby": errorId(form, input) } : {};
}

/** The form's error about `input`, if that's what it's about. It's hidden
 * from the label it sits in; the input reads it as its description. */
export function FieldError({ form, input }: { form: Form; input: string }) {
  return form.problem?.input === input ? (
    <span class="field-err" id={errorId(form, input)} aria-hidden="true">
      {form.problem.message}
    </span>
  ) : null;
}

/** The form's error that isn't about one of its inputs. */
export function FormError({ form }: { form: Form }) {
  return form.problem && form.problem.input === null ? (
    <p class="field-err" role="alert">
      {form.problem.message}
    </p>
  ) : null;
}
