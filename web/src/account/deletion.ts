// Deleting the account (#519): which servers stand in the way, and how
// the dialog confirms it is the person.

import { ApiError } from "../api/client.ts";
import { needsReauth } from "../auth/signInMethods.ts";
import { deleteAccount } from "../auth/session.ts";
import type { Server } from "../servers/types.ts";
import { leaveVoice } from "../voice/session.ts";

/** A server the account still owns, as `409 owns_servers` lists it. The
 * server list has no member count, so rows taken from it may lack one. */
export type OwnedServer = { id: string; name: string; member_count?: number };

/** The servers the list says this account owns. */
export function ownedServers(servers: readonly Server[] | undefined) {
  return (servers ?? [])
    .filter((server) => server.role === "owner")
    .map(({ id, name }): OwnedServer => ({ id, name }));
}

/** The servers of a `409 owns_servers`, which outranks the list. */
export function ownedServersOf(error: unknown): OwnedServer[] | null {
  if (!(error instanceof ApiError) || error.code !== "owns_servers")
    return null;
  const raw = error.body.servers;
  if (!Array.isArray(raw)) return [];
  return raw.flatMap((entry: unknown): OwnedServer[] => {
    if (entry === null || typeof entry !== "object") return [];
    const { id, name, member_count } = entry as Record<string, unknown>;
    if (typeof id !== "string" || typeof name !== "string") return [];
    return [
      typeof member_count === "number"
        ? { id, name, member_count }
        : { id, name },
    ];
  });
}

/** How the person confirms: the password, or the account's address when
 * the account has none (it signs in only through a provider). */
export type DeleteConfirmation = { password: string } | { email: string };

/**
 * Deletes the account, then leaves as a logout does: the call ends (its
 * session is gone already) and the sign-in page says why.
 */
export async function deleteAndLeave(
  confirmation: DeleteConfirmation,
  navigate: (options: {
    to: "/login";
    search: { reason: "deleted" };
  }) => unknown,
): Promise<void> {
  await deleteAccount(confirmation);
  leaveVoice();
  void navigate({ to: "/login", search: { reason: "deleted" } });
}

/** What the dialog does with a refused deletion. */
export type DeletionRefusal =
  | { kind: "owned"; servers: OwnedServer[] }
  | { kind: "reauth" }
  | { kind: "emailWrong" }
  | { kind: "other" };

export function deletionRefusal(error: unknown): DeletionRefusal {
  const servers = ownedServersOf(error);
  if (servers) return { kind: "owned", servers };
  if (needsReauth(error)) return { kind: "reauth" };
  // Any refusal of the address but "required" means it is not this one.
  const email = error instanceof ApiError ? error.fields.email : undefined;
  if (email && email !== "required") return { kind: "emailWrong" };
  return { kind: "other" };
}
