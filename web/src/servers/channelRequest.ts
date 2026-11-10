// The request "Kanal erstellen" and "Kanal bearbeiten" send
// (components/ServerDialogs.tsx).

import { accessRoleIds, type AccessSelection } from "./roles.ts";
import type {
  Channel,
  ChannelKind,
  CreateChannelInput,
  UpdateChannelInput,
} from "./types.ts";

/** What the channel form's submit sends: a patch of the channel, or the
 * new channel. Access goes along only when it changed: `[]` would open an
 * owner-only channel. */
export function channelRequest(
  editing: Pick<Channel, "id"> | null,
  form: {
    name: string;
    kind: ChannelKind;
    categoryId: string;
    initialAccess: AccessSelection;
    access: AccessSelection;
  },
):
  | { mode: "edit"; id: string; patch: UpdateChannelInput }
  | { mode: "create"; input: CreateChannelInput } {
  const role_ids = accessRoleIds(form.initialAccess, form.access);
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
