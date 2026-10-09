// Reading submitted forms.

/** A field's trimmed text, or undefined when it's empty or missing. */
export function textOf(data: FormData): (key: string) => string | undefined {
  return (key) => {
    const value = data.get(key);
    return typeof value === "string" && value.trim() !== "" ? value.trim() : undefined;
  };
}
