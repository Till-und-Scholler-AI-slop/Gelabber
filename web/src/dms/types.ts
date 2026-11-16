// Wire shapes of `/api/dms` (see `backend/api/src/dms`).

export type DmPeer = {
  id: string;
  name: string;
  avatar_url: string | null;
};

/** A 1:1 DM as the signed-in user sees it. `peer` is the other person. */
export type DirectMessage = {
  id: string;
  kind: "dm";
  created_at: string;
  peer: DmPeer;
  /** Set while the DM is a message request (#563); absent once accepted. */
  request?: DmRequest;
};

/**
 * A pending message request. `incoming`: the peer started it and waits for
 * an answer. Otherwise the signed-in user sent it and may write `remaining`
 * more text-only messages until it is accepted.
 */
export type DmRequest = {
  incoming: boolean;
  remaining: number;
};
