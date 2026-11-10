// The sections of the server settings page, the one place that decides who
// sees which (mirrored natively in `server_settings/mod.rs`). Later packages
// add their line here.

import { can } from "../servers/permissions.ts";
import type { Server } from "../servers/types.ts";

export type SectionId =
  | "allgemein"
  | "rollen"
  | "einladungen"
  | "mitglieder"
  | "sperren"
  | "gefahrenzone";

/**
 * The sections the caller sees, in page order. `manage_server` edits the
 * server; `moderate_members` kicks, bans and so sees the ban list;
 * everyone sees the members and can leave.
 */
export function serverSettingsSections(
  server: Pick<Server, "role" | "permissions">,
): SectionId[] {
  const manage = can(server, "manage_server");
  const moderate = can(server, "moderate_members");
  return [
    ...(manage ? (["allgemein", "rollen", "einladungen"] as const) : []),
    "mitglieder",
    ...(moderate ? (["sperren"] as const) : []),
    "gefahrenzone",
  ];
}
