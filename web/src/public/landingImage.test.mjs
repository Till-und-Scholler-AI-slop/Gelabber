// The landing page shows real screenshots of the app
// (scripts/landing/shot.mjs), never the drawn preview they replaced.
/* global URL */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const sha256 = (path) =>
  createHash("sha256")
    .update(readFileSync(new URL(path, import.meta.url)))
    .digest("hex");

const DRAWN = sha256("../../../docs/design/v0.10/landing-preview-en.png");

describe("landing screenshots", () => {
  it.each(["en", "de"])("app-%s.png is not the drawn preview", (lang) => {
    expect(sha256(`../../public/images/landing/app-${lang}.png`)).not.toBe(
      DRAWN,
    );
  });

  it("are two different pictures", () => {
    expect(sha256("../../public/images/landing/app-en.png")).not.toBe(
      sha256("../../public/images/landing/app-de.png"),
    );
  });
});
