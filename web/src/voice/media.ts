import { api } from "../api/client.ts";
import type { TrackKind } from "../ws/protocol.ts";
import type {
  DtlsParameters,
  IceParameters,
  RtpCapabilities,
  RtpParameters,
  TransportOptions,
} from "mediasoup-client/types";

export const MEDIA_VERSION = 4 as const;
export type IceServer = RTCIceServer;
export type MediaTicket = {
  ticket: string;
  expires_in?: number;
  media_path: string;
  ice_servers: IceServer[];
};
export type MediaRequests = {
  j: { tk: string; w?: string; v: typeof MEDIA_VERSION };
  capabilities: { rtp: RtpCapabilities };
  transport: { direction: "send" | "recv" };
  connect: { transportId: string; dtls: DtlsParameters };
  restartIce: { transportId: string };
  closeTransport: { transportId: string };
  produce: {
    k: TrackKind;
    rtp: RtpParameters;
    epoch: string;
    parent?: string;
    lc?: string;
    expectedOldProducerId?: string;
    height?: number;
    paused?: boolean;
  };
  pauseProducer: { producerId: string };
  resumeProducer: { producerId: string };
  closeProducer: { producerId: string };
  consumerReady: { consumerId: string; generation: string };
  consumerFailed: { consumerId: string; generation: string };
  w: { u: string; k: "s" | "l"; on: boolean };
  q: { consumerId: string; generation: string; h: number; congested: boolean };
  l: Record<string, never>;
};
export type MediaResults = {
  j: {
    c: string;
    u: string;
    v: typeof MEDIA_VERSION;
    generation: string;
    routerRtpCapabilities: RtpCapabilities;
  };
  capabilities: Record<string, never>;
  transport: TransportOptions;
  connect: Record<string, never>;
  restartIce: { iceParameters: IceParameters };
  closeTransport: Record<string, never>;
  produce: { producerId: string };
  pauseProducer: Record<string, never>;
  resumeProducer: Record<string, never>;
  closeProducer: Record<string, never>;
  consumerReady: Record<string, never>;
  consumerFailed: Record<string, never>;
  w: Record<string, never>;
  q: Record<string, never>;
  l: Record<string, never>;
};
export type MediaMethod = keyof MediaRequests;
export type MediaClientFrame = {
  [K in MediaMethod]: K extends "l"
    ? { op: K; id: number }
    : MediaRequests[K] & { op: K; id: number };
}[MediaMethod];
export type ConsumerAnnouncement = {
  op: "consumer";
  consumerId: string;
  producerId: string;
  owner: string;
  k: TrackKind;
  epoch: string;
  generation: string;
  parent?: string;
  kind: "audio" | "video";
  rtpParameters: RtpParameters;
  paused: boolean;
};
export type MediaEvent =
  | ConsumerAnnouncement
  | { op: "consumerClosed"; consumerId: string; generation: string }
  | {
      op: "consumerState";
      consumerId: string;
      generation: string;
      paused: boolean;
    }
  | { op: "producerClosed"; producerId: string; epoch: string }
  | {
      op: "layers";
      consumerId: string;
      generation: string;
      spatialLayer: number | null;
      temporalLayer: number | null;
    };
export type MediaServerFrame =
  | { op: "result"; id: number; data: unknown }
  | { op: "err"; id?: number; e: string; lc?: string }
  | MediaEvent;
export type MediaSocket = {
  send(frame: MediaClientFrame): void;
  close(): void;
  onFrame(handler: (frame: MediaServerFrame) => void): () => void;
  onClose(handler: () => void): () => void;
};
export type OpenMedia = (url: string) => MediaSocket;
export type MediaRequest = <K extends MediaMethod>(
  method: K,
  data: MediaRequests[K],
  /** Local absolute operation budget; never serialized onto the wire. */
  deadlineEpochMs?: number,
) => Promise<MediaResults[K]>;
export class MediaError extends Error {
  constructor(
    readonly code: string,
    readonly lc?: string,
  ) {
    super(code);
    this.name = "MediaError";
  }
}
const UUID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i;
const trackKinds = new Set(["a", "v", "s", "l", "sa", "la"]);
const record = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
const id = (v: unknown): v is string =>
  typeof v === "string" && v.length > 0 && v.length <= 256;
