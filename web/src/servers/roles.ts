// Server roles (v0.10, #516), as pure helpers over a `ServerDetail`. The
// owner and "Alle Mitglieder" (`member_permissions`, held by every member)
// are fixed and no rows; custom roles are `server.roles`, by position.
// Mirrors `backend/api/src/servers/membership.rs`.

import type { FieldCode } from "../api/client.ts";
import { translate } from "../i18n/catalog.ts";
import { roles as rolesCatalog } from "../i18n/catalogs/roles.ts";
import { ALL_PERMISSIONS, normalisePermissions } from "./permissions.ts";
import { validateRoleName } from "./rules.ts";
import type {
  Channel,
  Member,
  Permission,
  RoleColor,
  ServerDetail,
  ServerRole,
} from "./types.ts";

/** The six role colours, in the order "Neue Rolle" hands them out. */
export const ROLE_COLORS: readonly RoleColor[] = [
  "amber",
  "blue",
  "green",
  "pink",
  "violet",
  "coral",
];

/** Custom roles of a server and the role ids of a member. */
export const ROLES_MAX = 50;

type Roles = Pick<ServerDetail, "roles">;

function byPosition(a: ServerRole, b: ServerRole): number {
  return a.position - b.position || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

/** How many hold each role: the owner is one, "Alle Mitglieder" everyone,
 * a custom role whoever has its id. */
export function roleCounts(server: Pick<ServerDetail, "members" | "roles">): {
  owner: number;
  base: number;
  roles: Record<string, number>;
} {
  const counts: Record<string, number> = {};
  for (const role of server.roles) counts[role.id] = 0;
  for (const member of server.members) {
    for (const id of new Set(member.role_ids)) {
      if (id in counts) counts[id] = (counts[id] ?? 0) + 1;
    }
  }
  return { owner: 1, base: server.members.length, roles: counts };
}

/** The member's custom roles, in position order. Unknown ids are skipped. */
export function rolesOf(
  member: Pick<Member, "role_ids">,
  server: Roles,
): ServerRole[] {
  const held = new Set(member.role_ids);
  return server.roles.filter((role) => held.has(role.id)).sort(byPosition);
}

/** Whether the member may see the channel, the rule of `Membership::sees`:
 * the owner sees everything, everyone sees a channel that is not limited,
 * and a limited one needs one of its roles. */
export function seesChannel(
  member: Pick<Member, "user_id" | "role" | "role_ids">,
  channel: Pick<Channel, "limited" | "role_ids">,
  server: Pick<ServerDetail, "owner_id">,
): boolean {
  if (member.role === "owner" || member.user_id === server.owner_id)
    return true;
  if (!channel.limited) return true;
  const held = new Set(member.role_ids);
  return channel.role_ids.some((id) => held.has(id));
}

/** What the "Zugriff" field reads: "Alle Mitglieder", the role names in
 * position order, or "Nur Inhaber" for a limited channel without roles. */
export function accessSummary(
  channel: Pick<Channel, "limited" | "role_ids">,
  server: Roles,
): string {
  if (!channel.limited) return translate(rolesCatalog, "base");
  const names = rolesOf(channel, server).map((role) => role.name);
  if (names.length === 0) return translate(rolesCatalog, "ownerOnly");
  return names.join(", ");
}

/** The "Alle Mitglieder" row of an access selection. */
export const BASE_ROLE = "base";

/**
 * A channel's access as the "Zugriff" field edits it: `null` is "Alle
 * Mitglieder", a list is the roles it is limited to. A limited channel
 * whose roles were all deleted is `[]` ("Nur Inhaber").
 */
export type AccessSelection = string[] | null;

export function accessOf(
  channel: Pick<Channel, "limited" | "role_ids">,
): AccessSelection {
  return channel.limited ? [...channel.role_ids] : null;
}

/**
 * One click in the "Zugriff" checklist. Choosing "Alle Mitglieder" clears
 * the roles; choosing a role clears "Alle Mitglieder"; unchecking the last
 * role selects "Alle Mitglieder" again.
 */
export function toggleAccess(
  selection: AccessSelection,
  id: string,
): AccessSelection {
  if (id === BASE_ROLE) return null;
  if (selection === null) return [id];
  if (selection.includes(id)) {
    const rest = selection.filter((role) => role !== id);
    return rest.length > 0 ? rest : null;
  }
  return [...selection, id];
}

/** Whether two access selections allow the same people. */
export function sameAccess(a: AccessSelection, b: AccessSelection): boolean {
  if (a === null || b === null) return a === b;
  const held = new Set(a);
  return a.length === b.length && b.every((id) => held.has(id));
}

/**
 * The `role_ids` a channel form sends: `undefined` while the selection is
 * still `initial` (the channel keeps its access; on create it is open to
 * everyone), `[]` for "Alle Mitglieder", else the roles. A channel whose
 * roles were all deleted ("Nur Inhaber", `[]`) must not send `[]` unasked:
 * that would open it to everyone.
 */
export function accessRoleIds(
  initial: AccessSelection,
  selection: AccessSelection,
): string[] | undefined {
  if (sameAccess(initial, selection)) return undefined;
  return selection === null ? [] : [...selection];
}

/** The members who see the channel, as the `@…` suggestions offer them. */
export function membersWhoSee(
  server: Pick<ServerDetail, "owner_id" | "members">,
  channel: Pick<Channel, "limited" | "role_ids">,
): Member[] {
  if (!channel.limited) return server.members;
  return server.members.filter((member) =>
    seesChannel(member, channel, server),
  );
}

/** Whether "Alle Mitglieder" grants the flag, so a custom role shows it on
 * and fixed ("über „Alle Mitglieder“"). */
export function inheritedFromBase(
  server: Pick<ServerDetail, "member_permissions">,
  permission: Permission,
): boolean {
  return server.member_permissions.includes(permission);
}

/** The colour for a new role: the first one no role uses, else amber. */
export function nextColor(server: Roles): RoleColor {
  const used = new Set(server.roles.map((role) => role.color));
  return ROLE_COLORS.find((color) => !used.has(color)) ?? "amber";
}

/** `name`, or `name 2`, `name 3` … when a role already has it (names are
 * unique per server, ignoring case). */
export function nextRoleName(server: Roles, name: string): string {
  const taken = new Set(server.roles.map((role) => role.name.toLowerCase()));
  if (!taken.has(name.toLowerCase())) return name;
  for (let n = 2; ; n++) {
    const candidate = `${name} ${n}`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
}

/** A member's effective flags: everything for the owner, else the union of
 * "Alle Mitglieder" and their roles, in canonical order. */
export function effectivePermissions(
  member: Pick<Member, "user_id" | "role" | "role_ids">,
  server: Pick<ServerDetail, "owner_id" | "member_permissions" | "roles">,
): Permission[] {
  if (member.role === "owner" || member.user_id === server.owner_id)
    return [...ALL_PERMISSIONS];
  return normalisePermissions([
    ...server.member_permissions,
    ...rolesOf(member, server).flatMap((role) => role.permissions),
  ]);
}

/** The CSS colour of a role's dot or swatch (`--lr-role-*` in index.css).
 * The owner's dot is `--lr-accent`, the base role's `--lr-muted`. */
export function roleColorVar(color: RoleColor): string {
  return `var(--lr-role-${color})`;
}

/** The owner's row in the roles editor; "Alle Mitglieder" is `BASE_ROLE`. */
export const OWNER_ROLE = "owner";

/** The role the editor opens on: the first custom role, else "Alle
 * Mitglieder". */
export function defaultRoleSelection(server: Roles): string {
  return [...server.roles].sort(byPosition)[0]?.id ?? BASE_ROLE;
}

/** Custom roles in the order the editor and the pickers list them. */
export function rolesInOrder(server: Roles): ServerRole[] {
  return [...server.roles].sort(byPosition);
}

/** One switch flipped: the flags with `permission` added or taken away, in
 * canonical order. */
export function togglePermission(
  permissions: readonly Permission[],
  permission: Permission,
): Permission[] {
  return permissions.includes(permission)
    ? permissions.filter((p) => p !== permission)
    : normalisePermissions([...permissions, permission]);
}

/** One checkbox of the member's role picker: the role ids with `id` added
 * or taken away. */
export function toggleMemberRole(
  roleIds: readonly string[],
  id: string,
): string[] {
  return roleIds.includes(id)
    ? roleIds.filter((role) => role !== id)
    : [...roleIds, id];
}

/** What "Neue Rolle" creates: the name (numbered when taken), the first
 * free colour and no flags. `name` is "Neue Rolle" in the interface's
 * language. */
export function newRoleInput(
  server: Roles,
  name: string,
): { name: string; color: RoleColor; permissions: Permission[] } {
  return {
    name: nextRoleName(server, name),
    color: nextColor(server),
    permissions: [],
  };
}

/** The role name field: its text and the cached name it last took over. */
export type RoleNameField = { text: string; synced: string };

/**
 * The name field when the cached name changes (a rename elsewhere, or the
 * answer to one's own): an untouched field follows it, so a blur cannot
 * send the old name back; text typed since keeps its place.
 */
export function syncRoleName(
  field: RoleNameField,
  incoming: string,
): RoleNameField {
  const untouched =
    field.text === field.synced || field.text.trim() === incoming;
  return { text: untouched ? incoming : field.text, synced: incoming };
}

/** A rename typed into the name field: `null` when nothing changed, the
 * field code when the name is not allowed, else the name to save. */
export function roleRename(
  server: Roles,
  role: Pick<ServerRole, "id" | "name">,
  raw: string,
): { code: FieldCode } | { name: string } | null {
  const name = raw.trim();
  if (name === role.name) return null;
  const others = server.roles
    .filter((other) => other.id !== role.id)
    .map((other) => other.name);
  const code = validateRoleName(name, others);
  return code ? { code } : { name };
}
