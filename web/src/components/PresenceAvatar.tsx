import type { PresenceStatus } from "../ws/protocol.ts";
import { Avatar, type AvatarSize } from "./Avatar.tsx";

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
}: {
  name: string;
  url: string | null;
  status: PresenceStatus;
  size?: AvatarSize;
  className?: string;
}) {
  return (
    <span className={`gel-presence-avatar ${className}`}>
      <Avatar name={name} url={url} size={size} />
      <span
        className={`gel-presence-dot gel-presence-${status}`}
        title={LABEL[status]}
        aria-label={LABEL[status]}
      />
    </span>
  );
}
