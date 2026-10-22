// Whether an unread badge just appeared, so it may scale in once. Only a
// badge that shows up on a list already on screen with the read state
// loaded is news; one that is there when the list renders, or comes with
// the first snapshot, is not.

export type BadgeSeen = {
  /** The read state was loaded. */
  loaded: boolean;
  /** A count or a dot is shown. */
  shown: boolean;
  /** It appeared while looked at, and is still there. */
  fresh: boolean;
};

export function badgeSeen(
  previous: BadgeSeen | null,
  loaded: boolean,
  shown: boolean,
): BadgeSeen {
  if (!previous) return { loaded, shown, fresh: false };
  const appeared = previous.loaded && !previous.shown && shown;
  return { loaded, shown, fresh: shown && (appeared || previous.fresh) };
}
