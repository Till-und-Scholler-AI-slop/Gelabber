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

describe("avatar tint", () => {
  it("puts initials on a tile tint picked by the name", () => {
    const html = renderToStaticMarkup(<Avatar name="Ada" url={null} />);
    expect(html).toMatch(/--avatar-tint:var\(--lr-tile-[1-8]\)/);
    expect(
      renderToStaticMarkup(<Avatar name="Ada" url="" size="lg" />),
    ).toContain(html.match(/--avatar-tint:[^;"]+/)?.[0]);
  });

  it("leaves a picture without a tint", () => {
    const html = renderToStaticMarkup(
      <Avatar name="Ada" url="https://example.org/ada.png" />,
    );
    expect(html).not.toContain("--avatar-tint");
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
