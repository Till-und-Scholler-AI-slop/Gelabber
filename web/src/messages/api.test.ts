import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import { putPresigned, uploadTimeoutMs } from "./api.ts";
import { SIZE_MAX } from "./rules.ts";

afterEach(() => {
  vi.unstubAllGlobals();
});

function file(): File {
  return new File([new Uint8Array([1, 2, 3, 4])], "cat.png", {
    type: "image/png",
  });
}

describe("uploadTimeoutMs", () => {
  it("gives a large file time on a slow uplink", () => {
    expect(uploadTimeoutMs(0)).toBe(120_000);
    // 25 MiB at 512 kbit/s takes about 7 minutes.
    expect(uploadTimeoutMs(SIZE_MAX)).toBeGreaterThan((SIZE_MAX * 8) / 512);
  });
});

describe("putPresigned", () => {
  it("does not set the forbidden content-length header", async () => {
    const fetchMock = vi.fn<typeof fetch>(() =>
      Promise.resolve(new Response(null, { status: 200 })),
    );
    vi.stubGlobal("fetch", fetchMock);

    await putPresigned("https://minio.example/upload", file(), {
      "Content-Type": "image/png",
      "Content-Length": "4",
    });

    const init = fetchMock.mock.calls[0]?.[1];
    expect(init).toBeDefined();
    if (init === undefined) return;
    const headers = new Headers(init.headers);
    expect(headers.get("content-type")).toBe("image/png");
    expect(headers.get("content-length")).toBeNull();
    expect(init.method).toBe("PUT");
  });

  it("says when the upload is aborted", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>(() =>
        Promise.reject(new DOMException("aborted", "AbortError")),
      ),
    );
    await expect(
      putPresigned("https://minio.example/upload", file(), {
        "Content-Type": "image/png",
      }),
    ).rejects.toThrow("Der Upload wurde abgebrochen.");
  });

  it("says when the upload times out", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>(() =>
        Promise.reject(new DOMException("timed out", "TimeoutError")),
      ),
    );
    await expect(
      putPresigned("https://minio.example/upload", file(), {
        "Content-Type": "image/webp",
      }),
    ).rejects.toThrow(
      "Der Upload hat zu lange gedauert und wurde abgebrochen.",
    );
  });

  it("says when the object store rejects the PUT", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>(() =>
        Promise.resolve(new Response("no", { status: 403 })),
      ),
    );
    await expect(
      putPresigned("https://minio.example/upload", file(), {
        "Content-Type": "image/jpeg",
      }),
    ).rejects.toThrow("Der Upload ist fehlgeschlagen.");
  });

  it("keeps a network failure as a network error", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>(() =>
        Promise.reject(new TypeError("Failed to fetch")),
      ),
    );
    await expect(
      putPresigned("https://minio.example/upload", file(), {
        "Content-Type": "image/png",
      }),
    ).rejects.toBeInstanceOf(ApiError);
  });
});
