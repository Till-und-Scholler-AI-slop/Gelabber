import type { CSSProperties } from "react";

import { roleColorVar } from "../servers/roles.ts";
import type { RoleColor } from "../servers/types.ts";
import "./roleDot.css";

/** A role's dot. The owner's is the accent, the base role's muted. */
export function RoleDot({ color }: { color: RoleColor | "owner" | "base" }) {
  const background =
    color === "owner"
      ? "var(--lr-accent)"
      : color === "base"
        ? "var(--lr-muted)"
        : roleColorVar(color);
  return (
    <span
      className="role-dot"
      style={{ background } as CSSProperties}
      aria-hidden="true"
    />
  );
}
