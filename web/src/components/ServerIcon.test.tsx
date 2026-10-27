import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ServerIconFace } from "./ServerIcon.tsx";

describe("server icon face", () => {
  it("shows the initials without an icon", () => {
    for (const url of [null, undefined, ""]) {
      const html = renderToStaticMarkup(
        <ServerIconFace url={url} fallback="TS" />,
      );
      expect(html).toBe("TS");
    }
  });

  it("shows the uploaded icon as decoration of the named tile", () => {
    const html = renderToStaticMarkup(
      <ServerIconFace url="/api/server-icons/a/b" fallback="TS" />,
    );
    expect(html).toContain('src="/api/server-icons/a/b"');
    expect(html).toContain('alt=""');
    expect(html).not.toContain("TS");
  });
});
