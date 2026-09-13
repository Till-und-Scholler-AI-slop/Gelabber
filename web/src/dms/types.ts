// Wire shapes of `/api/dms` (see `api/src/dms`).

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
};
