// `/d/requests` (#563): message requests from people on the user's servers.
// Opening one shows the conversation with "Annehmen" / "Ablehnen" in place
// of the composer.

import { Link } from "@tanstack/react-router";

import { LoadError } from "../components/LoadError.tsx";
import { Avatar } from "../components/Avatar.tsx";
import { Button } from "../components/ui/Button.tsx";
import { MainSkeleton } from "../components/ui/Skeleton.tsx";
import { useT } from "../i18n/catalog.ts";
import { friends as friendsCatalog } from "../i18n/catalogs/friends.ts";
import {
  useAcceptDmRequest,
  useDeclineDmRequest,
  useDmRequests,
} from "./queries.ts";
import "./friends.css";

export function RequestsPage() {
  const t = useT(friendsCatalog);
  const { data, isPending, isError, isFetching, refetch } = useDmRequests();
  const accept = useAcceptDmRequest();
  const decline = useDeclineDmRequest();
  return (
    <div className="friends-page">
      <header className="friends-page__header">
        <h1>{t("requests.title")}</h1>
      </header>
      <div className="friends-page__body">
        <p className="friends-hint">{t("requests.hint")}</p>
        {isPending && !isError ? (
          <MainSkeleton kind="list" />
        ) : isError || !data ? (
          <LoadError
            message={t("requests.loadFailed")}
            pending={isFetching}
            onRetry={() => void refetch()}
          />
        ) : data.length === 0 ? (
          <p className="friends-empty">{t("requests.empty")}</p>
        ) : (
          <ul className="friend-list" aria-label={t("requests.title")}>
            {data.map((dm) => (
              <li key={dm.id} className="friend-row">
                <Avatar
                  name={dm.peer.name}
                  url={dm.peer.avatar_url}
                  decorative
                />
                <Link
                  className="friend-row__name friend-row__link"
                  to="/d/$channelId"
                  params={{ channelId: dm.id }}
                >
                  {dm.peer.name}
                </Link>
                <span className="friend-row__actions">
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={accept.isPending}
                    onClick={() => accept.mutate(dm.id)}
                  >
                    {t("action.accept")}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={decline.isPending}
                    onClick={() => decline.mutate(dm.id)}
                  >
                    {t("action.decline")}
                  </Button>
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

/**
 * In a conversation that is a message request: the answer bar for the
 * recipient, or the sender's note on what may still go.
 */
export function RequestBar({
  channelId,
  peerName,
  incoming,
  remaining,
}: {
  channelId: string;
  peerName: string;
  incoming: boolean;
  remaining: number;
}) {
  const t = useT(friendsCatalog);
  const accept = useAcceptDmRequest();
  const decline = useDeclineDmRequest();
  if (!incoming)
    return (
      <p className="request-bar request-bar--note" role="status">
        {remaining > 0
          ? t("bar.outgoing", { name: peerName, count: remaining })
          : t("bar.outgoingFull", { name: peerName })}
      </p>
    );
  return (
    <div className="request-bar" role="group" aria-label={t("requests.title")}>
      <p>{t("bar.incoming", { name: peerName })}</p>
      <div className="request-bar__actions">
        <Button
          variant="primary"
          disabled={accept.isPending}
          onClick={() => accept.mutate(channelId)}
        >
          {t("action.accept")}
        </Button>
        <Button
          variant="ghost"
          disabled={decline.isPending}
          onClick={() => decline.mutate(channelId)}
        >
          {t("action.decline")}
        </Button>
      </div>
    </div>
  );
}
