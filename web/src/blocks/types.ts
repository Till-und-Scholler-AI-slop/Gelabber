// Wire shapes of `/api/users/me/blocks` (#565, `backend/api/src/blocks`).

/** Someone the signed-in user blocks. */
export type BlockedUser = {
  id: string;
  name: string;
  avatar_url: string | null;
  created_at: string;
};
