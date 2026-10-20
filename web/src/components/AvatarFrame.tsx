import type { ReactNode } from "react";

import type { AvatarDecoration } from "../auth/types.ts";
import { accentStyle, isDecoration } from "./profileStyle.ts";

/**
 * Draws the chosen avatar decoration around `children` (an `Avatar`) in the
 * person's accent color. Pure CSS, so nothing is fetched for it; without a
 * decoration the avatar renders untouched.
 */
export function AvatarFrame({
  decoration,
  accent,
  children,
}: {
  decoration: AvatarDecoration | null | undefined;
  accent?: string | null | undefined;
  children: ReactNode;
}) {
  if (!isDecoration(decoration)) return <>{children}</>;
  return (
    <span
      className={`gel-deco gel-deco-${decoration}`}
      style={accentStyle(accent)}
    >
      {children}
    </span>
  );
}
