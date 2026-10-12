import { afterEach, describe, expect, it, vi } from "vitest";

import { randomUuid } from "./uuid.ts";

const V4 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const real = globalThis.crypto;

describe("randomUuid", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("uses crypto.randomUUID in a secure context", () => {
    const getRandomValues = vi.fn();
    vi.stubGlobal("crypto", {
      randomUUID: () => "11111111-2222-4333-8444-555555555555",
      getRandomValues,
    });
    expect(randomUuid()).toBe("11111111-2222-4333-8444-555555555555");
    expect(getRandomValues).not.toHaveBeenCalled();
  });

  it("builds version 4 ids from getRandomValues on a plain-http origin", () => {
    // An insecure context keeps getRandomValues and has no randomUUID.
    const getRandomValues = vi.fn((bytes: Uint8Array<ArrayBuffer>) =>
      real.getRandomValues(bytes),
    );
    vi.stubGlobal("crypto", { getRandomValues });
    const ids = Array.from({ length: 500 }, () => randomUuid());
    for (const id of ids) expect(id).toMatch(V4);
    expect(new Set(ids).size).toBe(ids.length);
    expect(getRandomValues).toHaveBeenCalledTimes(ids.length);
  });

  it("sets the version and variant bits whatever the random bytes are", () => {
    vi.stubGlobal("crypto", {
      getRandomValues: (bytes: Uint8Array) => bytes.fill(0x00),
    });
    expect(randomUuid()).toBe("00000000-0000-4000-8000-000000000000");
    vi.stubGlobal("crypto", {
      getRandomValues: (bytes: Uint8Array) => bytes.fill(0xff),
    });
    expect(randomUuid()).toBe("ffffffff-ffff-4fff-bfff-ffffffffffff");
    vi.stubGlobal("crypto", {
      getRandomValues: (bytes: Uint8Array) => {
        bytes.forEach((_, index) => (bytes[index] = index * 0x11));
        return bytes;
      },
    });
    expect(randomUuid()).toBe("00112233-4455-4677-8899-aabbccddeeff");
  });
});
