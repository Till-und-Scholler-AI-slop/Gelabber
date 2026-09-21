// `/d/$channelId`: same message pane as a text channel, peer in the header.

import { useNavigate, useParams } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { MessagePane } from "../components/MessagePane.tsx";
import { PresenceAvatar } from "../components/PresenceAvatar.tsx";
import { TypingBar } from "../components/TypingBar.tsx";
import { useLastDm } from "../dms/lastDm.ts";
import { isGoneError } from "../dms/open.ts";
import { forgetDm, useDm } from "../dms/queries.ts";
import type { DirectMessage } from "../dms/types.ts";
import type { Member } from "../servers/types.ts";
import { presenceOf, usePresenceStore } from "../ws/live.ts";
import { useTypingInput } from "../ws/useLive.ts";

export function DmChannelPage() {
  const { channelId } = useParams({ from: "/workspace/d/$channelId" });
  const client = useQueryClient();
  const navigate = useNavigate();
  const { data: dm, error, isPending } = useDm(channelId);
  const remember = useLastDm((s) => s.remember);
  const forgetLast = useLastDm((s) => s.forget);
  const gone = isGoneError(error);

  useEffect(() => {
    if (dm) remember(dm.id);
  }, [dm, remember]);

  useEffect(() => {
    if (!gone) return;
    // Same as a vanished server: drop the list row + last-DM, leave, then
    // drop the detail so this still-mounted query does not refetch.
    forgetDm(client, channelId, { keepDetail: true });
    forgetLast(channelId);
    void navigate({ to: "/d", replace: true }).then(() =>
      forgetDm(client, channelId),
    );
  }, [gone, client, channelId, forgetLast, navigate]);

  if (gone) return null;
  if (error) {
    return (
      <div className="flex flex-1 items-center justify-center p-8 text-center text-neutral-500 dark:text-neutral-400">
        <p>Diese Unterhaltung konnte gerade nicht geladen werden.</p>
      </div>
    );
  }
  if (isPending || !dm) return null;

  return <DmChat dm={dm} />;
}

function DmChat({ dm }: { dm: DirectMessage }) {
  const typing = useTypingInput(dm.id, dm.id, true);
  const members = [peerMember(dm)];
  const status = usePresenceStore((s) =>
    presenceOf(s.byServer, dm.id, dm.peer.id),
  );

  return (
    <div className="flex min-h-0 flex-1 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header className="flex h-12 shrink-0 items-center gap-2 border-b border-neutral-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-4">
          <PresenceAvatar
            name={dm.peer.name}
            url={dm.peer.avatar_url}
            status={status}
          />
          <h1 className="truncate font-semibold tracking-tight">
            {dm.peer.name}
          </h1>
        </header>
        <div className="relative min-h-0 flex-1">
          <div
            data-testid="message-pane"
            className="absolute inset-0 flex min-h-0 flex-col"
          >
            <MessagePane
              key={dm.id}
              channelId={dm.id}
              channelName={dm.peer.name}
              canSend
              canSendFiles
              mention="@"
              footer={<TypingBar channelId={dm.id} members={members} />}
              onDraftChange={typing.onChange}
              onDraftStop={typing.stop}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

function peerMember(dm: DirectMessage): Member {
  return {
    user_id: dm.peer.id,
    name: dm.peer.name,
    avatar_url: dm.peer.avatar_url,
    joined_at: dm.created_at,
    role: "member",
  };
}
