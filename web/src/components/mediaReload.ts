// Recovers an inline player whose signed storage link expired.

/** At most one fresh link per minute for a player that already played. */
const MEDIA_RELOAD_GAP_MS = 60_000;

type ReloadableMedia = Pick<
  HTMLMediaElement,
  "currentTime" | "paused" | "readyState" | "load" | "play"
> & {
  addEventListener(
    type: "loadedmetadata",
    listener: () => void,
    options: { once: true },
  ): void;
};

/**
 * A player that already had the file loses it when its signed storage link
 * expires during a long pause and it seeks or resumes. Asking the API again
 * (it redirects to a fresh link) and returning to the same spot keeps it a
 * player. Returns false when the file is unplayable after all.
 */
export function reloadMedia(
  el: ReloadableMedia,
  lastReload: number,
  now: number,
): boolean {
  if (el.readyState < 1 || now - lastReload < MEDIA_RELOAD_GAP_MS) return false;
  const at = el.currentTime;
  const resume = !el.paused;
  el.addEventListener(
    "loadedmetadata",
    () => {
      el.currentTime = at;
      if (resume) void el.play().catch(() => undefined);
    },
    { once: true },
  );
  el.load();
  return true;
}
