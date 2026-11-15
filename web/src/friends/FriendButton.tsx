// A profile card's friend button (#563): add, the request already sent,
// accept theirs, or "Befreundet".

import { useUserId } from "../auth/scope.ts";
import { Button } from "../components/ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { friends as friendsCatalog } from "../i18n/catalogs/friends.ts";
import {
  relationTo,
  useAcceptFriend,
  useFriends,
  useSendFriendRequest,
} from "./queries.ts";

export function FriendButton({ userId }: { userId: string }) {
  const t = useT(friendsCatalog);
  const me = useUserId();
  const { data } = useFriends();
  const send = useSendFriendRequest();
  const accept = useAcceptFriend();
  const relation = relationTo(data, me, userId);
  if (relation === undefined || relation === "self") return null;
  const busy = send.isPending || accept.isPending;
  switch (relation) {
    case "friend":
      return (
        <Button size="sm" variant="ghost" disabled>
          {t("card.friends")}
        </Button>
      );
    case "outgoing":
      return (
        <Button size="sm" variant="ghost" disabled>
          {t("card.pending")}
        </Button>
      );
    case "incoming":
      return (
        <Button size="sm" disabled={busy} onClick={() => accept.mutate(userId)}>
          {t("card.incoming")}
        </Button>
      );
    case "none":
      return (
        <Button
          size="sm"
          disabled={busy}
          onClick={() => send.mutate({ user_id: userId })}
        >
          {t("card.add")}
        </Button>
      );
  }
}
