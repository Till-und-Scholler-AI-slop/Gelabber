// The links an invite dialog makes. One is made as it opens; changing the
// terms makes a new one. A link nobody copied or shared is revoked once it is
// replaced or the dialog closes, so no unseen working invite is left behind.

import type { InviteTerms } from "../servers/inviteTerms.ts";
import type { Invite } from "../servers/types.ts";

/** How long the terms rest before a link is made for them: arrowing through
 * the chips selects each one on the way. */
export const INVITE_TERMS_DELAY_MS = 400;

export type InviteSession = {
  /** A link with these terms: now, or once the terms rested (`wait`). */
  request: (terms: InviteTerms, wait: boolean) => void;
  /** A copy or share of the link on screen started: someone may hold it. */
  handOut: () => void;
  /** The dialog closed: an untaken link is revoked, one on its way too. */
  close: () => void;
  /** Mounted again (React StrictMode's trial unmount). */
  open: () => void;
};

export function inviteSession({
  create,
  revoke,
  shown,
  failed,
}: {
  create: (terms: InviteTerms) => Promise<Invite>;
  revoke: (code: string) => void;
  /** A new link is on screen; `replaced` when an earlier one was. */
  shown: (invite: Invite, terms: InviteTerms, replaced: boolean) => void;
  /** The newest request failed; the terms of the link still shown. */
  failed: (shownTerms: InviteTerms | null) => void;
}): InviteSession {
  let current: { invite: Invite; terms: InviteTerms } | null = null;
  let handedOut = false;
  let closed = false;
  let latest = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const make = (terms: InviteTerms) => {
    const request = ++latest;
    create(terms).then(
      (created) => {
        // Closed, or the terms changed again meanwhile: nobody saw it.
        if (closed || request !== latest) {
          revoke(created.code);
          return;
        }
        const previous = current;
        current = { invite: created, terms };
        if (previous && !handedOut) revoke(previous.invite.code);
        handedOut = false;
        shown(created, terms, previous !== null);
      },
      () => {
        if (!closed && request === latest) failed(current?.terms ?? null);
      },
    );
  };

  return {
    request(terms, wait) {
      clearTimeout(timer);
      // Back to the link on screen: it serves, and one still on its way for
      // other terms is revoked when it arrives.
      if (
        current &&
        current.terms.expiry === terms.expiry &&
        current.terms.uses === terms.uses
      ) {
        latest++;
        return;
      }
      if (wait) timer = setTimeout(() => make(terms), INVITE_TERMS_DELAY_MS);
      else make(terms);
    },
    handOut() {
      handedOut = true;
    },
    close() {
      closed = true;
      clearTimeout(timer);
      if (current && !handedOut) revoke(current.invite.code);
      current = null;
    },
    open() {
      closed = false;
    },
  };
}
