// Media path (issue 11): short join ticket + own WS. SDP/ICE never ride
// the chat gateway. First audio/video is background work — the UI already
// flipped on join click.

import { api } from "../api/client.ts";

export type IceServer = {
  urls: string | string[];
  username?: string;
  credential?: string;
};

export type MediaTicket = {
  ticket: string;
  expires_in: number;
  media_path: string;
  ice_servers: IceServer[];
};

export type MediaClientFrame =
  | { op: "j"; tk: string }
  | { op: "o"; sdp: string }
  | { op: "a"; sdp: string }
  | { op: "i"; ice: string; mid?: string }
  | { op: "p"; k: "v" | "s" | "l" }
  | { op: "l" };

export type MediaServerFrame =
  | { op: "ok"; c: string; u: string }
  | { op: "o"; sdp: string }
  | { op: "a"; sdp: string }
  | { op: "i"; ice: string; mid?: string }
  | { op: "err"; e: string };

export type MediaSocket = {
  send(frame: MediaClientFrame): void;
  close(): void;
  onFrame(handler: (frame: MediaServerFrame) => void): () => void;
};

export function mediaWsUrl(path: string): string {
  if (typeof location === "undefined") {
    return path;
  }
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}${path}`;
}

export function requestMediaTicket(channelId: string): Promise<MediaTicket> {
  return api<MediaTicket>(`/channels/${channelId}/media-ticket`, {
    method: "POST",
  });
}

export type OpenMedia = (url: string) => MediaSocket;

export function openMediaSocket(url: string): MediaSocket {
  const socket = new WebSocket(url);
  const listeners = new Set<(frame: MediaServerFrame) => void>();
  const pending: MediaClientFrame[] = [];
  let open = false;

  socket.addEventListener("open", () => {
    open = true;
    for (const frame of pending) {
      socket.send(JSON.stringify(frame));
    }
    pending.length = 0;
  });
  socket.addEventListener("message", (event) => {
    if (typeof event.data !== "string") return;
    let value: unknown;
    try {
      value = JSON.parse(event.data) as unknown;
    } catch {
      return;
    }
    if (
      value === null ||
      typeof value !== "object" ||
      !("op" in value) ||
      typeof (value as { op: unknown }).op !== "string"
    ) {
      return;
    }
    const op = (value as { op: string }).op;
    if (op === "ok" || op === "o" || op === "a" || op === "i" || op === "err") {
      for (const listener of listeners) {
        listener(value as MediaServerFrame);
      }
    }
  });

  return {
    send(frame) {
      if (open && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify(frame));
        return;
      }
      pending.push(frame);
    },
    close() {
      open = false;
      pending.length = 0;
      socket.close();
    },
    onFrame(handler) {
      listeners.add(handler);
      return () => {
        listeners.delete(handler);
      };
    },
  };
}

export function isOurTicket(ticket: string): boolean {
  return /^[abcdefghjkmnpqrstuvwxyz23456789]{12}$/.test(ticket);
}
