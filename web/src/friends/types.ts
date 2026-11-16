// Wire shapes of `/api/friends` and `/api/me/friend-code` (see
// `backend/api/src/friends`).

export type FriendPerson = {
  id: string;
  name: string;
  avatar_url: string | null;
};

export type Friend = {
  user: FriendPerson;
  since: string;
  /** `o` online, `i` idle; absent when offline or not known. */
  status?: "o" | "i";
};

export type FriendRequest = {
  user: FriendPerson;
  created_at: string;
};

export type FriendList = {
  friends: Friend[];
  incoming: FriendRequest[];
  outgoing: FriendRequest[];
};

/** `pending`: the request waits; `friends`: they had asked already. */
export type SendAnswer = {
  status: "pending" | "friends";
  peer: FriendPerson;
};

/** How the signed-in user stands to someone, for a profile card's button. */
export type Relation = "self" | "friend" | "outgoing" | "incoming" | "none";
