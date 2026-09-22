// Compact WS frames (issue #6 + #8 + #10). Short keys, no extra envelope.
// Chat events stay on `op: "e"`. Presence/typing are live-only (`p` / `y`).
// Signaling is `op: "sig"` — live only.

export type EventType = "c" | "e" | "d";

export type Topic = {
  s: string;
  c?: string;
};

/** Presence plane. SDP and ICE are not gateway ops. */
export type SigType = "j" | "l" | "p" | "u" | "m" | "d" | "r";
export type TrackKind = "a" | "v" | "s" | "l";

export type VoiceEntry = {
  u: string;
  c: string;
  m?: boolean;
  d?: boolean;
  l?: boolean;
};

export type SigClientFrame = {
  op: "sig";
  t: Exclude<SigType, "r">;
  s: string;
  c: string;
  k?: TrackKind;
  on?: boolean;
};

export type PresenceStatus = "o" | "i" | "x";

export type ClientFrame =
  | { op: "h" }
  | { op: "s"; s: string; c?: string; n?: number }
  | { op: "u"; s: string; c?: string }
  | { op: "p"; st?: "o" | "i" }
  | { op: "y"; s: string; c: string; on: boolean }
  | SigClientFrame;

export type ChatEvent = {
  op: "e";
  t: EventType;
  s: string;
  c?: string;
  n: number;
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

export type ServerFrame =
  | { op: "h" }
  | { op: "ok"; s: string; c?: string; n: number }
  | ChatEvent
  | SigEvent
  | { op: "gap"; s: string; c?: string }
  | ErrFrame
  | PresenceFrame
  | TypingFrame;

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
    op === "y"
  ) {
    return value as ServerFrame;
  }
  return null;
}

/** Subscribe frame for reconnect: include last seq so the server can fill the hole. */
export function resumeFrame(
  topic: Topic,
  lastSeq: number | undefined,
): ClientFrame {
  if (lastSeq === undefined) {
    return { op: "s", s: topic.s, ...(topic.c ? { c: topic.c } : {}) };
  }
  return {
    op: "s",
    s: topic.s,
    ...(topic.c ? { c: topic.c } : {}),
    n: lastSeq,
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
