import { useEffect, useState } from "react";

import { getGateway } from "../ws/client.ts";
import {
  connectionNotices,
  type ConnectionNotice as Notice,
} from "../ws/connectionNotice.ts";
import { CheckIcon } from "./Icons.tsx";
import "./connection.css";

const TEXT: Record<Exclude<Notice, null>, string> = {
  offline: "Keine Verbindung – dein Gerät ist offline",
  reconnecting: "Keine Verbindung – wird wiederhergestellt…",
  restored: "Verbindung wiederhergestellt",
};

function useConnectionNotice(): Notice {
  const [notice, setNotice] = useState<Notice>(null);
  useEffect(() => {
    const gateway = getGateway();
    const notices = connectionNotices(setNotice);
    const update = () =>
      notices.update({ status: gateway.status, online: navigator.onLine });
    update();
    const off = gateway.onStatus(update);
    window.addEventListener("online", update);
    window.addEventListener("offline", update);
    return () => {
      off();
      window.removeEventListener("online", update);
      window.removeEventListener("offline", update);
      notices.stop();
    };
  }, []);
  return notice;
}

/**
 * A small pill under the top bar while the live connection is gone, and a
 * moment after it is back. The region stays in the page, so a screen
 * reader hears each change.
 */
export function ConnectionNotice() {
  const notice = useConnectionNotice();
  return (
    <div role="status" className="gel-connection">
      {notice ? (
        <p
          key={notice === "restored" ? "restored" : "lost"}
          className={`gel-connection__pill gel-connection__pill--${notice}`}
        >
          {notice === "restored" ? (
            <CheckIcon size={14} className="gel-connection__icon" />
          ) : (
            <span className="gel-connection__dot" aria-hidden="true" />
          )}
          {TEXT[notice]}
        </p>
      ) : null}
    </div>
  );
}
