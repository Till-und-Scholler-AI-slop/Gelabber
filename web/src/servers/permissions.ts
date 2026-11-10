// The six coarse flags of issue #4, in the API's canonical order, with the
// key of the copy the settings page shows (`serverSettings`
// `permission.<key>.label` / `.hint`). Mirrors
// `backend/api/src/servers/permissions.rs`.

import type { Permission, Server } from "./types.ts";

export const PERMISSIONS = [
  { id: "manage_server", key: "manageServer" },
  { id: "manage_channels", key: "manageChannels" },
  { id: "manage_messages", key: "manageMessages" },
  { id: "mention_everyone", key: "mentionEveryone" },
  { id: "send_messages", key: "sendMessages" },
  { id: "send_files", key: "sendFiles" },
  { id: "join_voice", key: "joinVoice" },
  { id: "go_live", key: "goLive" },
] as const satisfies ReadonlyArray<{ id: Permission; key: string }>;

export const ALL_PERMISSIONS: Permission[] = PERMISSIONS.map((p) => p.id);

/** Does the signed-in user hold `permission` in `server`? Owner: always. */
export function can(
  server: Pick<Server, "role" | "permissions"> | undefined,
  permission: Permission,
): boolean {
  if (!server) return false;
  return server.role === "owner" || server.permissions.includes(permission);
}

/** Canonical order, no duplicates — what the API would echo back. */
export function normalisePermissions(set: Iterable<Permission>): Permission[] {
  const wanted = new Set(set);
  return ALL_PERMISSIONS.filter((p) => wanted.has(p));
}
