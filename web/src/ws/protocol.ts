// Compact WS frames (issue #6 + #8 + #10). Short keys, no extra envelope.
// Chat events stay on `op: "e"`. Presence/typing are live-only (`p` / `y`).
// Signaling is `op: "sig"` — live only.

import type { Locale } from "../i18n/locale.ts";

export type EventType = "c" | "e" | "d";

export type Topic = {
  s: string;
  c?: string;
};

/** Presence plane. SDP and ICE are not gateway ops. */
export type SigType = "j" | "l" | "p" | "u" | "m" | "d" | "r" | "w";
export type TrackKind = "a" | "v" | "s" | "sa";

export type VoiceEntry = {
  u: string;
  c: string;
  m?: boolean;
  d?: boolean;
  /** Publishes a screen: Live. Not the server, unlike a frame's `s`. */
  s?: boolean;
};

export type SigClientFrame = {
  op: "sig";
  t: Exclude<SigType, "r">;
  s: string;
  c: string;
  k?: TrackKind;
  on?: boolean;
  /** `t: "w"`: the owner of the Live being watched. */
  o?: string;
};

/** Online, away, do not disturb (#590), offline. */
export type PresenceStatus = "o" | "i" | "d" | "x";

const PRESENCE_STATUSES: readonly string[] = ["o", "i", "d", "x"];

/** A status this client does not know (a newer server's) is someone who is
 * connected all the same: online. */
export function presenceStatusOf(raw: unknown): PresenceStatus {
  return typeof raw === "string" && PRESENCE_STATUSES.includes(raw)
    ? (raw as PresenceStatus)
    : "o";
}

export type ClientFrame =
  | { op: "h" }
  | { op: "s"; s: string; c?: string; n?: number; ep?: string }
  | { op: "u"; s: string; c?: string }
  | { op: "p"; st?: "o" | "i" }
  /** The page is in front (#243): no Web Push for the account meanwhile. */
  | { op: "f"; on: boolean }
  | { op: "y"; s: string; c: string; on: boolean }
  | SigClientFrame;

export type ChatEvent = {
  op: "e";
  t: EventType;
  s: string;
  c?: string;
  n: number;
  ep?: string;
  r?: number;
  i?: string;
  d?: unknown;
};

export type SigEvent = {
  op: "sig";
  t: SigType;
  s: string;
  c?: string;
  u?: string;
  k?: TrackKind;
  on?: boolean;
  m?: boolean;
  d?: boolean;
  snap?: VoiceEntry[];
  /** Existing occupant replay, not a new arrival. */
  replay?: boolean;
  /** `t: "w"`: every owner whose Live `u` watches in `c`; `[]` is none. */
  w?: string[];
};

export type ErrFrame = { op: "err"; e: string; s?: string; c?: string };

export type PresenceEntry = { u: string; st: PresenceStatus };

export type PresenceFrame = {
  op: "p";
  s: string;
  u?: string;
  st?: PresenceStatus;
  snap?: PresenceEntry[];
};

export type TypingFrame = {
  op: "y";
  s: string;
  c: string;
  u: string;
  on: boolean;
};

/** Frames for every socket of the account, whatever it subscribed to. */
export type AccountFrame =
  | {
      /** Notification settings changed on some device: fetch them again. */
      op: "ns";
    }
  | {
      /** Read positions moved on some device (#244). */
      op: "rd";
      d: ReadSync[];
    }
  | {
      /** The account's language changed on some device. */
      op: "lc";
      l: Locale;
    }
  | {
      /** The account's manual presence (#590) changed on some device or
       * ran out: `m` the mode, `u` its end (none: until changed). */
      op: "pm";
      m: string;
      u?: string;
    }
  | {
      /** A join request was accepted: the account is now a member of `s`. */
      op: "jn";
      s: string;
    }
  | {
      /** The pending join requests of server `s` changed; `n` are open. */
      op: "jq";
      s: string;
      n: number;
    }
  | {
      /** Friends or friend requests changed (#563): fetch them again. */
      op: "fr";
    }
  | {
      /** Incoming message requests changed (#563): fetch them again. */
      op: "mr";
    }
  | {
      /** The signed-in user blocked or unblocked someone (#565). */
      op: "bl";
    };

/** A conversation's new read position, as `PUT …/read` answers it. */
export type ReadSync = {
  channel_id: string;
  server_id: string | null;
  read_message_id: string | null;
  read_at: string | null;
  read_order?: number | null;
  membership_at?: string;
  latest_order?: number | null;
  unread_count: number;
  mention_count: number;
};

export type ServerFrame =
  | { op: "h" }
  | { op: "ok"; s: string; c?: string; n: number; ep?: string }
  | ChatEvent
  | SigEvent
  | { op: "gap"; s: string; c?: string; ep?: string }
  | { op: "resync" }
  | { op: "dm"; c: string }
  | ErrFrame
  | PresenceFrame
  | TypingFrame
  | AccountFrame;

export function topicKey(topic: Topic): string {
  return topic.c ? `c:${topic.c}` : `s:${topic.s}`;
}

export function encode(frame: ClientFrame): string {
  return JSON.stringify(frame);
}

export function decode(raw: string): ServerFrame | null {
  let value: unknown;
  try {
    value = JSON.parse(raw) as unknown;
  } catch {
    return null;
  }
  if (value === null || typeof value !== "object" || !("op" in value)) {
    return null;
  }
  const op = (value as { op: unknown }).op;
  if (
    op === "h" ||
    op === "ok" ||
    op === "e" ||
    op === "sig" ||
    op === "gap" ||
    op === "err" ||
    op === "p" ||
    op === "y" ||
    op === "resync" ||
    op === "dm" ||
    op === "ns" ||
    op === "rd" ||
    op === "lc" ||
    op === "pm" ||
    op === "jn" ||
    op === "jq" ||
    op === "fr" ||
    op === "mr" ||
    op === "bl"
  ) {
    if (op === "p") return presenceFrame(value as PresenceFrame);
    return value as ServerFrame;
  }
  return null;
}

/** Presence with every status one this client knows. */
function presenceFrame(frame: PresenceFrame): PresenceFrame {
  const known = { ...frame };
  if (frame.st !== undefined) known.st = presenceStatusOf(frame.st);
  if (Array.isArray(frame.snap))
    known.snap = frame.snap.map((entry) => ({
      ...entry,
      st: presenceStatusOf(entry.st),
    }));
  return known;
}

/** Subscribe frame for reconnect: include last seq so the server can fill the hole. */
export function resumeFrame(
  topic: Topic,
  lastSeq: number | undefined,
  epoch?: string,
): ClientFrame {
  if (lastSeq === undefined) {
    return { op: "s", s: topic.s, ...(topic.c ? { c: topic.c } : {}) };
  }
  return {
    op: "s",
    s: topic.s,
    ...(topic.c ? { c: topic.c } : {}),
    n: lastSeq,
    ...(epoch ? { ep: epoch } : {}),
  };
}

/** Keep the highest seq per topic; drop a frame the client already applied. */
export function nextCursor(
  current: number | undefined,
  incoming: number,
): { accept: boolean; cursor: number } {
  if (current !== undefined && incoming <= current) {
    return { accept: false, cursor: current };
  }
  return { accept: true, cursor: incoming };
}
