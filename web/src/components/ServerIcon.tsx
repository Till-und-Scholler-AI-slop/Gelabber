import { useState, type ReactNode } from "react";

/**
 * The face of a server tile: its uploaded icon, or `fallback` (the
 * initials) without one or when the image does not load. Decorative: the
 * tile around it carries the server's name.
 */
export function ServerIconFace({
  url,
  fallback,
}: {
  url: string | null | undefined;
  fallback: ReactNode;
}) {
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  if (!url || failedUrl === url) return <>{fallback}</>;
  return (
    <img
      src={url}
      alt=""
      className="server-icon-img"
      draggable={false}
      onError={() => setFailedUrl(url)}
    />
  );
}
