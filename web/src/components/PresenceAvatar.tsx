import type { PresenceStatus } from "../ws/protocol.ts";
import type { AvatarDecoration } from "../auth/types.ts";
import { Avatar, type AvatarSize } from "./Avatar.tsx";
import { AvatarFrame } from "./AvatarFrame.tsx";

const LABEL: Record<PresenceStatus, string> = {
  o: "Online",
  i: "Abwesend",
  x: "Offline",
};

export function PresenceAvatar({
  name,
  url,
  status,
  size = "sm",
  className = "",
  decoration,
  accent,
}: {
  name: string;
  url: string | null;
  status: PresenceStatus;
  size?: AvatarSize;
  className?: string;
  decoration?: AvatarDecoration | null | undefined;
  accent?: string | null | undefined;
}) {
  return (
    <span className={`gel-presence-avatar ${className}`}>
      <AvatarFrame decoration={decoration} accent={accent}>
        <Avatar name={name} url={url} size={size} />
      </AvatarFrame>
      <span
        className={`gel-presence-dot gel-presence-${status}`}
        title={LABEL[status]}
        aria-label={LABEL[status]}
      />
    </span>
  );
}
