// What a workspace may do from inside, with `iglu`, as the console edits it.

import type { AccessGrant } from "../generated/AccessGrant.ts";
import type { Permission } from "../generated/Permission.ts";
import type { WorkspaceId } from "../generated/WorkspaceId.ts";

/** Every permission, in the order the console shows them, with what each
 * lets a workspace do, said plainly. */
export const PERMISSIONS: readonly { permission: Permission; label: string; title: string }[] = [
  { permission: "view", label: "See", title: "See the workspace, its columns and what they're doing" },
  { permission: "read_output", label: "Read", title: "Read what its terminals show" },
  { permission: "send_input", label: "Type", title: "Type into its terminals: as much as running commands there" },
  { permission: "manage_columns", label: "Columns", title: "Open, close and restart its columns" },
  { permission: "publish_routes", label: "Publish", title: "Publish its ports as previews" },
  { permission: "operate", label: "Start/stop", title: "Start, freeze, stop and rename it" },
];

export function holds(access: readonly AccessGrant[], workspace: WorkspaceId, permission: Permission): boolean {
  return access.some((grant) => grant.workspace === workspace && grant.permissions.includes(permission));
}

/** `access` with `permission` on `workspace` given or taken away. A
 * workspace left with nothing drops out. */
export function toggled(access: readonly AccessGrant[], workspace: WorkspaceId, permission: Permission): AccessGrant[] {
  const had = holds(access, workspace, permission);
  const found = access.some((grant) => grant.workspace === workspace);
  const next = access.map((grant) =>
    grant.workspace !== workspace
      ? grant
      : { ...grant, permissions: had ? grant.permissions.filter((p) => p !== permission) : [...grant.permissions, permission] },
  );
  if (!found) next.push({ workspace, permissions: [permission] });
  return next.filter((grant) => grant.permissions.length > 0);
}

/** The workspaces `access` names, the workspace itself first. */
export function named(access: readonly AccessGrant[], self: WorkspaceId): WorkspaceId[] {
  const others = access.map((grant) => grant.workspace).filter((id) => id !== self);
  return [self, ...others];
}
