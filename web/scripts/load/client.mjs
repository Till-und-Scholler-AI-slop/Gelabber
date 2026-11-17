/* global fetch, WebSocket, URL, setTimeout, setInterval, clearInterval */
// A small Gelabber client for load tests: cookie jar, the CSRF double submit
// and the gateway WebSocket, with no browser. Used by chat.mjs,
// connections.mjs and call.mjs.

export const PASSWORD = "loadtest-password-123";

export class Client {
  constructor(base) {
    this.base = base;
    this.jar = new Map();
  }
  cookieHeader() {
    return [...this.jar].map(([k, v]) => `${k}=${v}`).join("; ");
  }
  absorb(setCookies) {
    for (const line of setCookies) {
      const [pair] = line.split(";");
      const at = pair.indexOf("=");
      if (at > 0)
        this.jar.set(pair.slice(0, at).trim(), pair.slice(at + 1).trim());
    }
  }
  csrf() {
    return this.jar.get("gelabber_csrf") ?? "";
  }
  async req(method, path, body) {
    const headers = { Cookie: this.cookieHeader(), Origin: this.base };
    if (body !== undefined) headers["Content-Type"] = "application/json";
    if (method !== "GET") headers["X-CSRF-Token"] = this.csrf();
    const res = await fetch(`${this.base}/api${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    this.absorb(res.headers.getSetCookie());
    const text = await res.text();
    if (!res.ok)
      throw new Error(`${method} ${path}: ${res.status} ${text.slice(0, 200)}`);
    return text ? JSON.parse(text) : null;
  }
  async register(email, name) {
    await this.req("GET", "/auth/session"); // sets the CSRF cookie
    return this.req("POST", "/auth/register", {
      email,
      password: PASSWORD,
      name,
      locale: "de",
    });
  }
  /**
   * Opens the gateway; resolves once open. `onFrame` gets every frame but
   * heartbeats; `onClose` runs when the server closes the socket (a full
   * outbound queue, a failed session check), not after `close()`.
   */
  connect(onFrame, onClose) {
    const url = new URL("/ws", this.base);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    // Node's WebSocket (undici) accepts request headers here; browsers do not.
    const ws = new WebSocket(url, {
      headers: { Cookie: this.cookieHeader(), Origin: this.base },
    });
    this.socket = ws;
    this.closing = false;
    ws.onclose = (event) => {
      clearInterval(this.beat);
      if (!this.closing) onClose?.(event);
    };
    ws.onmessage = (message) => {
      let frame;
      try {
        frame = JSON.parse(message.data);
      } catch {
        return;
      }
      if (frame.op === "h") ws.send('{"op":"h"}');
      else onFrame?.(frame);
    };
    this.beat = setInterval(
      () => ws.readyState === 1 && ws.send('{"op":"h"}'),
      15_000,
    );
    return new Promise((resolve, reject) => {
      ws.onopen = () => resolve(ws);
      ws.onerror = () => reject(new Error("gateway connection failed"));
    });
  }
  subscribe(serverId, channelId) {
    this.socket.send(JSON.stringify({ op: "s", s: serverId, c: channelId }));
  }
  close() {
    this.closing = true;
    clearInterval(this.beat);
    this.socket?.close();
  }
}

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Runs `fn` over `items` with at most `limit` at a time; results keep their order. */
export async function pool(items, limit, fn) {
  const out = new Array(items.length);
  let next = 0;
  await Promise.all(
    Array.from({ length: Math.min(limit, items.length) }, async () => {
      while (next < items.length) {
        const i = next++;
        out[i] = await fn(items[i], i);
      }
    }),
  );
  return out;
}

/** Unlabelled `gelabber_*` gauges and counters from a Prometheus text page. */
export function parseMetrics(text) {
  const values = {};
  for (const line of text.split("\n")) {
    const m = /^gelabber_([a-z_]+) ([0-9.e+-]+)$/.exec(line);
    if (m) values[m[1]] = Number(m[2]);
  }
  return values;
}
