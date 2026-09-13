// `/d`: no conversation picked yet — jump to the last DM when we still
// have it, otherwise explain how to start one.

import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { Redirect } from "../components/Redirect.tsx";
import { useLastDm } from "../dms/lastDm.ts";
import { isGoneError, lastDmStillListed, shouldOpenLastDm } from "../dms/open.ts";
import { forgetDm, useDm, useDms } from "../dms/queries.ts";

export function DmIndexPage() {
  const client = useQueryClient();
  const { data: dms, isPending, isError } = useDms();
  const lastId = useLastDm((s) => s.channelId);
  const forgetLast = useLastDm((s) => s.forget);
  // Read-only: do not refetch last-DM from the index. A failed detail
  // stays in cache and must block the auto-open Redirect.
  const lastDetail = useDm(lastId ?? undefined, false);
  const detailFailed = Boolean(lastId && lastDetail.isError);

  useEffect(() => {
    if (!lastId) return;
    if (detailFailed && isGoneError(lastDetail.error)) {
      forgetDm(client, lastId, { keepDetail: true });
      forgetLast(lastId);
      return;
    }
    if (dms && !lastDmStillListed(lastId, dms)) {
      forgetLast(lastId);
    }
  }, [lastId, dms, detailFailed, lastDetail.error, client, forgetLast]);

  if (lastId && shouldOpenLastDm(lastId, dms, detailFailed)) {
    return <Redirect to="/d/$channelId" params={{ channelId: lastId }} />;
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
