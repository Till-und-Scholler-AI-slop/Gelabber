import { useState } from "react";

import { initials } from "./initials.ts";

export type AvatarSize = "sm" | "md" | "lg" | "hero" | "room";

type AvatarProps = {
  name: string;
  url: string | null;
  size?: AvatarSize;
  className?: string;
};

/** Image when there is a working URL, initials otherwise — never a broken image icon. */
export function Avatar({
  name,
  url,
  size = "sm",
  className = "",
}: AvatarProps) {
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const showImage = url !== null && url.length > 0 && failedUrl !== url;

  return (
    <span
      className={`gel-avatar gel-avatar-${size} ${className}`}
      aria-hidden={showImage ? undefined : true}
    >
      {showImage ? (
        <img
          src={url}
          alt={`Avatar von ${name}`}
          onError={() => setFailedUrl(url)}
        />
      ) : (
        initials(name)
      )}
    </span>
  );
}
