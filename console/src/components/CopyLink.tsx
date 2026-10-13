// Copies a preview's link, and says who it works for: only its owner can open one.

import { m } from "../messages.tsx";
import { say } from "../state/store.ts";

export function copyLink(url: string): void {
  navigator.clipboard.writeText(url).then(
    () => say(m.preview.copied(url)),
    () => say(m.preview.notCopied),
  );
}

export function CopyLink({ url, name }: { url: string; name: string }) {
  return (
    <button type="button" class="btn" aria-label={m.preview.copyLabel(name)} title={m.preview.onlyYou} onClick={() => copyLink(url)}>
      {m.preview.copy}
    </button>
  );
}
