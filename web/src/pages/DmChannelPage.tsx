// `/d/$channelId`: same message pane as a text channel, peer in the header.

import { useParams } from "@tanstack/react-router";
import { useEffect } from "react";

import { MessagePane } from "../components/MessagePane.tsx";
import { PresenceAvatar } from "../components/PresenceAvatar.tsx";
import { Redirect } from "../components/Redirect.tsx";
import { TypingBar } from "../components/TypingBar.tsx";
import { useDm } from "../dms/queries.ts";
import { useLastDm } from "../dms/lastDm.ts";
import type { DirectMessage } from "../dms/types.ts";
import type { Member } from "../servers/types.ts";
import { presenceOf, usePresenceStore } from "../ws/live.ts";
import { useTypingInput } from "../ws/useLive.ts";

export function DmChannelPage() {
  const { channelId } = useParams({ from: "/workspace/d/$channelId" });
  const { data: dm, error, isPending } = useDm(channelId);
  const remember = useLastDm((s) => s.remember);

  useEffect(() => {
    if (dm) remember(dm.id);
  }, [dm, remember]);

  if (error) {
    return <Redirect to="/d" />;
  }
  if (isPending || !dm) return null;

  return <DmChat dm={dm} />;
}

function DmChat({ dm }: { dm: DirectMessage }) {
  const typing = useTypingInput(dm.id, dm.id, true);
  const members = [peerMember(dm)];
  const status = usePresenceStore((s) => presenceOf(s.byServer, dm.id, dm.peer.id));

  return (
    <div className="flex min-h-0 flex-1 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header className="flex h-12 shrink-0 items-center gap-2 border-b border-neutral-200 bg-white px-4">
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
