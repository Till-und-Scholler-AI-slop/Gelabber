// The main area of a server an operator blocked (#517): it says so, and
// offers the one thing still possible there, leaving it (members) or
// deleting it (the owner), through the confirmations of the server settings.

import { useNavigate } from "@tanstack/react-router";

import { useSession } from "../auth/session.ts";
import { confirmAction } from "../components/confirm.ts";
import { Button } from "../components/ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { serverSettings } from "../i18n/catalogs/serverSettings.ts";
import { useLastChannel } from "./lastChannel.ts";
import { useDeleteServer, useLeaveServer } from "./queries.ts";
import type { Server } from "./types.ts";

import "../pages/auth.css";

/** `server`: the list entry, when the list has loaded; without it the
 * notice offers no action, since it cannot tell owner from member. */
export function BlockedServerNotice({
  serverId,
  server,
}: {
  serverId: string;
  server: Server | undefined;
}) {
  const t = useT(serverSettings);
  const ta = useT(auth);
  const navigate = useNavigate();
  const user = useSession((s) => s.user);
  const forget = useLastChannel((s) => s.forget);
  const remove = useDeleteServer();
  const leave = useLeaveServer();
  const owner = server?.role === "owner";

  const act = async () => {
    if (!server || !user) return;
    const name = server.name;
    if (owner) {
      if (
        !(await confirmAction({
          title: t("danger.deleteTitle", { name }),
          message: t("danger.deleteMessage"),
          confirmLabel: t("danger.delete"),
        }))
      )
        return;
      remove.mutate(serverId);
    } else {
      if (
        !(await confirmAction({
          title: t("danger.leaveTitle", { name }),
          confirmLabel: t("danger.leaveConfirm"),
        }))
      )
        return;
      leave.mutate(serverId);
    }
    // Optimistic, as in the settings: the server is already off the rail.
    forget(user.id, serverId);
    void navigate({ to: "/", replace: true });
  };

  return (
    <section className="lr-auth-card mx-auto my-10 w-full max-w-sm">
      {server ? (
        <>
          <h1>{server.name}</h1>
          <p className="auth-lead">{ta("error.serverBlocked")}</p>
        </>
      ) : (
        <h1>{ta("error.serverBlocked")}</h1>
      )}
      {server && user ? (
        <div className="auth-actions">
          <Button variant="danger-outline" onClick={() => void act()}>
            {owner ? t("danger.delete") : t("danger.leave")}
          </Button>
        </div>
      ) : null}
    </section>
  );
}
