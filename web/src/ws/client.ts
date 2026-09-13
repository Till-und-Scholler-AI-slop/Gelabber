// Browser stand-in for the native gateway: one socket per tab, session
// cookie on the handshake, heartbeat, and reconnect with last-seq resume
// so a blip is invisible and never doubles a message.

import {
  type ChatEvent,
  type ClientFrame,
  type ErrFrame,
  type PresenceFrame,
  type ServerFrame,
  type SigEvent,
  type Topic,
  type TypingFrame,
  decode,
  encode,
  nextCursor,
  resumeFrame,
  topicKey,
} from "./protocol.ts";

export type GatewayStatus = "idle" | "connecting" | "open" | "reconnecting";

export type SocketLike = {
  send(data: string): void;
  close(): void;
  addEventListener(
    type: "open" | "message" | "close" | "error",
    handler: (event: { data?: string }) => void,
  ): void;
  removeEventListener(
    type: "open" | "message" | "close" | "error",
    handler: (event: { data?: string }) => void,
  ): void;
};

export type GatewayOptions = {
  url?: string;
  open?: (url: string) => SocketLike;
  heartbeatMs?: number;
  deadMs?: number;
  retryMs?: number;
  maxRetryMs?: number;
  now?: () => number;
  onStatus?: (status: GatewayStatus) => void;
};

const DEFAULT_HEARTBEAT_MS = 15_000;
const DEFAULT_DEAD_MS = 30_000;
const DEFAULT_RETRY_MS = 200;
const DEFAULT_MAX_RETRY_MS = 2_000;

