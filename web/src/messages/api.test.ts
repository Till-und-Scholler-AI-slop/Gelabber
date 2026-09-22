import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import { putPresigned } from "./api.ts";

afterEach(() => {
  vi.unstubAllGlobals();
});

function file(): File {
  return new File([new Uint8Array([1, 2, 3, 4])], "cat.png", {
    type: "image/png",
  });
}

describe("putPresigned", () => {
  it("does not set the forbidden content-length header", async () => {
    const fetchMock = vi.fn(() =>
      Promise.resolve(new Response(null, { status: 200 })),
    );
    vi.stubGlobal("fetch", fetchMock);

    await putPresigned("https://minio.example/upload", file(), {
      "Content-Type": "image/png",
      "Content-Length": "4",
    });

    const init = fetchMock.mock.calls[0]?.[1] as RequestInit;
    const headers = new Headers(init.headers);
    expect(headers.get("content-type")).toBe("image/png");
    expect(headers.get("content-length")).toBeNull();
    expect(init.method).toBe("PUT");
  });

  it("says when the upload is aborted", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new DOMException("aborted", "AbortError"))),
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
      vi.fn(() =>
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
      vi.fn(() => Promise.resolve(new Response("no", { status: 403 }))),
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
      vi.fn(() => Promise.reject(new TypeError("Failed to fetch"))),
    );
    await expect(
      putPresigned("https://minio.example/upload", file(), {
        "Content-Type": "image/png",
      }),
    ).rejects.toBeInstanceOf(ApiError);
  });
});
