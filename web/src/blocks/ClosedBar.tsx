// The composer's place in a DM nobody can write in (#565): "Du hast {name}
// blockiert" with "Blockierung aufheben" for the one who blocks; for the
// other a neutral line that does not say why.

import { useT } from "../i18n/catalog.ts";
import { blocks as blocksCatalog } from "../i18n/catalogs/blocks.ts";
import { BlockButton } from "./BlockButton.tsx";

export function ClosedBar({
  peerId,
  peerName,
  blocked,
}: {
  peerId: string;
  peerName: string;
  blocked: boolean;
}) {
  const t = useT(blocksCatalog);
  if (!blocked)
    return (
      <p className="request-bar request-bar--note" role="status">
        {t("composer.closed")}
      </p>
    );
  return (
    <div className="request-bar" role="status">
      <p>{t("composer.blocked", { name: peerName })}</p>
      <div className="request-bar__actions">
        <BlockButton userId={peerId} name={peerName} />
      </div>
    </div>
  );
}