export function gatewayUrl(): string {
  if (typeof location === "undefined") {
    return "/ws";
  }
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/ws`;
}

export type GapNotice = Topic;

export class Gateway {
  private readonly opts: Required<
    Pick<
      GatewayOptions,
      "heartbeatMs" | "deadMs" | "retryMs" | "maxRetryMs" | "now"
    >
  > &
    GatewayOptions;
  private socket: SocketLike | null = null;
  private desired = new Map<string, Topic>();
  private cursors = new Map<string, number>();
  private retryAttempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private watchdogTimer: ReturnType<typeof setInterval> | null = null;
  private lastServerAt = 0;
  private stopped = true;
  private eventListeners = new Set<(event: ChatEvent) => void>();
  private sigListeners = new Set<(event: SigEvent) => void>();
  private errListeners = new Set<(err: ErrFrame) => void>();
  private readyListeners = new Set<() => void>();
  private gapListeners = new Set<(gap: GapNotice) => void>();
  private presenceListeners = new Set<(frame: PresenceFrame) => void>();
  private typingListeners = new Set<(frame: TypingFrame) => void>();

  constructor(opts: GatewayOptions = {}) {
    this.opts = {
      heartbeatMs: opts.heartbeatMs ?? DEFAULT_HEARTBEAT_MS,
      deadMs: opts.deadMs ?? DEFAULT_DEAD_MS,
      retryMs: opts.retryMs ?? DEFAULT_RETRY_MS,
      maxRetryMs: opts.maxRetryMs ?? DEFAULT_MAX_RETRY_MS,
      now: opts.now ?? Date.now,
      ...opts,
    };
  }

  get cursorsSnapshot(): ReadonlyMap<string, number> {
    return this.cursors;
  }

  onEvent(listener: (event: ChatEvent) => void): () => void {
    this.eventListeners.add(listener);
    return () => {
      this.eventListeners.delete(listener);
    };
  }

  onGap(listener: (gap: GapNotice) => void): () => void {
    this.gapListeners.add(listener);
    return () => {
      this.gapListeners.delete(listener);
    };
  }

  onSig(listener: (event: SigEvent) => void): () => void {
    this.sigListeners.add(listener);
    return () => {
      this.sigListeners.delete(listener);
    };
  }

  onErr(listener: (err: ErrFrame) => void): () => void {
    this.errListeners.add(listener);
    return () => {
      this.errListeners.delete(listener);
    };
  }

  onReady(listener: () => void): () => void {
    this.readyListeners.add(listener);
    return () => {
      this.readyListeners.delete(listener);
    };
  }

  onPresence(listener: (frame: PresenceFrame) => void): () => void {
    this.presenceListeners.add(listener);
    return () => {
      this.presenceListeners.delete(listener);
    };
  }

  onTyping(listener: (frame: TypingFrame) => void): () => void {
    this.typingListeners.add(listener);
    return () => {
      this.typingListeners.delete(listener);
    };
  }

  /** Send a frame on the live socket (signaling, subscribe, presence). */
  send(frame: ClientFrame): void {
    try {
      this.socket?.send(encode(frame));
    } catch {
      // Closing / already gone — reconnect will resend.
    }
  }

  sendPresence(st?: "o" | "i"): void {
    this.send(st ? { op: "p", st } : { op: "p" });
  }

  sendTyping(serverId: string, channelId: string, on: boolean): void {
    this.send({ op: "y", s: serverId, c: channelId, on });
  }

  start(): void {
    if (!this.stopped) {
      return;
    }
    this.stopped = false;
    this.open();
  }

  stop(): void {
    this.stopped = true;
    this.clearTimers();
    this.socket?.close();
    this.socket = null;
    this.opts.onStatus?.("idle");
  }

  /** Replace the subscribe set. Diffs against the last set and the live socket. */
  setTopics(topics: Topic[]): void {
    const next = new Map<string, Topic>();
    for (const topic of topics) {
      next.set(topicKey(topic), topic);
    }
    if (this.socket) {
      for (const [key, topic] of this.desired) {
        if (!next.has(key)) {
          this.send({ op: "u", s: topic.s, ...(topic.c ? { c: topic.c } : {}) });
        }
      }
      for (const [key, topic] of next) {
        if (!this.desired.has(key)) {
          this.send(resumeFrame(topic, this.cursors.get(key)));
        }
      }
    }
    this.desired = next;
  }

  private open(): void {
    this.clearTimers();
    this.opts.onStatus?.(this.retryAttempt === 0 ? "connecting" : "reconnecting");
    const url = this.opts.url ?? gatewayUrl();
    const socket = this.opts.open
      ? this.opts.open(url)
      : new WebSocket(url);
    this.socket = socket;

    const onOpen = () => {
      this.retryAttempt = 0;
      this.lastServerAt = this.opts.now();
      this.opts.onStatus?.("open");
      this.startHeartbeat();
      for (const [key, topic] of this.desired) {
        this.send(resumeFrame(topic, this.cursors.get(key)));
      }
      for (const listener of this.readyListeners) {
        listener();
      }
    };
    const onMessage = (event: { data?: string }) => {
      if (typeof event.data !== "string") {
        return;
      }
      this.lastServerAt = this.opts.now();
      const frame = decode(event.data);
      if (frame) {
        this.onFrame(frame);
      }
    };
    const onClose = () => {
      this.socket = null;
      this.clearHeartbeat();
      if (!this.stopped) {
        this.scheduleReconnect();
      }
    };

    socket.addEventListener("open", onOpen);
    socket.addEventListener("message", onMessage);
    socket.addEventListener("close", onClose);
  }

  private onFrame(frame: ServerFrame): void {
    switch (frame.op) {
      case "h":
        this.send({ op: "h" });
        return;
      case "ok": {
        const key = topicKey(frame);
        const current = this.cursors.get(key);
        if (current === undefined || frame.n > current) {
          this.cursors.set(key, frame.n);
        }
        return;
      }
      case "e": {
        const key = topicKey(frame);
        const { accept, cursor } = nextCursor(this.cursors.get(key), frame.n);
        this.cursors.set(key, cursor);
        if (accept) {
          for (const listener of this.eventListeners) {
            listener(frame);
          }
        }
        return;
      }
      case "sig":
        for (const listener of this.sigListeners) {
          listener(frame);
        }
        return;
      case "gap":
        for (const listener of this.gapListeners) {
          listener({ s: frame.s, c: frame.c });
        }
        return;
      case "p":
        for (const listener of this.presenceListeners) {
          listener(frame);
        }
        return;
      case "y":
        for (const listener of this.typingListeners) {
          listener(frame);
        }
        return;
      case "err":
        for (const listener of this.errListeners) {
          listener(frame);
        }
        return;
    }
  }

  private startHeartbeat(): void {
    this.clearHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      this.send({ op: "h" });
    }, this.opts.heartbeatMs);
    this.watchdogTimer = setInterval(() => {
      if (this.opts.now() - this.lastServerAt >= this.opts.deadMs) {
        this.socket?.close();
      }
    }, Math.min(1_000, this.opts.heartbeatMs));
  }

  private scheduleReconnect(): void {
    this.opts.onStatus?.("reconnecting");
    // First drop: reconnect on the next turn so the close handler can
    // finish. After that, exponential backoff (capped) — still short.
    const delay =
      this.retryAttempt === 0
        ? 0
        : Math.min(
            this.opts.maxRetryMs,
            this.opts.retryMs * 2 ** (this.retryAttempt - 1),
          );
    this.retryAttempt += 1;
    this.reconnectTimer = setTimeout(() => {
      if (!this.stopped) {
        this.open();
      }
    }, delay);
  }

  private clearHeartbeat(): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
    if (this.watchdogTimer) {
      clearInterval(this.watchdogTimer);
      this.watchdogTimer = null;
    }
  }

  private clearTimers(): void {
    this.clearHeartbeat();
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }
}

let singleton: Gateway | null = null;

export function getGateway(): Gateway {
  singleton ??= new Gateway();
  return singleton;
}

/** Tests / logout: drop the process-wide socket. */
export function resetGatewayForTests(): void {
  singleton?.stop();
  singleton = null;
}
