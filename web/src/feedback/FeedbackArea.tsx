// Einstellungen → Feedback: every entry the person sent, as small threads
// with the operators' replies. Entries with unread replies are marked read
// once shown; their "Neu" marks stay until the person leaves the page.

import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";

import { takeStamp } from "../auth/scope.ts";
import { LoadError } from "../components/LoadError.tsx";
import { Button } from "../components/ui/Button.tsx";
import { SkeletonRows } from "../components/ui/Skeleton.tsx";
import { useT } from "../i18n/catalog.ts";
import { feedback } from "../i18n/catalogs/feedback.ts";
import { openFeedbackDialog } from "./dialogStore.ts";
import { FeedbackThread } from "./FeedbackThread.tsx";
import { markFeedbackRead, useFeedbackList } from "./queries.ts";
import { entriesToRead, rememberNew } from "./thread.ts";
import type { FeedbackEntry } from "./types.ts";
import "./feedback.css";

/** The messages that were new when this visit showed them: a refetch
 * after the read calls answers `new: false`, and the marks stay. */
function useVisitMarks(entries: readonly FeedbackEntry[]) {
  const [marks, setMarks] = useState<ReadonlySet<string>>(() => new Set());
  const next = rememberNew(marks, entries);
  if (next !== marks) setMarks(next);
  return next;
}

/** `POST /api/feedback/{id}/read` for each unread entry on screen. */
function useReadOnSight(entries: readonly FeedbackEntry[]) {
  const client = useQueryClient();
  const asked = useRef(new Set<string>());
  useEffect(() => {
    const stamp = takeStamp();
    if (!stamp) return;
    for (const { id, through } of entriesToRead(entries, asked.current)) {
      asked.current.add(id);
      // A failed call is tried again with the next list it shows up in.
      markFeedbackRead(client, stamp, id, through).catch(() =>
        asked.current.delete(id),
      );
    }
  }, [client, entries]);
}

export function FeedbackArea() {
  const t = useT(feedback);
  const list = useFeedbackList();
  const entries = useMemo(
    () => list.data?.pages.flatMap((page) => page.entries) ?? [],
    [list.data],
  );
  const marks = useVisitMarks(entries);
  useReadOnSight(entries);

  let body;
  if (list.isPending) body = <SkeletonRows count={3} />;
  else if (list.isError && entries.length === 0)
    body = (
      <LoadError
        message={t("area.loadFailed")}
        pending={list.isFetching}
        onRetry={() => void list.refetch()}
      />
    );
  else if (entries.length === 0)
    body = (
      <div className="feedback-empty">
        <p>{t("area.empty")}</p>
        <p className="ui-hint">{t("area.emptyHint")}</p>
      </div>
    );
  else
    body = (
      <>
        <div className="feedback-entries">
          {entries.map((entry) => (
            <FeedbackThread
              key={entry.id}
              entry={entry}
              isNew={(id) => marks.has(id)}
            />
          ))}
        </div>
        {list.hasNextPage ? (
          <div className="feedback-more">
            <Button
              pending={list.isFetchingNextPage}
              pendingLabel={t("area.more")}
              onClick={() => void list.fetchNextPage()}
            >
              {t("area.more")}
            </Button>
          </div>
        ) : null}
      </>
    );

  return (
    <div className="feedback-area">
      <div className="feedback-area__actions">
        <Button variant="primary" onClick={openFeedbackDialog}>
          {t("area.give")}
        </Button>
      </div>
      {body}
    </div>
  );
}
