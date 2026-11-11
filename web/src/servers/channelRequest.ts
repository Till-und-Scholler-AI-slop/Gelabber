// The request "Kanal erstellen" and "Kanal bearbeiten" send
// (components/ServerDialogs.tsx).

import { accessRoleIds, sameAccess, type AccessSelection } from "./roles.ts";
import type {
  Channel,
  ChannelKind,
  CreateChannelInput,
  ServerRole,
  UpdateChannelInput,
} from "./types.ts";

/** A selection without the roles deleted since the dialog opened. */
function liveAccess(
  selection: AccessSelection,
  roles: readonly Pick<ServerRole, "id">[],
): AccessSelection {
  if (selection === null) return null;
  const live = new Set(roles.map((role) => role.id));
  return selection.filter((id) => live.has(id));
}

/** What the channel form's submit sends: a patch of the channel, or the
 * new channel. Access goes along only when it changed: `[]` would open an
 * owner-only channel. Roles deleted while the dialog was open drop out;
 * when every chosen role is gone, the ids go along so the server refuses
 * rather than the channel opening. */
export function channelRequest(
  editing: Pick<Channel, "id"> | null,
  form: {
    name: string;
    kind: ChannelKind;
    categoryId: string;
    initialAccess: AccessSelection;
    access: AccessSelection;
    /** The server's roles now. */
    roles: readonly Pick<ServerRole, "id">[];
  },
):
  | { mode: "edit"; id: string; patch: UpdateChannelInput }
  | { mode: "create"; input: CreateChannelInput } {
  const initial = liveAccess(form.initialAccess, form.roles);
  const access = liveAccess(form.access, form.roles);
  const gone =
    access !== null &&
    access.length === 0 &&
    form.access !== null &&
    form.access.length > 0 &&
    !sameAccess(initial, access);
  const role_ids = gone ? [...form.access!] : accessRoleIds(initial, access);
  const category_id = form.categoryId || null;
  if (editing)
    return {
      mode: "edit",
      id: editing.id,
      patch: { name: form.name, category_id, role_ids },
    };
  return {
    mode: "create",
    input: { name: form.name, kind: form.kind, category_id, role_ids },
  };
}
