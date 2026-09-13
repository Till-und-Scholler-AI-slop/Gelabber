// Wire shapes of `/api/channels/{id}/messages` and `/api/messages/{id}`.

export type MessageAuthor = {
  id: string;
  name: string;
  avatar_url: string | null;
};

export type Message = {
  id: string;
  channel_id: string;
  author: MessageAuthor;
  content: string;
  created_at: string;
  edited_at: string | null;
};

export type MessagePage = {
  /** Oldest → newest within the page. */
  messages: Message[];
  has_more: boolean;
  /** `{created_at}|{id}` of the oldest row when this page arrived — kept
   *  after optimistic deletes so `has_more` can still page. */
  older?: string;
};

export type ListMessagesParams = {
  before?: string;
  after?: string;
  limit?: number;
};
