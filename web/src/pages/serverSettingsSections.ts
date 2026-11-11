// The sections of the server settings page, the one place that decides who
// sees which (mirrored natively in `server_settings/mod.rs`). Later packages
// add their line here.

import { can } from "../servers/permissions.ts";
import type { Member, Server, ServerDetail } from "../servers/types.ts";

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
 * everyone sees the members and can leave. `directoryEnabled` is for the
 * Verzeichnis section that discovery adds.
 */
export function serverSettingsSections(
  server: Pick<Server, "role" | "permissions">,
  directoryEnabled: boolean,
): SectionId[] {
  void directoryEnabled; // Verzeichnis, with discovery
  const manage = can(server, "manage_server");
  const moderate = can(server, "moderate_members");
  return [
    ...(manage ? (["allgemein", "rollen", "einladungen"] as const) : []),
    "mitglieder",
    ...(moderate ? (["sperren"] as const) : []),
    "gefahrenzone",
  ];
}

/** A row of Gefahrenzone, top to bottom. */
export type DangerRow = "transfer" | "delete" | "leave";

/** The owner hands the server on (#519) or deletes it; everyone else
 * leaves. */
export function dangerRows(server: Pick<Server, "role">): DangerRow[] {
  return server.role === "owner" ? ["transfer", "delete"] : ["leave"];
}

/** Who the server can be handed to: every member but the owner. */
export function transferCandidates(
  server: Pick<ServerDetail, "owner_id" | "members">,
): Member[] {
  return server.members.filter((member) => member.user_id !== server.owner_id);
}
