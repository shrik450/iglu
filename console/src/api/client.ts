// iglud's JSON API. The console trusts iglud's responses as their generated
// types: iglud serves this bundle, so both ship from the same commit. A tab
// left open across an upgrade notices from the snapshot stream's boot ID
// (events.ts) and asks to reload.

import type { ActivityEntry } from "../generated/ActivityEntry.ts";
import type { AddColumn } from "../generated/AddColumn.ts";
import type { BuildStarted } from "../generated/BuildStarted.ts";
import type { ColumnSpec } from "../generated/ColumnSpec.ts";
import type { ColumnStatus } from "../generated/ColumnStatus.ts";
import type { ChangeProject } from "../generated/ChangeProject.ts";
import type { CreateEnvironment } from "../generated/CreateEnvironment.ts";
import type { CreateProject } from "../generated/CreateProject.ts";
import type { DnsLabel } from "../generated/DnsLabel.ts";
import type { CreateWorkspace } from "../generated/CreateWorkspace.ts";
import type { DesiredState } from "../generated/DesiredState.ts";
import type { ErrorBody } from "../generated/ErrorBody.ts";
import type { ErrorKind } from "../generated/ErrorKind.ts";
import type { Field } from "../generated/Field.ts";
import type { GuestPort } from "../generated/GuestPort.ts";
import type { LiveView } from "../generated/LiveView.ts";
import type { Me } from "../generated/Me.ts";
import type { ProjectId } from "../generated/ProjectId.ts";
import type { ProjectView } from "../generated/ProjectView.ts";
import type { PublishPort } from "../generated/PublishPort.ts";
import type { PutLayout } from "../generated/PutLayout.ts";
import type { PutSecret } from "../generated/PutSecret.ts";
import type { SecretName } from "../generated/SecretName.ts";
import type { SecretView } from "../generated/SecretView.ts";
import type { RenameWorkspace } from "../generated/RenameWorkspace.ts";
import type { RouteId } from "../generated/RouteId.ts";
import type { RouteView } from "../generated/RouteView.ts";
import type { SessionName } from "../generated/SessionName.ts";
import type { SetDesiredState } from "../generated/SetDesiredState.ts";
import type { WorkspaceId } from "../generated/WorkspaceId.ts";
import type { WorkspaceName } from "../generated/WorkspaceName.ts";
import type { WorkspaceView } from "../generated/WorkspaceView.ts";

/** A request iglu refused, or that never got an answer from iglu. */
export class ApiError extends Error {
  /** 0 when iglu couldn't be reached. */
  readonly status: number;
  /** null when the answer wasn't iglu's, such as a proxy's. */
  readonly kind: ErrorKind | null;
  /** The input the error is about, when it's about one. */
  readonly field: Field | null;

  constructor(status: number, kind: ErrorKind | null, message: string, field: Field | null = null) {
    super(message);
    this.status = status;
    this.kind = kind;
    this.field = field;
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
  let response: Response;
  try {
    response = await fetch(path, init);
  } catch {
    throw new ApiError(0, null, "Couldn't reach iglu. Check the connection and try again.");
  }
  if (response.status === 401) {
    location.assign(`/auth/login?return=${encodeURIComponent(location.pathname)}`);
    throw new ApiError(401, "unauthorized", "Signing in…");
  }
  if (!response.ok) {
    const error = await errorBody(response);
    if (error) throw new ApiError(response.status, error.error, error.message, error.field);
    const status = `${response.status} ${response.statusText}`.trim();
    throw new ApiError(response.status, null, `Something other than iglu answered ${status}.`);
  }
  if (response.status === 204) return undefined as T;
  try {
    return (await response.json()) as T;
  } catch {
    throw new ApiError(response.status, null, "iglu's reply wasn't what this page expected. Reload the page.");
  }
}

/** iglu's error body, if the response has one. */
async function errorBody(response: Response): Promise<ErrorBody | null> {
  if (!response.headers.get("content-type")?.startsWith("application/json")) return null;
  return (await response.json().catch(() => null)) as ErrorBody | null;
}

export const api = {
  async me(): Promise<Me> {
    const me = await request<Me>("GET", "/v1/me");
    csrf = me.csrf_token;
    return me;
  },
  create: (body: CreateWorkspace) =>
    request<WorkspaceView>("POST", "/v1/workspaces", body, { "idempotency-key": crypto.randomUUID() }),
  setState: (ws: WorkspaceView, state: DesiredState) =>
    request<WorkspaceView>("PUT", `/v1/workspaces/${ws.id}/desired-state`, { state, expected_revision: ws.revision } satisfies SetDesiredState),
  rename: (id: WorkspaceId, name: WorkspaceName) => request<WorkspaceView>("PUT", `/v1/workspaces/${id}/name`, { name } satisfies RenameWorkspace),
  seen: (id: WorkspaceId) => request<void>("POST", `/v1/workspaces/${id}/seen`),
  columns: (id: WorkspaceId) => request<ColumnStatus[]>("GET", `/v1/workspaces/${id}/columns`),
  addColumn: (id: WorkspaceId, body: AddColumn) => request<ColumnSpec>("POST", `/v1/workspaces/${id}/columns`, body),
  putLayout: (id: WorkspaceId, body: PutLayout) => request<void>("PUT", `/v1/workspaces/${id}/layout`, body),
  restartColumn: (id: WorkspaceId, session: SessionName) => request<void>("POST", `/v1/workspaces/${id}/columns/${session}/restart`),
  closeColumn: (id: WorkspaceId, session: SessionName) => request<void>("DELETE", `/v1/workspaces/${id}/columns/${session}`),
  publish: (id: WorkspaceId, port: GuestPort) => request<RouteView>("POST", `/v1/workspaces/${id}/routes`, { port } satisfies PublishPort),
  unpublish: (id: WorkspaceId, route: RouteId) => request<void>("DELETE", `/v1/workspaces/${id}/routes/${route}`),
  createProject: (body: CreateProject) => request<ProjectView>("POST", "/v1/projects", body),
  changeProject: (id: ProjectId, body: ChangeProject) => request<ProjectView>("PUT", `/v1/projects/${id}`, body),
  removeProject: (id: ProjectId) => request<void>("DELETE", `/v1/projects/${id}`),
  createEnvironment: (body: CreateEnvironment) => request<BuildStarted>("POST", "/v1/environments", body),
  buildEnvironment: (name: DnsLabel) => request<BuildStarted>("POST", `/v1/environments/${name}/builds`),
  secrets: () => request<SecretView[]>("GET", "/v1/secrets"),
  putSecret: (name: SecretName, body: PutSecret) => request<void>("PUT", `/v1/secrets/${name}`, body),
  deleteSecret: (name: SecretName) => request<void>("DELETE", `/v1/secrets/${name}`),
  live: (id: WorkspaceId) => request<LiveView>("GET", `/v1/workspaces/${id}/live`),
  activity: (id: WorkspaceId) => request<ActivityEntry[]>("GET", `/v1/workspaces/${id}/activity`),
  logout: () => request<void>("POST", "/auth/logout"),
};

/** A message to show for anything a request threw. */
export function failure(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
