// The composer's place in a DM the user blocks (#565): "Du hast {name}
// blockiert" with "Blockierung aufheben". The one blocked keeps the composer
// and is not told (#619).

import { useT } from "../i18n/catalog.ts";
import { blocks as blocksCatalog } from "../i18n/catalogs/blocks.ts";
import { BlockButton } from "./BlockButton.tsx";

export function ClosedBar({
  peerId,
  peerName,
}: {
  peerId: string;
  peerName: string;
}) {
  const t = useT(blocksCatalog);
  return (
    <div className="request-bar" role="status">
      <p>{t("composer.blocked", { name: peerName })}</p>
      <div className="request-bar__actions">
        <BlockButton userId={peerId} name={peerName} />
      </div>
    </div>
  );
}
