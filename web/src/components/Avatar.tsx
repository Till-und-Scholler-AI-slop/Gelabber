import { useState } from "react";

import { initials } from "./initials.ts";

type AvatarProps = {
  name: string;
  url: string | null;
  size?: "sm" | "lg";
};

/** Image when there is a working URL, initials otherwise — never a broken image icon. */
export function Avatar({ name, url, size = "sm" }: AvatarProps) {
  // Remember which URL failed so a newly typed one gets a fresh attempt.
  const [failedUrl, setFailedUrl] = useState<string | null>(null);

  const box = size === "lg" ? "size-20 text-2xl" : "size-8 text-xs";
  const showImage = url !== null && url.length > 0 && failedUrl !== url;

  return (
    <span
      className={`${box} inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full bg-neutral-200 dark:bg-neutral-700 font-semibold text-neutral-700 dark:text-neutral-300 select-none`}
      aria-hidden={showImage ? undefined : true}
    >
      {showImage ? (
        <img
          src={url}
          alt={`Avatar von ${name}`}
          className="size-full object-cover"
          onError={() => setFailedUrl(url)}
        />
      ) : (
        initials(name)
      )}
    </span>
  );
}
