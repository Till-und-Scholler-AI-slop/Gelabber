// `/d`: no conversation picked yet — jump to the last DM when we still
// have it, otherwise explain how to start one.

import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { useUserId } from "../auth/scope.ts";
import { DmRow } from "../components/DmSidebar.tsx";
import { Redirect } from "../components/Redirect.tsx";
import { LoadError } from "../components/LoadError.tsx";
import { useMediaQuery } from "../components/useMediaQuery.ts";
import { NAVIGATION_DRAWER_MAX_WIDTH } from "../components/WorkspaceNavigation.tsx";
import { lastDmId, useLastDm } from "../dms/lastDm.ts";
import {
  isGoneError,
  lastDmStillListed,
  shouldOpenLastDm,
} from "../dms/open.ts";
import { forgetDm, useDm, useDms } from "../dms/queries.ts";

export function DmIndexPage() {
  const client = useQueryClient();
  const userId = useUserId();
  const { data: dms, isPending, isError, isFetching, refetch } = useDms();
  const byUser = useLastDm((s) => s.byUser);
  const lastId = lastDmId(byUser, userId);
  const forgetLast = useLastDm((s) => s.forget);
  // Read-only: do not refetch last-DM from the index. A failed detail
  // stays in cache and must block the auto-open Redirect.
  const lastDetail = useDm(lastId ?? undefined, false);
  const detailFailed = Boolean(lastId && lastDetail.isError);
  // Where the sidebar sits in a closed drawer, this page lists the DMs. Only
  // there: each row subscribes to presence and unread state of its own.
  const drawer = useMediaQuery(`(max-width: ${NAVIGATION_DRAWER_MAX_WIDTH}px)`);

  useEffect(() => {
    if (!lastId) return;
    if (detailFailed && isGoneError(lastDetail.error)) {
      forgetDm(client, userId, lastId, { keepDetail: true });
      forgetLast(userId, lastId);
      return;
    }
    if (dms && !lastDmStillListed(lastId, dms)) {
      forgetLast(userId, lastId);
    }
  }, [lastId, dms, detailFailed, lastDetail.error, client, forgetLast, userId]);

  if (lastId && shouldOpenLastDm(lastId, dms, detailFailed)) {
    return <Redirect to="/d/$channelId" params={{ channelId: lastId }} />;
  }
  if (isPending && !isError) return null;
  if (isError)
    return (
      <div className="p-6">
        <LoadError
          message="Deine Direktnachrichten konnten gerade nicht geladen werden."
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      </div>
    );

  if (dms && dms.length > 0)
    return (
      <div className="dm-index">
        {drawer ? (
          <nav aria-label="Unterhaltungen" className="dm-index-list">
            <h1>Nachrichten</h1>
            {dms.map((dm) => (
              <DmRow key={dm.id} dm={dm} active={false} />
            ))}
          </nav>
        ) : (
          <p className="dm-index-hint">Wähle eine Unterhaltung.</p>
        )}
      </div>
    );

  return (
    <div className="flex flex-1 items-center justify-center p-8 text-center text-neutral-500 dark:text-neutral-400">
      <p>Öffne eine Direktnachricht über die Mitgliederliste eines Servers.</p>
    </div>
  );
}
