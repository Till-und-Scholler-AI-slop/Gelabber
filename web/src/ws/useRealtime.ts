// Apply sequenced chat/server events and kick/ban errors to the query
// cache. Message deletes land immediately; the list pin lives in MessagePane.

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";

import { useSession } from "../auth/session.ts";
import { notify } from "../components/toasts.ts";
import { applyChannelEvent } from "../messages/queries.ts";
import { applyMemberRemoved, forgetServer } from "../servers/queries.ts";
import { leaveVoice, stopWatching, useVoice } from "../voice/session.ts";
import { getGateway } from "./client.ts";
import { shouldLeaveView } from "./leaveView.ts";

/** Kick/ban closes both planes before the sidebar that hosts voice unmounts. */
function dropVoice(serverId: string): void {
  const voice = useVoice.getState();
  if (voice.status === "joined" && voice.serverId === serverId) {
    leaveVoice();
  }
  if (voice.watching && voice.watchServerId === serverId) {
    stopWatching();
  }
}

export function useRealtimeBridge(
  viewingServerId?: string,
  onSelfRemoved?: (serverId: string) => void,
): void {
  const client = useQueryClient();
  const navigate = useNavigate();
  const me = useSession((s) => s.user?.id);

  useEffect(() => {
    const leave = (reason: "kicked" | "banned", serverId: string) => {
      dropVoice(serverId);
      onSelfRemoved?.(serverId);
      notify(
        reason === "banned"
          ? "Du wurdest vom Server gesperrt."
          : "Du wurdest vom Server entfernt.",
      );
      if (shouldLeaveView(viewingServerId, serverId)) {
        forgetServer(client, serverId, { keepDetail: true });
        void navigate({ to: "/", replace: true }).then(() =>
          forgetServer(client, serverId),
        );
        return;
      }
      forgetServer(client, serverId);
    };

    const gateway = getGateway();
    const offEvent = gateway.onEvent((event) => {
      if (event.c) {
        applyChannelEvent(client, event);
        return;
      }
      if (event.t === "d" && event.i) {
        const result = applyMemberRemoved(client, event.s, event.i, me);
        if (result === "self") {
          const banned =
            event.d !== null &&
            typeof event.d === "object" &&
            "k" in event.d &&
            event.d.k === "b";
          leave(banned ? "banned" : "kicked", event.s);
        }
      }
    });
    const offErr = gateway.onErr((err) => {
      if ((err.e === "kicked" || err.e === "banned") && err.s) {
        leave(err.e, err.s);
      }
    });
    return () => {
      offEvent();
      offErr();
    };
  }, [client, me, navigate, onSelfRemoved, viewingServerId]);
}
