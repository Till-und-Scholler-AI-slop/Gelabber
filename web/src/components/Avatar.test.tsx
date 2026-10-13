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
