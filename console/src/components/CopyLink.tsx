// Copies a preview's link, and says who it works for: only its owner can open one.

import { say } from "../state/store.ts";

export const ONLY_YOU = "Only you can open it, signed in to iglu.";

export function copyLink(url: string): void {
  navigator.clipboard.writeText(url).then(
    () => say(`Copied ${url}. ${ONLY_YOU}`),
    () => say("This browser didn't let iglu copy the link."),
  );
}

export function CopyLink({ url, name }: { url: string; name: string }) {
  return (
    <button type="button" class="btn" aria-label={`Copy the link to ${name}`} title={ONLY_YOU} onClick={() => copyLink(url)}>
      Copy link
    </button>
  );
}
