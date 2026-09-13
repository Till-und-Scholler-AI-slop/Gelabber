// Wire shapes of `/api/channels/{id}/messages` and `/api/messages/{id}`.

export type MessageAuthor = {
  id: string;
  name: string;
  avatar_url: string | null;
};

export type Attachment = {
  id: string;
  filename: string;
  content_type: string;
  size: number;
  /** Client-only: local object URL while the PUT is still in flight. */
  preview_url?: string;
};

export type Message = {
  id: string;
  channel_id: string;
  author: MessageAuthor;
  content: string;
  created_at: string;
  edited_at: string | null;
  attachments: Attachment[];
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

export type PresignRequest = {
  filename: string;
  content_type: string;
  size: number;
};

export type PresignResponse = {
  id: string;
  upload_url: string;
  headers: Record<string, string>;
  expires_in: number;
  attachment: Attachment;
};
