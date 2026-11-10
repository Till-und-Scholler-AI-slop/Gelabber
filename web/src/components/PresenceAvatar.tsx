import type { PresenceStatus } from "../ws/protocol.ts";
import type { AvatarDecoration } from "../auth/types.ts";
import { Avatar, type AvatarSize } from "./Avatar.tsx";
import { AvatarFrame } from "./AvatarFrame.tsx";
import { useT } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";

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
  const t = useT(shell);
  return (
    <span className={`gel-presence-avatar ${className}`}>
      <AvatarFrame decoration={decoration} accent={accent}>
        <Avatar name={name} url={url} size={size} />
      </AvatarFrame>
      <span
        className={`gel-presence-dot gel-presence-${status}`}
        title={t(`presence.${status}`)}
        aria-label={t(`presence.${status}`)}
      />
    </span>
  );
}
