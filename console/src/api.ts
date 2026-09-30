// Typed access to iglud's API. Shapes mirror the server's JSON views.

export type Phase =
  | "creating"
  | "starting"
  | "running"
  | "freezing"
  | "frozen"
  | "stopping"
  | "stopped"
  | "deleting"
  | "deleted";

export type DesiredState = "running" | "frozen" | "stopped" | "deleted";
export type AttentionState = "working" | "waiting" | "done" | "idle" | "exited";
export type Seen = "seen" | "unseen";

export interface Attention {
  session: string;
  state: AttentionState;
  summary: string;
  updated_at: number;
  seen: Seen;
}

export interface Route {
  id: string;
  name: string;
  port: number;
  url: string;
}

export type Condition =
  | { kind: "error"; code: string; message: string; at: number }
  | { kind: "capacity"; available: number; needed: number }
  | { kind: "runtime_failed" }
  | { kind: "host_offline"; last_seen: number | null };

export interface Workspace {
  id: string;
  name: string;
  repo: string;
  branch: string;
  environment: string;
  phase: Phase;
  desired: DesiredState;
  revision: number;
  condition: Condition | null;
  memory: number | null;
  observed_at: number | null;
  attention: Attention | null;
  sessions: Attention[];
  routes: Route[];
  created_at: number;
}

export interface Me {
  id: string;
  name: string | null;
  email: string | null;
  csrf_token: string;
  preview_domain: string;
}

export interface Environment {
  name: string;
  source: string;
  latest: { status: "building" | "ready" | "failed"; log_tail?: string } | null;
}

export interface TerminalInfo {
  name: string;
  clients: number;
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

let csrf = "";

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
    const message = typeof data === "object" && data !== null && "message" in data ? String(data.message) : response.statusText;
    throw new ApiError(response.status, message);
  }
  return data as T;
}

export const api = {
  async me(): Promise<Me> {
    const me = await request<Me>("GET", "/v1/me");
    csrf = me.csrf_token;
    return me;
  },
  workspaces: () => request<Workspace[]>("GET", "/v1/workspaces"),
  environments: () => request<Environment[]>("GET", "/v1/environments"),
  create: (body: { environment: string; repo: string; branch?: string; base?: string; name?: string }) =>
    request<Workspace>("POST", "/v1/workspaces", body, { "idempotency-key": crypto.randomUUID() }),
  setState: (ws: Workspace, state: DesiredState) =>
    request<Workspace>("PUT", `/v1/workspaces/${ws.id}/desired-state`, { state, expected_revision: ws.revision }),
  seen: (id: string) => request<void>("POST", `/v1/workspaces/${id}/seen`),
  terminals: (id: string) => request<TerminalInfo[]>("GET", `/v1/workspaces/${id}/terminals`),
  newTerminal: (id: string) => request<{ name: string }>("POST", `/v1/workspaces/${id}/terminals`),
  closeTerminal: (id: string, session: string) => request<void>("DELETE", `/v1/workspaces/${id}/terminals/${session}`),
  publish: (id: string, port: number) => request<Route>("POST", `/v1/workspaces/${id}/routes`, { port }),
  unpublish: (id: string, route: string) => request<void>("DELETE", `/v1/workspaces/${id}/routes/${route}`),
  logout: () => request<void>("POST", "/auth/logout"),
};

/** Streams the caller's workspaces; reconnects on its own. */
export function watchWorkspaces(onUpdate: (workspaces: Workspace[]) => void): EventSource {
  const source = new EventSource("/v1/events");
  source.addEventListener("workspaces", (event) => {
    onUpdate(JSON.parse((event as MessageEvent<string>).data) as Workspace[]);
  });
  return source;
}