const uuid = (v: unknown): v is string => typeof v === "string" && UUID.test(v);
const requestId = (v: unknown): v is number =>
  Number.isInteger(v) && Number(v) > 0 && Number(v) <= 0xffffffff;
const layer = (v: unknown) =>
  v === null || (Number.isInteger(v) && Number(v) >= 0 && Number(v) <= 255);

/** Reject malformed identities before dispatch. RTP details also pass SDK validation. */
export function decodeMediaFrame(raw: unknown): MediaServerFrame | null {
  if (!record(raw)) return null;
  if (raw.op === "result")
    return requestId(raw.id) && "data" in raw
      ? (raw as MediaServerFrame)
      : null;
  if (raw.op === "err")
    return id(raw.e) &&
      (raw.id === undefined || requestId(raw.id)) &&
      (raw.lc === undefined || uuid(raw.lc))
      ? (raw as MediaServerFrame)
      : null;
  if (raw.op === "producerClosed")
    return id(raw.producerId) && uuid(raw.epoch) ? (raw as MediaEvent) : null;
  if (!id(raw.consumerId) || !uuid(raw.generation)) return null;
  if (raw.op === "consumerClosed") return raw as MediaEvent;
  if (raw.op === "consumerState")
    return typeof raw.paused === "boolean" ? (raw as MediaEvent) : null;
  if (raw.op === "layers")
    return layer(raw.spatialLayer) && layer(raw.temporalLayer)
      ? (raw as MediaEvent)
      : null;
  if (
    raw.op !== "consumer" ||
    !id(raw.producerId) ||
    !uuid(raw.owner) ||
    !uuid(raw.epoch) ||
    !trackKinds.has(String(raw.k)) ||
    typeof raw.paused !== "boolean" ||
    !record(raw.rtpParameters)
  )
    return null;
  const audio = raw.k === "a" || raw.k === "sa" || raw.k === "la";
  if (raw.kind !== (audio ? "audio" : "video")) return null;
  if ((raw.k === "sa" || raw.k === "la") && !id(raw.parent)) return null;
  if (raw.parent !== undefined && !id(raw.parent)) return null;
  return raw as ConsumerAnnouncement;
}
export function mediaWsUrl(path: string): string {
  if (typeof location === "undefined") return path;
  return `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}${path}`;
}
export function requestMediaTicket(channelId: string): Promise<MediaTicket> {
  return api<MediaTicket>(`/channels/${channelId}/media-ticket`, {
    method: "POST",
  });
}
export function isOurTicket(ticket: string): boolean {
  return /^[abcdefghjkmnpqrstuvwxyz23456789]{12}$/.test(ticket);
}
export function openMediaSocket(url: string): MediaSocket {
  const socket = new WebSocket(url);
  const listeners = new Set<(frame: MediaServerFrame) => void>();
  const closers = new Set<() => void>();
  const pending: MediaClientFrame[] = [];
  let closed = false;
  const fail = () => {
    if (closed) return;
    closed = true;
    pending.length = 0;
    for (const handler of closers) handler();
    closers.clear();
  };
  socket.addEventListener("open", () => {
    if (closed) return;
    for (const frame of pending) socket.send(JSON.stringify(frame));
    pending.length = 0;
  });
  socket.addEventListener("close", fail);
  socket.addEventListener("error", () => {
    fail();
    socket.close();
  });
  socket.addEventListener("message", (event) => {
    if (typeof event.data !== "string" || event.data.length > 1_048_576) return;
    let value: unknown;
    try {
      value = JSON.parse(event.data) as unknown;
    } catch {
      return;
    }
    const frame = decodeMediaFrame(value);
    if (frame) for (const listener of listeners) listener(frame);
  });
  return {
    send(frame) {
      if (closed) throw new MediaError("connection_closed");
      if (socket.readyState === WebSocket.OPEN)
        socket.send(JSON.stringify(frame));
      else if (pending.length < 64) pending.push(frame);
      else throw new MediaError("request_overflow");
    },
    close() {
      fail();
      socket.close();
    },
    onFrame(handler) {
      listeners.add(handler);
      return () => {
        listeners.delete(handler);
      };
    },
    onClose(handler) {
      if (closed) {
        handler();
        return () => {};
      }
      closers.add(handler);
      return () => {
        closers.delete(handler);
      };
    },
  };
}
