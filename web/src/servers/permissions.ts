// The six coarse flags of issue #4, in the API's canonical order, with the
// copy the settings page shows. Mirrors `api/src/servers/permissions.rs`.

import type { Permission, Server } from "./types.ts";

export const PERMISSIONS: ReadonlyArray<{
  id: Permission;
  label: string;
  hint: string;
}> = [
  {
    id: "manage_server",
    label: "Server verwalten",
    hint: "Umbenennen, Rechte ändern, Einladungen, Kick und Ban.",
  },
  {
    id: "manage_channels",
    label: "Kanäle verwalten",
    hint: "Kategorien und Kanäle anlegen, umbenennen, löschen.",
  },
  {
    id: "manage_messages",
    label: "Nachrichten löschen",
    hint: "Fremde Nachrichten im Chat löschen.",
  },
  {
    id: "send_messages",
    label: "Nachrichten schreiben",
    hint: "In Textkanälen schreiben.",
  },
  {
    id: "send_files",
    label: "Dateien senden",
    hint: "Anhänge hochladen.",
  },
  {
    id: "join_voice",
    label: "Voice beitreten",
    hint: "Sprachkanälen beitreten.",
  },
  {
    id: "go_live",
    label: "Go Live starten",
    hint: "Bildschirm oder Kamera in einen Sprachkanal streamen.",
  },
];

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
