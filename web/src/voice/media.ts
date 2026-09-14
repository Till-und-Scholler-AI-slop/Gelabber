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

const OPUS_FMTP =
  "minptime=10;useinbandfec=1;stereo=0;maxaveragebitrate=128000";

/**
 * Keep voice on Opus with in-band FEC. Default Chrome fmtp is fine on a
 * LAN; without FEC a few lost packets on a real homelab path sound torn.
 */
export function tuneAudioSdp(sdp: string): string {
  const nl = sdp.includes("\r\n") ? "\r\n" : "\n";
  const lines = sdp.split(/\r?\n/);
  const out: string[] = [];
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i] ?? "";
    out.push(line);
    const rtpmap = /^a=rtpmap:(\d+) opus\/48000/i.exec(line);
    if (!rtpmap) continue;
    const pt = rtpmap[1] ?? "";
    const prefix = `a=fmtp:${pt}`;
    const next = lines[i + 1] ?? "";
    if (next.toLowerCase().startsWith(prefix.toLowerCase())) {
      i += 1;
      const sp = next.indexOf(" ");
      const params = sp >= 0 ? next.slice(sp + 1).trim() : "";
      const parts = new Map<string, string>();
      for (const piece of params.split(";")) {
        const trimmed = piece.trim();
        if (!trimmed) continue;
        const eq = trimmed.indexOf("=");
        if (eq <= 0) continue;
        parts.set(trimmed.slice(0, eq).toLowerCase(), trimmed.slice(eq + 1));
      }
      if (!parts.has("minptime")) parts.set("minptime", "10");
      parts.set("useinbandfec", "1");
      parts.set("stereo", "0");
      parts.set("maxaveragebitrate", "128000");
      const body = [...parts.entries()]
        .map(([key, value]) => `${key}=${value}`)
        .join(";");
      out.push(`${prefix} ${body}`);
    } else {
      out.push(`${prefix} ${OPUS_FMTP}`);
    }
  }
  return out.join(nl);
}
