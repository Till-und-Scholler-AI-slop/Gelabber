// Media path (issue 11): short join ticket + own WS. SDP/ICE never ride
// the chat gateway. First audio/video is background work — the UI already
// flipped on join click.

import { api } from "../api/client.ts";
import { audioBitrate } from "./settings.ts";

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
  | { op: "u"; k: "v" | "s" | "l" }
  | { op: "x" }
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
  /** Transport closed or failed. Fires at most once. */
  onClose(handler: () => void): () => void;
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
  const closers = new Set<() => void>();
  const pending: MediaClientFrame[] = [];
  let open = false;
  let closed = false;

  const fail = () => {
    if (closed) return;
    closed = true;
    open = false;
    pending.length = 0;
    for (const handler of closers) handler();
    closers.clear();
  };

  socket.addEventListener("open", () => {
    if (closed) return;
    open = true;
    for (const frame of pending) {
      socket.send(JSON.stringify(frame));
    }
    pending.length = 0;
  });
  socket.addEventListener("close", fail);
  socket.addEventListener("error", fail);
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
      if (closed) return;
      if (open && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify(frame));
        return;
      }
      pending.push(frame);
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

export function isOurTicket(ticket: string): boolean {
  return /^[abcdefghjkmnpqrstuvwxyz23456789]{12}$/.test(ticket);
}

/**
 * Keep voice on Opus with in-band FEC. Default Chrome fmtp is fine on a
 * LAN; without FEC a few lost packets on a real homelab path sound torn.
 *
 * Chromium puts `a=rtcp-fb` between rtpmap and fmtp. Search the `m=`
 * section for the existing fmtp of that payload type, merge, emit one line.
 */
export function tuneAudioSdp(sdp: string, bitrate = audioBitrate()): string {
  const nl = sdp.includes("\r\n") ? "\r\n" : "\n";
  const lines = sdp.split(/\r?\n/);
  const firstM = lines.findIndex((line) => line.startsWith("m="));
  if (firstM < 0) return tuneOpusSection(lines, bitrate).join(nl);
  const out = lines.slice(0, firstM);
  let i = firstM;
  while (i < lines.length) {
    const start = i;
    i += 1;
    while (i < lines.length && !lines[i]?.startsWith("m=")) i += 1;
    out.push(...tuneOpusSection(lines.slice(start, i), bitrate));
  }
  return out.join(nl);
}

/** Read the Opus maxaveragebitrate the client actually put on the SDP. */
export function opusMaxAverageBitrate(sdp: string): number | null {
  const match = /a=fmtp:\d+[^\n]*maxaveragebitrate=(\d+)/i.exec(sdp);
  if (!match?.[1]) return null;
  const value = Number(match[1]);
  return Number.isFinite(value) ? value : null;
}

function tuneOpusSection(section: string[], bitrate: number): string[] {
  const pts: string[] = [];
  for (const line of section) {
    const rtpmap = /^a=rtpmap:(\d+) opus\/48000/i.exec(line);
    if (rtpmap?.[1]) pts.push(rtpmap[1]);
  }
  if (pts.length === 0) return section;

  const lines = [...section];
  for (const pt of pts) {
    const prefix = `a=fmtp:${pt}`;
    const found: number[] = [];
    for (let i = 0; i < lines.length; i += 1) {
      if (isFmtpForPt(lines[i] ?? "", pt)) found.push(i);
    }
    const parts = new Map<string, string>();
    for (const idx of found) mergeFmtpParams(parts, lines[idx] ?? "");
    applyVoiceFmtp(parts, bitrate);
    const merged = `${prefix} ${[...parts.entries()]
      .map(([key, value]) => `${key}=${value}`)
      .join(";")}`;
    if (found.length > 0) {
      const keep = found[0] ?? 0;
      lines[keep] = merged;
      for (let k = found.length - 1; k >= 1; k -= 1) {
        lines.splice(found[k] ?? 0, 1);
      }
    } else {
      const rtpmapIdx = lines.findIndex((line) =>
        new RegExp(`^a=rtpmap:${pt} opus/48000`, "i").test(line),
      );
      lines.splice(rtpmapIdx >= 0 ? rtpmapIdx + 1 : lines.length, 0, merged);
    }
  }
  return lines;
}

function isFmtpForPt(line: string, pt: string): boolean {
  return new RegExp(`^a=fmtp:${pt}(?:\\s|$)`, "i").test(line);
}

function mergeFmtpParams(parts: Map<string, string>, line: string): void {
  const sp = line.indexOf(" ");
  const params = sp >= 0 ? line.slice(sp + 1).trim() : "";
  for (const piece of params.split(";")) {
    const trimmed = piece.trim();
    if (!trimmed) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    parts.set(trimmed.slice(0, eq).toLowerCase(), trimmed.slice(eq + 1));
  }
}

function applyVoiceFmtp(parts: Map<string, string>, bitrate: number): void {
  if (!parts.has("minptime")) parts.set("minptime", "10");
  parts.set("useinbandfec", "1");
  parts.set("stereo", "0");
  parts.set("maxaveragebitrate", String(bitrate));
}
