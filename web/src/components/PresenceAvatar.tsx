import type { PresenceStatus } from "../ws/protocol.ts";
import { Avatar } from "./Avatar.tsx";

const LABEL: Record<PresenceStatus, string> = {
  o: "Online",
  i: "Abwesend",
  x: "Offline",
};

const DOT: Record<PresenceStatus, string> = {
  o: "bg-emerald-500",
  i: "bg-amber-400",
  x: "bg-neutral-400",
};

export function PresenceAvatar({
  name,
  url,
  status,
}: {
  name: string;
  url: string | null;
  status: PresenceStatus;
}) {
  return (
    <span className="relative inline-flex shrink-0">
      <Avatar name={name} url={url} />
      <span
        className={[
          "absolute right-0 bottom-0 size-2.5 rounded-full ring-2 ring-white",
          DOT[status],
        ].join(" ")}
        title={LABEL[status]}
        aria-label={LABEL[status]}
      />
    </span>
  );
}
