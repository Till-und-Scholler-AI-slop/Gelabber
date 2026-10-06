import {
  MediaError,
  MEDIA_VERSION,
  type MediaClientFrame,
  type MediaMethod,
  type MediaRequests,
  type MediaResults,
  type MediaServerFrame,
  type MediaSocket,
} from "./media.ts";
import type { MediaConnection } from "./mediasoupConnection.ts";

const OUTBOUND_CAP = 64;
export const MEDIA_REQUEST_DEADLINE_MS = 10_000;

export class MediaRetry {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private attempts = 0;

  get active(): boolean {
    return this.attempts > 0;
  }

  cancel(reset = true): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    if (reset) this.attempts = 0;
  }

  schedule(retry: () => void, exhausted: () => void): void {
    if (this.timer !== null) return;
    if (this.attempts >= 7) {
      exhausted();
      return;
    }
    const base = Math.min(250 * 2 ** this.attempts++, 4000);
    const delay = base * (0.8 + Math.random() * 0.4);
    this.timer = setTimeout(() => {
      this.timer = null;
      retry();
    }, delay);
  }
}

type PendingRequest = {
  resolve(value: unknown): void;
  reject(error: Error): void;
  timer: ReturnType<typeof setTimeout>;
  deadline: number;
};

/** Each seat/watch owns its correlated socket, SDK connection and generation. */
export class MediaPeer {
  connection: MediaConnection | null = null;
  socket: MediaSocket | null = null;
  generation = 0;
  accepted = false;
  serverGeneration: string | null = null;
  private nextId = 1;
  private pending = new Map<number, PendingRequest>();
  private unbind: (() => void) | null = null;
  private unbindClose: (() => void) | null = null;
  private transportLive = false;

  isOpen(): boolean {
    return this.socket !== null && this.transportLive;
  }

  async request<K extends MediaMethod>(
    method: K,
    data: MediaRequests[K],
    deadlineEpochMs?: number,
  ): Promise<MediaResults[K]> {
    if (!this.isOpen()) throw new MediaError("connection_closed");
    if (method !== "j" && !this.accepted) throw new MediaError("join_required");
    if (this.pending.size >= OUTBOUND_CAP)
      throw new MediaError("request_overflow");
    const deadline = Math.min(
      Date.now() + MEDIA_REQUEST_DEADLINE_MS,
      deadlineEpochMs ?? Infinity,
    );
    if (!Number.isFinite(deadline) || Date.now() >= deadline)
      throw new MediaError("request_timeout");
    if (this.nextId > 0xffffffff) {
      this.socket?.close();
      throw new MediaError("request_id_exhausted");
    }
    const generation = this.generation,
      socket = this.socket;
    const requestId = this.nextId++;
    const response = await new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(
        () => {
          this.pending.delete(requestId);
          reject(new MediaError("request_timeout"));
        },
        Math.max(0, deadline - Date.now()),
      );
      this.pending.set(requestId, { resolve, reject, timer, deadline });
      try {
        this.socket!.send({
          op: method,
          id: requestId,
          ...data,
        } as MediaClientFrame);
      } catch (error) {
        clearTimeout(timer);
        this.pending.delete(requestId);
        reject(
          error instanceof Error ? error : new MediaError("connection_closed"),
        );
      }
    });
    if (Date.now() >= deadline) throw new MediaError("request_timeout");
    if (
      this.generation !== generation ||
      this.socket !== socket ||
      !this.isOpen()
    )
      throw new MediaError("connection_closed");
    if (!response || typeof response !== "object" || Array.isArray(response))
      throw new MediaError("invalid_response");
    if (method === "j") {
      const joined = response as Partial<MediaResults["j"]>;
      if (joined.v !== MEDIA_VERSION) throw new MediaError("update_required");
      if (
        typeof joined.generation !== "string" ||
        !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(
          joined.generation,
        ) ||
        !joined.routerRtpCapabilities ||
        typeof joined.c !== "string" ||
        typeof joined.u !== "string"
      )
        throw new MediaError("invalid_response");
      this.accepted = true;
      this.serverGeneration = joined.generation;
    }
    return response as MediaResults[K];
  }

  bind(
    socket: MediaSocket,
    onFrame: (frame: MediaServerFrame) => void,
    onTransportDead?: () => void,
  ): void {
    this.unbind?.();
    this.unbindClose?.();
    this.socket = socket;
    this.transportLive = true;
    this.accepted = false;
    this.unbind = socket.onFrame((frame) => {
      if (this.socket !== socket) return;
      if (
        frame.op === "result" ||
        (frame.op === "err" && frame.id !== undefined)
      ) {
        const request = this.pending.get(frame.id!);
        if (request) {
          clearTimeout(request.timer);
          this.pending.delete(frame.id!);
          if (Date.now() >= request.deadline)
            request.reject(new MediaError("request_timeout"));
          else if (frame.op === "err")
            request.reject(new MediaError(frame.e, frame.lc));
          else request.resolve(frame.data);
        }
        // The requesting operation owns its rejection and source cleanup.
        // A late/duplicate RPC error must not become an unsolicited room error.
        return;
      }
      onFrame(frame);
    });
    this.unbindClose = socket.onClose(() => {
      if (this.socket !== socket) return;
      this.transportLive = false;
      this.socket = null;
      this.cancelPending();
      this.unbind?.();
      this.unbind = null;
      this.unbindClose = null;
      onTransportDead?.();
    });
  }

  private cancelPending(): void {
    for (const request of this.pending.values()) {
      clearTimeout(request.timer);
      request.reject(new MediaError("connection_closed"));
    }
    this.pending.clear();
  }

  close(): void {
    // Explicitly withdraw the owned server peer before closing local resources.
    // WebSocket.close alone cannot enforce Leave against a retained receiver.
    // This final bounded frame owns no reply promise and never reuses an id.
    if (
      this.isOpen() &&
      this.accepted &&
      this.serverGeneration &&
      this.nextId <= 0xffffffff
    ) {
      try {
        this.socket!.send({ op: "l", id: this.nextId++ });
      } catch {
        /* Continue local teardown if the socket already failed. */
      }
    }
    this.generation += 1;
    this.nextId = 1;
    this.accepted = false;
    this.serverGeneration = null;
    this.transportLive = false;
    this.cancelPending();
    this.unbindClose?.();
    this.unbindClose = null;
    this.unbind?.();
    this.unbind = null;
    const socket = this.socket;
    this.socket = null;
    this.connection?.close();
    this.connection = null;
    socket?.close();
  }
}
