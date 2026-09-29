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
export type GapRecovery = { topic: Topic; version: number };
export type TopicCursor = { ep?: string; n: number };

export class Gateway {
  private readonly opts: Required<
    Pick<
      GatewayOptions,
      "heartbeatMs" | "deadMs" | "retryMs" | "maxRetryMs" | "now"
    >
  > &
    GatewayOptions;
  private socket: SocketLike | null = null;
  /** Drops listeners for the socket `open` most recently bound. */
  private detachLiveSocket: (() => void) | null = null;
  private desired = new Map<string, Topic>();
  private cursors = new Map<string, TopicCursor>();
  private gaps = new Map<
    string,
    GapRecovery & {
      ep?: string;
      cursor?: number;
      headKnown: boolean;
      reconciled: boolean;
    }
  >();
  private gapVersion = 0;
  private discoveredDms = new Set<string>();
  private resyncVersion = 0;
  private completedResync = 0;
  private retryAttempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private watchdogTimer: ReturnType<typeof setInterval> | null = null;
  private lastServerAt = 0;
  private stopped = true;
  /** Bumped by `resetSession` so a frame already in hand cannot fan out. */
  private epoch = 0;
  private eventListeners = new Set<(event: ChatEvent) => void>();
  private sigListeners = new Set<(event: SigEvent) => void>();
  private errListeners = new Set<(err: ErrFrame) => void>();
  private readyListeners = new Set<() => void>();
  private gapListeners = new Set<(gap: GapNotice) => void>();
  private presenceListeners = new Set<(frame: PresenceFrame) => void>();
  private resyncListeners = new Set<() => void>();
  private dmListeners = new Set<(channelId: string) => void>();
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
    return new Map([...this.cursors].map(([key, cursor]) => [key, cursor.n]));
  }

  get topicCursorsSnapshot(): ReadonlyMap<string, TopicCursor> {
    return new Map(
      [...this.cursors].map(([key, cursor]) => [key, { ...cursor }]),
    );
  }

  /** Gap heads become resume cursors only once REST recovery has succeeded. */
  gapRecoveries(): GapRecovery[] {
    return [...this.gaps.values()]
      .filter((gap) => !gap.reconciled)
      .map(({ topic, version }) => ({
        topic,
        version,
      }));
  }

  completeGap(recovery: GapRecovery): void {
    const key = topicKey(recovery.topic);
    const pending = this.gaps.get(key);
    if (!pending || pending.version !== recovery.version) return;
    pending.reconciled = true;
    this.finishGap(key);
  }

  private finishGap(key: string): void {
    const pending = this.gaps.get(key);
    if (
      !pending?.reconciled ||
      !pending.headKnown ||
      pending.cursor === undefined
    )
      return;
    this.cursors.set(key, { ep: pending.ep, n: pending.cursor });
    this.gaps.delete(key);
  }

  private markGap(topic: Topic, ep?: string, headKnown = false): void {
    const key = topicKey(topic);
    const previous = this.gaps.get(key);
    const current = this.cursors.get(key);
    const oldEpoch = previous?.ep ?? current?.ep;
    this.gaps.set(key, {
      topic: { s: topic.s, ...(topic.c ? { c: topic.c } : {}) },
      version: ++this.gapVersion,
      ep: ep ?? oldEpoch,
      cursor:
        ep && ep !== oldEpoch ? undefined : (previous?.cursor ?? current?.n),
      headKnown,
      reconciled: false,
    });
    const generation = this.epoch;
    for (const listener of this.gapListeners) {
      if (generation !== this.epoch) return;
      listener(topic);
    }
  }

  forgetTopic(key: string): void {
    this.setTopics(
      [...this.desired].filter(([id]) => id !== key).map(([, topic]) => topic),
    );
    this.cursors.delete(key);
    this.gaps.delete(key);
  }

  /** Private discovery is verified by REST before joining the topic union. */
  addTopic(topic: Topic): void {
    this.setTopics([...this.desired.values(), topic]);
  }

  dmDiscoveries(): string[] {
    return [...this.discoveredDms];
  }
  completeDm(channelId: string): void {
    this.discoveredDms.delete(channelId);
  }
  get pendingResync(): number | undefined {
    return this.resyncVersion !== this.completedResync
      ? this.resyncVersion
      : undefined;
  }
  completeResync(version: number | undefined): void {
    if (version === this.resyncVersion) this.completedResync = version;
  }

  onResync(listener: () => void): () => void {
    this.resyncListeners.add(listener);
    return () => {
      this.resyncListeners.delete(listener);
    };
  }

  onDm(listener: (channelId: string) => void): () => void {
    this.dmListeners.add(listener);
    return () => {
      this.dmListeners.delete(listener);
    };
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
    const socket = this.socket;
    this.unbindSocket();
    this.socket = null;
    socket?.close();
    this.opts.onStatus?.("idle");
  }

  /**
   * Logout / account switch: unsubscribe, forget resume cursors, and close
   * the socket. The next `start` is a new session with no prior topics.
   */
  resetSession(): void {
    this.epoch += 1;
    this.desired.clear();
    this.cursors.clear();
    this.gaps.clear();
    this.discoveredDms.clear();
    this.completedResync = this.resyncVersion;
    this.stop();
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
          this.send({
            op: "u",
            s: topic.s,
            ...(topic.c ? { c: topic.c } : {}),
          });
        }
      }
      for (const [key, topic] of next) {
        if (!this.desired.has(key)) {
          this.send(
            resumeFrame(
              topic,
              this.cursors.get(key)?.n,
              this.cursors.get(key)?.ep,
            ),
          );
        }
      }
    }
    this.desired = next;
  }

  private open(): void {
    this.clearTimers();
    this.unbindSocket();
    this.opts.onStatus?.(
      this.retryAttempt === 0 ? "connecting" : "reconnecting",
    );
    const url = this.opts.url ?? gatewayUrl();
    const socket = this.opts.open ? this.opts.open(url) : new WebSocket(url);
    this.socket = socket;
    // Captured here, not in `onFrame`: a close/open/message from the socket
    // we just replaced must not clear the new one or answer on it.
    const epoch = this.epoch;
    const live = () => this.socket === socket && this.epoch === epoch;

    const onOpen = () => {
      if (!live()) {
        detach();
        return;
      }
      this.retryAttempt = 0;
      this.lastServerAt = this.opts.now();
      this.opts.onStatus?.("open");
      this.startHeartbeat();
      for (const [key, topic] of this.desired) {
        this.send(
          resumeFrame(
            topic,
            this.cursors.get(key)?.n,
            this.cursors.get(key)?.ep,
          ),
        );
      }
      for (const listener of this.readyListeners) {
        listener();
      }
    };
    const onMessage = (event: { data?: string }) => {
      if (!live()) {
        detach();
        return;
      }
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
      const still = live();
      detach();
      if (!still) return;
      this.socket = null;
      this.clearHeartbeat();
      if (!this.stopped) {
        this.scheduleReconnect();
      }
    };
    const detach = () => {
      socket.removeEventListener("open", onOpen);
      socket.removeEventListener("message", onMessage);
      socket.removeEventListener("close", onClose);
      if (this.detachLiveSocket === detach) {
        this.detachLiveSocket = null;
      }
    };

    socket.addEventListener("open", onOpen);
    socket.addEventListener("message", onMessage);
    socket.addEventListener("close", onClose);
    this.detachLiveSocket = detach;
  }

  private unbindSocket(): void {
    this.detachLiveSocket?.();
    this.detachLiveSocket = null;
  }

  private onFrame(frame: ServerFrame): void {
    const epoch = this.epoch;
    const alive = () => epoch === this.epoch;
    switch (frame.op) {
      case "h":
        this.send({ op: "h" });
        return;
      case "ok": {
        const key = topicKey(frame);
        const current = this.gaps.get(key) ?? this.cursors.get(key);
        if (frame.ep && current && frame.ep !== current.ep) {
          this.markGap(frame, frame.ep);
          if (!alive()) return;
        }
        const gap = this.gaps.get(key);
        if (gap) {
          gap.ep = frame.ep ?? gap.ep;
          gap.cursor = Math.max(gap.cursor ?? 0, frame.n);
          gap.headKnown = true;
          this.finishGap(key);
          return;
        }
        const old = this.cursors.get(key);
        if (!old || frame.n > old.n || (frame.ep && !old.ep))
          this.cursors.set(key, { ep: frame.ep ?? old?.ep, n: frame.n });
        return;
      }
      case "e": {
        const key = topicKey(frame);
        const current = this.gaps.get(key) ?? this.cursors.get(key);
        if (frame.ep && current && frame.ep !== current.ep) {
          this.markGap(frame, frame.ep, true);
          if (!alive()) return;
        }
        const gap = this.gaps.get(key);
        const { accept, cursor } = nextCursor(
          gap ? gap.cursor : this.cursors.get(key)?.n,
          frame.n,
        );
        // After an epoch reset, the old transport number is irrelevant.
        const newEpoch = frame.ep && frame.ep !== this.cursors.get(key)?.ep;
        const result =
          gap && newEpoch && gap.cursor === undefined
            ? { accept: true, cursor: frame.n }
            : { accept, cursor };
        if (gap) {
          gap.cursor = result.cursor;
          gap.ep = frame.ep ?? gap.ep;
          // Live delivery may announce an epoch with gap + e rather than a
          // subscribe acknowledgement. The event itself is a current head.
          gap.headKnown = true;
          this.finishGap(key);
        } else
          this.cursors.set(key, {
            ep: frame.ep ?? this.cursors.get(key)?.ep,
            n: result.cursor,
          });
        if (result.accept) {
          for (const listener of this.eventListeners) {
            if (!alive()) return;
            listener(frame);
          }
        }
        return;
      }
      case "sig":
        for (const listener of this.sigListeners) {
          if (!alive()) return;
          listener(frame);
        }
        return;
      case "gap":
        this.markGap(frame, frame.ep);
        // The announced head may be below our cursor even in the same epoch.
        if (alive()) this.gaps.get(topicKey(frame))!.cursor = undefined;
        return;
      case "resync": {
        this.resyncVersion++;
        // Even a socket with no known topic must rediscover membership/DMs.
        for (const [key, topic] of this.desired) {
          this.markGap(topic, this.cursors.get(key)?.ep);
          if (!alive()) return;
          this.send(
            resumeFrame(
              topic,
              this.cursors.get(key)?.n,
              this.cursors.get(key)?.ep,
            ),
          );
        }
        for (const listener of this.resyncListeners) {
          if (!alive()) return;
          listener();
        }
        return;
      }
      case "dm":
        if (typeof frame.c !== "string" || !frame.c) return;
        this.discoveredDms.add(frame.c);
        for (const listener of this.dmListeners) {
          if (!alive()) return;
          listener(frame.c);
        }
        return;
      case "p":
        for (const listener of this.presenceListeners) {
          if (!alive()) return;
          listener(frame);
        }
        return;
      case "y":
        for (const listener of this.typingListeners) {
          if (!alive()) return;
          listener(frame);
        }
        return;
      case "err":
        for (const listener of this.errListeners) {
          if (!alive()) return;
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
    this.watchdogTimer = setInterval(
      () => {
        if (this.opts.now() - this.lastServerAt >= this.opts.deadMs) {
          this.socket?.close();
        }
      },
      Math.min(1_000, this.opts.heartbeatMs),
    );
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
