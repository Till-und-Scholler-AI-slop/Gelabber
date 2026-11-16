// Einstellungen → Blockiert (#565): whom the user blocks, each with
// "Blockierung aufheben" (no question: it can be undone at once).

import { ActionRow } from "../components/ActionRow.tsx";
import { LoadError } from "../components/LoadError.tsx";
import { Button } from "../components/ui/Button.tsx";
import { SkeletonRows } from "../components/ui/Skeleton.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import { useT } from "../i18n/catalog.ts";
import { blocks as blocksCatalog } from "../i18n/catalogs/blocks.ts";
import { formatDate } from "../i18n/format.ts";
import { useBlocks, useUnblock } from "./queries.ts";
import type { BlockedUser } from "./types.ts";

export function BlockedArea() {
  const t = useT(blocksCatalog);
  const query = useBlocks();
  return (
    <section className="settings-group settings-action-section">
      <p className="settings-hint">{t("area.hint")}</p>
      <ul className="settings-action-list">
        {query.data ? (
          query.data.length === 0 ? (
            <li className="settings-action-list__state settings-hint">
              {t("area.empty")}
            </li>
          ) : (
            query.data.map((person) => (
              <BlockedRow key={person.id} person={person} />
            ))
          )
        ) : query.isError ? (
          <li className="settings-action-list__state">
            <LoadError
              message={t("area.loadFailed")}
              pending={query.isFetching}
              onRetry={() => void query.refetch()}
            />
          </li>
        ) : (
          <li className="settings-action-list__state">
            <SkeletonRows count={2} />
          </li>
        )}
      </ul>
    </section>
  );
}

function BlockedRow({ person }: { person: BlockedUser }) {
  const t = useT(blocksCatalog);
  const unblock = useUnblock();
  const { status } = useInlineStatus();
  return (
    <ActionRow
      title={person.name}
      hint={t("area.since", { date: formatDate(person.created_at) })}
      status={status}
      action={(describedBy) => (
        <Button
          size="sm"
          aria-describedby={describedBy}
          disabled={unblock.isPending}
          onClick={() => unblock.mutate(person.id)}
        >
          {t("action.unblock")}
        </Button>
      )}
    />
  );
}
