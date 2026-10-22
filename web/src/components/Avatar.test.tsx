import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Avatar } from "./Avatar.tsx";

describe("avatar", () => {
  it("loads foreign avatar URLs without a referrer", () => {
    const html = renderToStaticMarkup(
      <Avatar name="Ada" url="https://upload.wikimedia.org/ada.png" />,
    );
    expect(html).toContain('referrerPolicy="no-referrer"');
  });
});

describe("decorative avatar", () => {
  it("is hidden from assistive technology next to a written name", () => {
    const html = renderToStaticMarkup(
      <Avatar name="Ada" url="https://example.org/ada.png" decorative />,
    );
    expect(html).toContain('alt=""');
    expect(html).toContain('aria-hidden="true"');
    expect(html).not.toContain("Avatar von Ada");
  });

  it("keeps its name when it stands alone", () => {
    const html = renderToStaticMarkup(
      <Avatar name="Ada" url="https://example.org/ada.png" />,
    );
    expect(html).toContain('alt="Avatar von Ada"');
    expect(html).not.toContain("aria-hidden");
  });
});
