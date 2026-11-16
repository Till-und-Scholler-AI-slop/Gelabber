// "Blockieren" on a profile card and in a conversation (#565): asks first;
// for someone already blocked it unblocks without asking.

import { Button } from "../components/ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { blocks as blocksCatalog } from "../i18n/catalogs/blocks.ts";
import {
  confirmBlock,
  useBlock,
  useBlockedIds,
  useUnblock,
} from "./queries.ts";

export function BlockButton({
  userId,
  name,
  onBlocked,
}: {
  userId: string;
  name: string;
  /** After a block went through (a dialog closes, a page moves on). */
  onBlocked?: () => void;
}) {
  const t = useT(blocksCatalog);
  const blocked = useBlockedIds().has(userId);
  const block = useBlock();
  const unblock = useUnblock();
  const busy = block.isPending || unblock.isPending;
  if (blocked)
    return (
      <Button
        size="sm"
        variant="ghost"
        disabled={busy}
        onClick={() => unblock.mutate(userId)}
      >
        {t("action.unblock")}
      </Button>
    );
  return (
    <Button
      size="sm"
      variant="ghost"
      disabled={busy}
      onClick={() => {
        void confirmBlock(name).then((ok) => {
          if (ok) block.mutate(userId, { onSuccess: () => onBlocked?.() });
        });
      }}
    >
      {t("action.block")}
    </Button>
  );
}
