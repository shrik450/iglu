// Per-browser conveniences, such as which groups are folded. Settings are
// the person's and live in iglud (preferences.ts). Storage can be missing or
// refuse writes; everything works without it.

function read(key: string): unknown {
  try {
    const raw = localStorage.getItem(`iglu.${key}`);
    return raw === null ? null : (JSON.parse(raw) as unknown);
  } catch {
    return null;
  }
}

function write(key: string, value: unknown): void {
  try {
    localStorage.setItem(`iglu.${key}`, JSON.stringify(value));
  } catch {
    // Storage is a convenience; losing it only loses the preference.
  }
}

/** Calls `changed` when another tab of this browser changes the preference,
 * so every tab follows it without a reload. */
export function watch(key: "collapsed", changed: () => void): void {
  addEventListener("storage", (event) => {
    // A key of null is storage cleared.
    if (event.key === `iglu.${key}` || event.key === null) changed();
  });
}

export function loadCollapsed(): Set<string> {
  const value = read("collapsed");
  return new Set(Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : []);
}

export const saveCollapsed = (keys: ReadonlySet<string>) => write("collapsed", [...keys]);
