// `/d`: no conversation picked yet — jump to the last DM when we still
// have it, otherwise explain how to start one.

import { Redirect } from "../components/Redirect.tsx";
import { useDms } from "../dms/queries.ts";
import { useLastDm } from "../dms/lastDm.ts";

export function DmIndexPage() {
  const { data: dms, isPending, isError } = useDms();
  const lastId = useLastDm((s) => s.channelId);
  const remembered = lastId
    ? dms?.find((dm) => dm.id === lastId)
    : undefined;
  if (remembered) {
    return (
      <Redirect to="/d/$channelId" params={{ channelId: remembered.id }} />
    );
  }
  if (isPending && !isError) return null;

  return (
    <div className="flex flex-1 items-center justify-center p-8 text-center text-neutral-500">
      <p>
        {isError
          ? "Deine Direktnachrichten konnten gerade nicht geladen werden."
          : dms && dms.length > 0
            ? "Wähle eine Unterhaltung."
            : "Öffne eine Direktnachricht über die Mitgliederliste eines Servers."}
      </p>
    </div>
  );
}
