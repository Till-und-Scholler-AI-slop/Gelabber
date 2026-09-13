// Compact WS frames (issue #6 + #10). Short keys, no extra envelope.
// Chat events stay on `op: "e"`. Signaling is `op: "sig"` — live only.

export type EventType = "c" | "e" | "d";

export type Topic = {
  s: string;
  c?: string;
};

export type SigType = "j" | "l" | "o" | "a" | "i" | "p" | "u";
export type TrackKind = "a" | "v";

export type SigClientFrame = {
  op: "sig";
  t: SigType;
  s: string;
  c: string;
  sdp?: string;
  ice?: string;
  mid?: string;
  k?: TrackKind;
};

export type ClientFrame =
  | { op: "h" }
  | { op: "s"; s: string; c?: string; n?: number }
  | { op: "u"; s: string; c?: string }
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
  c: string;
  u: string;
  sdp?: string;
  ice?: string;
  mid?: string;
  k?: TrackKind;
};

export type ErrFrame = { op: "err"; e: string; s?: string; c?: string };

export type ServerFrame =
  | { op: "h" }
  | { op: "ok"; s: string; c?: string; n: number }
  | ChatEvent
  | SigEvent
  | { op: "gap"; s: string; c?: string }
  | ErrFrame;

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
    op === "err"
  ) {
    return value as ServerFrame;
  }
  return null;
}

/** Subscribe frame for reconnect: include last seq so the server can fill the hole. */
export function resumeFrame(topic: Topic, lastSeq: number | undefined): ClientFrame {
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
