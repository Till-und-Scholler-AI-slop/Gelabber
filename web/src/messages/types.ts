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
  /** Durable database revision; optional only for the older API. */
  revision?: number;
  /** Immutable server INSERT order; optimistic/legacy messages have none. */
  created_order?: number;
  channel_id: string;
  author: MessageAuthor;
  content: string;
  created_at: string;
  edited_at: string | null;
  attachments: Attachment[];
  /** Optional for older API snapshots; absent means no reactions. */
  reactions?: Reaction[];
  /** A GIF message (empty content, no attachments). */
  gif?: Gif | null;
  /** Link previews (#254), added by an edit event after sending. */
  previews?: LinkPreview[];
};

/** A link preview as the API stores it. Text is the page's plain text;
 *  the image loads from the API (`/api/previews/…`), never from the site. */
export type LinkPreview = {
  url: string;
  title?: string;
  description?: string;
  site_name?: string;
  image?: { url: string; width: number; height: number };
  youtube_id?: string;
};

const YOUTUBE_ID = /^[A-Za-z0-9_-]{11}$/;

/** Previews from the wire, whole or dropped: cjson turns `[]` into `{}`, and
 *  only http(s) pages, API images and YouTube ids are kept. */
export function asPreviewList(value: unknown): LinkPreview[] {
  if (!Array.isArray(value)) return [];
  const text = (v: unknown) => (typeof v === "string" && v ? v : undefined);
  const out: LinkPreview[] = [];
  for (const raw of value as unknown[]) {
    if (raw === null || typeof raw !== "object") continue;
    const p = raw as Record<string, unknown>;
    if (typeof p.url !== "string" || !/^https?:\/\//i.test(p.url)) continue;
    const image = p.image as Record<string, unknown> | null | undefined;
    const youtube = text(p.youtube_id);
    out.push({
      url: p.url,
      title: text(p.title),
      description: text(p.description),
      site_name: text(p.site_name),
      image:
        image &&
        typeof image.url === "string" &&
        image.url.startsWith("/api/previews/") &&
        typeof image.width === "number" &&
        typeof image.height === "number" &&
        image.width > 0 &&
        image.height > 0
          ? { url: image.url, width: image.width, height: image.height }
          : undefined,
      youtube_id: youtube && YOUTUBE_ID.test(youtube) ? youtube : undefined,
    });
  }
  return out;
}

/** One GIF from the server's Klipy relay. URLs are Klipy's; load them
 *  only through `gifMediaUrl`. */
export type Gif = {
  provider: string;
  slug: string;
  title: string;
  width: number;
  height: number;
  /** Animated WebP (or GIF). */
  url: string;
  /** First frame, shown instead of the animation for reduced motion. */
  still: string | null;
};

/** Redis cjson re-encoding may hand back anything; a GIF must be whole. */
export function asGif(value: unknown): Gif | null {
  if (value === null || typeof value !== "object") return null;
  const gif = value as Partial<Gif>;
  if (
    typeof gif.slug !== "string" ||
    typeof gif.url !== "string" ||
    typeof gif.width !== "number" ||
    typeof gif.height !== "number" ||
    gif.width <= 0 ||
    gif.height <= 0
  )
    return null;
  return {
    provider: typeof gif.provider === "string" ? gif.provider : "klipy",
    slug: gif.slug,
    title: typeof gif.title === "string" ? gif.title : "",
    width: gif.width,
    height: gif.height,
    url: gif.url,
    still: typeof gif.still === "string" ? gif.still : null,
  };
}

export type Reaction = { emoji: string; user_ids: string[] };

export type MessagePage = {
  /** Oldest → newest within the page. */
  messages: Message[];
  has_more: boolean;
  /** `{created_at}|{id}` of the oldest row when this page arrived — kept
   *  after optimistic deletes so `has_more` can still page. */
  older?: string;
};

/** Redis Lua `cjson.encode` turns `[]` into `{}`. */
export function asAttachmentList(value: unknown): Attachment[] {
  return Array.isArray(value) ? value : [];
}

/** Redis cjson re-encoding represents empty arrays as {}. */
export function asReactionList(value: unknown): Reaction[] {
  if (!Array.isArray(value)) return [];
  return value.filter(
    (reaction): reaction is Reaction =>
      reaction !== null &&
      typeof reaction === "object" &&
      typeof reaction.emoji === "string" &&
      Array.isArray(reaction.user_ids) &&
      reaction.user_ids.every((id: unknown) => typeof id === "string"),
  );
}

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
