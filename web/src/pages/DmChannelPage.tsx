// `/d/$channelId`: same message pane as a text channel, peer in the header.

import { useNavigate, useParams } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { useUserId } from "../auth/scope.ts";
import { MessagePane } from "../components/MessagePane.tsx";
import { LoadError } from "../components/LoadError.tsx";
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
  const userId = useUserId();
  const { data: dm, error, isPending, isFetching, refetch } = useDm(channelId);
  const remember = useLastDm((s) => s.remember);
  const forgetLast = useLastDm((s) => s.forget);
  const gone = isGoneError(error);

  useEffect(() => {
    if (dm && userId) remember(userId, dm.id);
  }, [dm, remember, userId]);

  useEffect(() => {
    if (!gone) return;
    // Same as a vanished server: drop the list row + last-DM, leave, then
    // drop the detail so this still-mounted query does not refetch.
    if (!userId) return;
    forgetDm(client, userId, channelId, { keepDetail: true });
    forgetLast(userId, channelId);
    void navigate({ to: "/d", replace: true }).then(() =>
      forgetDm(client, userId, channelId),
    );
  }, [gone, client, channelId, forgetLast, navigate, userId]);

  if (gone) return null;
  if (error) {
    return (
      <div className="flex flex-1 items-center justify-center p-8 text-center text-neutral-500 dark:text-neutral-400">
        <LoadError
          message="Diese Unterhaltung konnte gerade nicht geladen werden."
          pending={isFetching}
          onRetry={() => void refetch()}
        />
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
      <div className="lr-channel-content flex min-w-0 flex-1 flex-col overflow-hidden">
        <div className="relative min-h-0 flex-1">
          <div
            data-testid="message-pane"
            className="absolute inset-0 flex min-h-0 flex-col"
          >
            <MessagePane
              key={dm.id}
              title={
                <>
                  <PresenceAvatar
                    name={dm.peer.name}
                    url={dm.peer.avatar_url}
                    status={status}
                  />
                  <h1>{dm.peer.name}</h1>
                </>
              }
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
