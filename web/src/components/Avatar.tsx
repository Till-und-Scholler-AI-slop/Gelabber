import { useState } from "react";

import { initials } from "./initials.ts";

export type AvatarSize = "sm" | "md" | "lg" | "hero" | "room";

type AvatarProps = {
  name: string;
  url: string | null;
  size?: AvatarSize;
  className?: string;
  /** The name is written right next to it: hide the picture from screen
   * readers instead of reading the name twice. */
  decorative?: boolean;
};

/** Image when there is a working URL, initials otherwise — never a broken image icon. */
export function Avatar({
  name,
  url,
  size = "sm",
  className = "",
  decorative = false,
}: AvatarProps) {
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const showImage = url !== null && url.length > 0 && failedUrl !== url;

  return (
    <span
      className={`gel-avatar gel-avatar-${size} ${className}`}
      aria-hidden={showImage && !decorative ? undefined : true}
    >
      {showImage ? (
        <img
          src={url}
          alt={decorative ? "" : `Avatar von ${name}`}
          // Avatar URLs may point anywhere (#208); do not tell that host
          // which Gelabber page the viewer is on.
          referrerPolicy="no-referrer"
          onError={() => setFailedUrl(url)}
        />
      ) : (
        initials(name)
      )}
    </span>
  );
}
