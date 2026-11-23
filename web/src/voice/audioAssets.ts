// The pinned files of public/audio: downloaded from this origin, checked
// against their SHA-256 and loaded once per page.
import { translate } from "../i18n/catalog.ts";
import { diagnostics } from "../i18n/catalogs/diagnostics.ts";

/** A download that stops arriving gives up; a slow one that keeps arriving
 * does not (the files are megabytes and may be sent uncompressed). */
const STALL_MS = 8_000;
/** After a failed load, the next tries wait this long, longer after each
 * failure. */
const RETRY_MS = [30_000, 120_000, 600_000];

async function fetchAsset(url: string): Promise<ArrayBuffer> {
  const abort = new AbortController();
  let stall: ReturnType<typeof setTimeout> | undefined;
  const arm = () => {
    clearTimeout(stall);
    stall = setTimeout(
      () =>
        abort.abort(
          new Error(translate(diagnostics, "processing.error.modelStalled")),
        ),
      STALL_MS,
    );
  };
  arm();
  try {
    const response = await fetch(url, {
      cache: "force-cache",
      signal: abort.signal,
    });
    if (!response.ok)
      throw new Error(translate(diagnostics, "processing.error.modelLoad"));
    if (!response.body) return await response.arrayBuffer();
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    for (;;) {
      arm();
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      size += value.byteLength;
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return bytes.buffer;
  } finally {
    clearTimeout(stall);
  }
}

/** `/audio/<name>`, refused unless its SHA-256 is `sha256`. The hash in
 * the URL keeps a cached copy of another version out. */
export async function fetchVerified(
  name: string,
  sha256: string,
): Promise<ArrayBuffer> {
  const bytes = await fetchAsset(`/audio/${name}?sha256=${sha256}`);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  const hash = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  if (hash !== sha256)
    throw new Error(translate(diagnostics, "processing.error.modelChecksum"));
  return bytes;
}

/** `load` once; its result is kept. After a failure, `get` fails at once
 * with the same error until the retry delay has passed. */
export function onceWithRetry<T>(load: () => Promise<T>): {
  get: () => Promise<T>;
  reset: () => void;
} {
  let pending: Promise<T> | null = null;
  let failure: { error: unknown; count: number; until: number } | null = null;
  return {
    get() {
      if (failure && Date.now() < failure.until)
        return Promise.reject(failure.error);
      pending ??= load().then(
        (value) => {
          failure = null;
          return value;
        },
        (error: unknown) => {
          pending = null;
          const count = (failure?.count ?? 0) + 1;
          failure = {
            error,
            count,
            until: Date.now() + RETRY_MS[Math.min(count, RETRY_MS.length) - 1]!,
          };
          throw error;
        },
      );
      return pending;
    },
    reset() {
      pending = null;
      failure = null;
    },
  };
}
