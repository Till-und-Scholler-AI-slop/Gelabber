// `/d`: no conversation picked yet — jump to the last DM when we still
// have it, otherwise offer to start one.

import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { useUserId } from "../auth/scope.ts";
import { DmRow } from "../components/DmSidebar.tsx";
import { PlusIcon } from "../components/Icons.tsx";
import { NewMessageDialog } from "../components/NewMessageDialog.tsx";
import { Redirect } from "../components/Redirect.tsx";
import { LoadError } from "../components/LoadError.tsx";
import { Button } from "../components/ui/Button.tsx";
import { MainSkeleton } from "../components/ui/Skeleton.tsx";
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
  const [writing, setWriting] = useState(false);

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
  if (isPending && !isError) return <MainSkeleton kind="list" />;
  if (isError)
    return (
      <div className="p-6">
        <LoadError
          message="Die Direktnachrichten konnten nicht geladen werden."
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      </div>
    );

  const start = (
    <>
      <Button
        icon={<PlusIcon size={16} />}
        variant={dms && dms.length > 0 ? "secondary" : "primary"}
        onClick={() => setWriting(true)}
      >
        Neue Nachricht
      </Button>
      <NewMessageDialog open={writing} onClose={() => setWriting(false)} />
    </>
  );

  if (dms && dms.length > 0)
    return (
      <div className="dm-index">
        {drawer ? (
          <nav aria-label="Unterhaltungen" className="dm-index-list">
            <div className="dm-index-list-heading">
              <h1>Nachrichten</h1>
              {start}
            </div>
            {dms.map((dm) => (
              <DmRow key={dm.id} dm={dm} active={false} />
            ))}
          </nav>
        ) : (
          <div className="dm-index-hint">
            <p>Wähle eine Unterhaltung.</p>
            {start}
          </div>
        )}
      </div>
    );

  return (
    <div className="dm-index">
      <div className="dm-index-hint">
        <h1>Keine Unterhaltungen</h1>
        {start}
      </div>
    </div>
  );
}
