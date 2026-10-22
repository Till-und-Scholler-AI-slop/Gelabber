// The workspace's keyboard shortcuts (shellShortcuts.ts), listening on the
// whole page while the workspace is open.

import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef } from "react";

import { useUserId } from "../auth/scope.ts";
import { useDms } from "../dms/queries.ts";
import { useBadges } from "../messages/useBadge.ts";
import {
  lastChannelsFor,
  railChannel,
  useLastChannel,
} from "../servers/lastChannel.ts";
import { isPendingId, useServer, useServers } from "../servers/queries.ts";
import { channelOrder } from "../servers/rows.ts";
import { useVoiceRoster } from "../voice/roster.ts";
import {
  nextUnread,
  shellShortcut,
  stepIndex,
  type ShellShortcut,
} from "./shellShortcuts.ts";

function modalOpen(): boolean {
  try {
    return document.querySelector("dialog:modal") !== null;
  } catch {
    return false;
  }
}

/**
 * A shortcut runs only when nothing else used its keys first: a page's own
 * Strg+K (message search) or a field that handles Alt+↑ itself. Such a
 * listener may have been added after this one, so the decision waits until
 * the key has passed every listener: one more listener, added while the
 * event is on its way down, runs last on its way back up.
 */
function listenLast(
  onShortcut: (shortcut: ShellShortcut, event: KeyboardEvent) => void,
): () => void {
  const onCapture = (event: KeyboardEvent) => {
    if (event.repeat || event.isComposing) return;
    const shortcut = shellShortcut(event);
    if (!shortcut) return;
    const late = (bubbled: KeyboardEvent) => {
      window.removeEventListener("keydown", late);
      if (bubbled !== event || event.defaultPrevented || modalOpen()) return;
      event.preventDefault();
      onShortcut(shortcut, event);
    };
    window.addEventListener("keydown", late);
  };
  window.addEventListener("keydown", onCapture, true);
  return () => window.removeEventListener("keydown", onCapture, true);
}

export function useShellShortcuts({
  serverId,
  channelId,
  dms: onDms,
  onSwitcher,
  onHelp,
}: {
  serverId: string | undefined;
  channelId: string | undefined;
  /** The direct messages are open. */
  dms: boolean;
  onSwitcher: () => void;
  onHelp: () => void;
}) {
  const navigate = useNavigate();
  const userId = useUserId();
  const { data: servers } = useServers();
  const { data: server } = useServer(serverId);
  const { data: dms } = useDms();
  const badges = useBadges();
  const byUser = useLastChannel((s) => s.byUser);

  const run = (shortcut: ShellShortcut) => {
    if (shortcut.kind === "switcher") return onSwitcher();
    if (shortcut.kind === "help") return onHelp();
    if (shortcut.kind === "server") {
      const list = servers ?? [];
      const at = stepIndex(
        list.length,
        list.findIndex((row) => row.id === serverId),
        shortcut.step,
      );
      const next = at === null ? undefined : list[at];
      if (!next) return;
      const target = railChannel(
        lastChannelsFor(byUser, userId)[next.id],
        Object.keys(useVoiceRoster.getState().byServer[next.id] ?? {}).length >
          0,
      );
      void (target
        ? navigate({
            to: "/s/$serverId/c/$channelId",
            params: { serverId: next.id, channelId: target },
          })
        : navigate({ to: "/s/$serverId", params: { serverId: next.id } }));
      return;
    }
    // Channels of the server open, or the direct messages.
    const ids = server
      ? channelOrder(server)
          .map((channel) => channel.id)
          .filter((id) => !isPendingId(id))
      : onDms
        ? (dms ?? []).map((dm) => dm.id)
        : [];
    const index = channelId ? ids.indexOf(channelId) : -1;
    const at =
      shortcut.kind === "channel"
        ? stepIndex(ids.length, index, shortcut.step)
        : nextUnread(
            ids.map((id) => {
              const badge = badges.get(id);
              return Boolean(badge && (badge.count > 0 || badge.dot));
            }),
            index,
            shortcut.step,
          );
    const next = at === null ? undefined : ids[at];
    if (!next) return;
    void (server
      ? navigate({
          to: "/s/$serverId/c/$channelId",
          params: { serverId: server.id, channelId: next },
        })
      : navigate({ to: "/d/$channelId", params: { channelId: next } }));
  };
  const latest = useRef(run);
  useEffect(() => {
    latest.current = run;
  });
  useEffect(() => listenLast((shortcut) => latest.current(shortcut)), []);
}
