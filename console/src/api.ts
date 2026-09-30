// Typed access to iglud's API. The shapes are generated from the server's
// Rust types (`just api-types`), so the two can't drift apart.

import type { CreateWorkspace } from "./generated/CreateWorkspace";
import type { DesiredState } from "./generated/DesiredState";
import type { EnvironmentView } from "./generated/EnvironmentView";
import type { ErrorBody } from "./generated/ErrorBody";
import type { Me } from "./generated/Me";
import type { NewTerminal } from "./generated/NewTerminal";
import type { PublishPort } from "./generated/PublishPort";
import type { RouteId } from "./generated/RouteId";
import type { RouteView } from "./generated/RouteView";
import type { SessionName } from "./generated/SessionName";
import type { SetDesiredState } from "./generated/SetDesiredState";
import type { TerminalView } from "./generated/TerminalView";
import type { WorkspaceId } from "./generated/WorkspaceId";
import type { WorkspaceView } from "./generated/WorkspaceView";

export type { AttentionView } from "./generated/AttentionView";
export type { Condition } from "./generated/Condition";
export type { CreateWorkspace, DesiredState, EnvironmentView, Me, WorkspaceView };

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

let csrf = "";

function isErrorBody(data: unknown): data is ErrorBody {
  return typeof data === "object" && data !== null && "message" in data && typeof data.message === "string";
}

async function request<T>(method: string, path: string, body?: unknown, headers: Record<string, string> = {}): Promise<T> {
  const init: RequestInit = {
    method,
    credentials: "same-origin",
    headers: { ...headers, ...(method === "GET" ? {} : { "x-csrf-token": csrf }), ...(body === undefined ? {} : { "content-type": "application/json" }) },
  };
  if (body !== undefined) init.body = JSON.stringify(body);
  const response = await fetch(path, init);
  if (response.status === 401) {
    location.href = `/auth/login?return=${encodeURIComponent(location.pathname)}`;
    throw new ApiError(401, "sign in first");
  }
  if (response.status === 204) return undefined as T;
  const data: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    throw new ApiError(response.status, isErrorBody(data) ? data.message : response.statusText);
  }
  return data as T;
}

export const api = {
  async me(): Promise<Me> {
    const me = await request<Me>("GET", "/v1/me");
    csrf = me.csrf_token;
    return me;
  },
  workspaces: () => request<WorkspaceView[]>("GET", "/v1/workspaces"),
  environments: () => request<EnvironmentView[]>("GET", "/v1/environments"),
  create: (body: CreateWorkspace) =>
    request<WorkspaceView>("POST", "/v1/workspaces", body, { "idempotency-key": crypto.randomUUID() }),
  setState: (ws: WorkspaceView, state: DesiredState) =>
    request<WorkspaceView>("PUT", `/v1/workspaces/${ws.id}/desired-state`, {
      state,
      expected_revision: ws.revision,
    } satisfies SetDesiredState),
  seen: (id: WorkspaceId) => request<void>("POST", `/v1/workspaces/${id}/seen`),
  terminals: (id: WorkspaceId) => request<TerminalView[]>("GET", `/v1/workspaces/${id}/terminals`),
  newTerminal: (id: WorkspaceId) => request<NewTerminal>("POST", `/v1/workspaces/${id}/terminals`),
  closeTerminal: (id: WorkspaceId, session: SessionName) =>
    request<void>("DELETE", `/v1/workspaces/${id}/terminals/${session}`),
  publish: (id: WorkspaceId, port: number) =>
    request<RouteView>("POST", `/v1/workspaces/${id}/routes`, { port } satisfies PublishPort),
  unpublish: (id: WorkspaceId, route: RouteId) => request<void>("DELETE", `/v1/workspaces/${id}/routes/${route}`),
  logout: () => request<void>("POST", "/auth/logout"),
};

/** Streams the caller's workspaces; reconnects on its own. */
export function watchWorkspaces(onUpdate: (workspaces: WorkspaceView[]) => void): EventSource {
  const source = new EventSource("/v1/events");
  source.addEventListener("workspaces", (event) => {
    onUpdate(JSON.parse((event as MessageEvent<string>).data) as WorkspaceView[]);
  });
  return source;
}
