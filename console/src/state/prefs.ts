// Per-browser conveniences: the look and folded groups. Storage can be missing or refuse writes; everything works
// without it.

export type Look = "auto" | "dark" | "light";


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

export function loadLook(): Look {
  const value = read("look");
  return value === "dark" || value === "light" ? value : "auto";
}

export const saveLook = (look: Look) => write("look", look);

export function loadCollapsed(): Set<string> {
  const value = read("collapsed");
  return new Set(Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : []);
}

export const saveCollapsed = (keys: ReadonlySet<string>) => write("collapsed", [...keys]);
