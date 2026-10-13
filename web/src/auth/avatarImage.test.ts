import { afterEach, describe, expect, it, vi } from "vitest";

import {
  AVATAR_INPUT_MAX,
  AvatarImageError,
  avatarImageMessage,
  fallbackType,
  prepareAvatar,
} from "./avatarImage.ts";

// Node has no canvas: the scaling itself is covered in the browser only.
describe("prepareAvatar", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("refuses absurd inputs before decoding them", async () => {
    const decode = vi.fn();
    vi.stubGlobal("createImageBitmap", decode);
    const huge = { size: AVATAR_INPUT_MAX + 1 } as Blob;
    const error = await prepareAvatar(huge).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(AvatarImageError);
    expect((error as AvatarImageError).reason).toBe("too_large");
    expect(avatarImageMessage(error as AvatarImageError)).toMatch(/20 MB/);
    expect(decode).not.toHaveBeenCalled();
  });

  it("reports a file the browser cannot decode as unreadable", async () => {
    vi.stubGlobal(
      "createImageBitmap",
      vi.fn(() => Promise.reject(new DOMException("bad", "InvalidStateError"))),
    );
    const error = await prepareAvatar(new Blob(["not an image"])).catch(
      (e: unknown) => e,
    );
    expect(error).toBeInstanceOf(AvatarImageError);
    expect((error as AvatarImageError).reason).toBe("unreadable");
  });
});

describe("fallbackType", () => {
  it("uses JPEG for fully opaque pixels", () => {
    expect(fallbackType([10, 20, 30, 255, 0, 0, 0, 255])).toBe("image/jpeg");
  });

  it("keeps PNG when any pixel is transparent", () => {
    expect(fallbackType([10, 20, 30, 255, 0, 0, 0, 254])).toBe("image/png");
    expect(fallbackType(new Uint8ClampedArray([1, 2, 3, 0]))).toBe("image/png");
  });
});
