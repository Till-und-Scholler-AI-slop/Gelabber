// Links that save a chat file. The page never learns when the download is
// done or where the file lands, only that the click started it, so a link
// says that for a moment. Clicks meanwhile start no further copies: without
// a sign, people clicked again and found the file ten times (#391).

import { useEffect, useState, type MouseEvent } from "react";

/** How long a link says that its download started; as long as an info
 *  toast stays. */
export const DOWNLOAD_STARTED_MS = 4000;

export const DOWNLOAD_STARTED = "Download gestartet";

/** Whether this link's download just started, and the click that starts
 *  it. Spread `link` onto the `<a download>`. */
export function useDownload(): {
  started: boolean;
  link: {
    onClick: (event: MouseEvent<HTMLAnchorElement>) => void;
    "aria-disabled": true | undefined;
  };
} {
  const [started, setStarted] = useState(false);
  useEffect(() => {
    if (!started) return;
    const timer = setTimeout(() => setStarted(false), DOWNLOAD_STARTED_MS);
    return () => clearTimeout(timer);
  }, [started]);
  return {
    started,
    link: {
      onClick: (event) => {
        if (started) event.preventDefault();
        else setStarted(true);
      },
      "aria-disabled": started || undefined,
    },
  };
}
