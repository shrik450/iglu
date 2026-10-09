// Telling the person something needs them: toasts, system notifications, the
// tab title and the favicon.

import { effect } from "@preact/signals";

import type { Need } from "./generated/Need.ts";
import type { WorkspaceView } from "./generated/WorkspaceView.ts";
import { situation } from "./state/situation.ts";
import { unreachable } from "./state/unsaved.ts";
import { current, toast, waiting } from "./state/store.ts";

let primed = false;

/** A key that changes when why a workspace needs the person changes. */
function mark(ws: WorkspaceView): string {
  const top = ws.attention;
  const status = top ? `${top.session}:${top.thread}:${top.state}:${top.seen}:${top.updated_at}` : "";
  return `${ws.needs_you ?? ""}:${status}:${ws.condition?.kind ?? ""}`;
}

/** What to tell the person, for the reason the core gave. */
function notice(ws: WorkspaceView, need: Need): { title: string; body: string } {
  switch (need) {
    case "waiting":
      return { title: `${ws.name} needs you`, body: ws.attention?.summary ?? "" };
    case "done":
      return { title: `${ws.name} is done`, body: ws.attention?.summary ?? "" };
    case "trouble": {
      const now = situation(ws);
      return { title: `${ws.name} is in trouble`, body: "detail" in now ? `${now.title}. ${now.detail}` : "" };
    }
    default:
      return unreachable(need);
  }
}

/** Toasts, and notifies when hidden, for workspaces that newly need the person. */
export function noticeChanges(before: readonly WorkspaceView[], after: readonly WorkspaceView[]): void {
  // The first snapshot is what was already there, not news.
  if (!primed) {
    primed = true;
    return;
  }
  const previous = new Map(before.map((ws) => [ws.id, ws]));
  for (const ws of after) {
    const was = previous.get(ws.id);
    const need = ws.needs_you;
    if (!need || (was && mark(was) === mark(ws))) continue;
    const looking = current.value?.id === ws.id && document.visibilityState === "visible";
    if (looking) continue;
    const { title, body } = notice(ws, need);
    toast(ws.name, title, body);
    if (document.visibilityState === "hidden" && "Notification" in window && Notification.permission === "granted") {
      const note = new Notification(title, { body, tag: ws.id });
      note.onclick = () => {
        window.focus();
        location.assign(`/w/${ws.name}`);
      };
    }
  }
}

export async function enableNotifications(): Promise<NotificationPermission> {
  if (!("Notification" in window)) return "denied";
  return Notification.permission === "default" ? Notification.requestPermission() : Notification.permission;
}

function favicon(dot: boolean): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><path d="M2 26a14 14 0 0 1 28 0z" fill="#eaf5ff" stroke="#8fb6dd"/><path d="M13 26v-4a3 3 0 0 1 6 0v4z" fill="#0b1530"/>${dot ? '<circle cx="26" cy="7" r="6" fill="#ffc46b"/>' : ""}</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

effect(() => {
  const count = waiting.value.length;
  const page = current.value?.name;
  document.title = `${count ? `(${count}) ` : ""}${page ? `${page} · ` : ""}iglu`;
  let link = document.querySelector<HTMLLinkElement>("link[rel=icon]");
  if (!link) {
    link = document.createElement("link");
    link.rel = "icon";
    document.head.append(link);
  }
  link.href = favicon(count > 0);
});
