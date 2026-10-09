// How the palette orders what a search finds: the closest matches first.

/** How closely a search matches an item, from the item's whole name down to
 * its letters scattered in order through the name and what's beside it. */
export type Closeness = "name" | "start" | "word" | "inside" | "beside" | "scattered";

const ORDER: readonly Closeness[] = ["name", "start", "word", "inside", "beside", "scattered"];

/** How closely `query` matches an item named `label`, with `sub` beside it,
 * or null if it doesn't. Case doesn't matter. */
export function closeness(query: string, label: string, sub = ""): Closeness | null {
  const q = query.trim().toLowerCase();
  const name = label.toLowerCase();
  if (name === q) return "name";
  if (name.startsWith(q)) return "start";
  const at = name.indexOf(q);
  if (at >= 0) return startsWord(name, q) ? "word" : "inside";
  if (sub.toLowerCase().includes(q)) return "beside";
  return scattered(q, `${name} ${sub.toLowerCase()}`) ? "scattered" : null;
}

function startsWord(text: string, q: string): boolean {
  for (let at = text.indexOf(q); at >= 0; at = text.indexOf(q, at + 1)) if (at === 0 || !/[\p{L}\p{N}]/u.test(text[at - 1] ?? "")) return true;
  return false;
}

function scattered(q: string, text: string): boolean {
  let at = 0;
  for (const c of text) if (c === q[at]) at++;
  return at === q.length;
}

/** The items `query` matches, closest first, keeping their order among equals.
 * An empty query matches everything, in order. */
export function search<T extends { label: string; sub?: string | undefined }>(query: string, items: readonly T[]): T[] {
  if (!query.trim()) return [...items];
  return items
    .map((item) => ({ item, close: closeness(query, item.label, item.sub) }))
    .filter((found): found is { item: T; close: Closeness } => found.close !== null)
    .sort((a, b) => ORDER.indexOf(a.close) - ORDER.indexOf(b.close))
    .map((found) => found.item);
}
