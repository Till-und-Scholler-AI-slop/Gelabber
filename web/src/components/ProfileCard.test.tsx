import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { AvatarFrame } from "./AvatarFrame.tsx";
import { ProfileCardView } from "./ProfileCard.tsx";

const BANNER =
  "/api/banners/0f8fad5b-d9cb-469f-a165-70867728950e/0f8fad5b-d9cb-469f-a165-70867728950e";

describe("profile card", () => {
  it("shows every part someone filled in", () => {
    const html = renderToStaticMarkup(
      <ProfileCardView
        person={{
          name: "Ada",
          avatar_url: null,
          pronouns: "sie/ihr",
          status_text: "Rechnet gerade",
          bio: "Mathematikerin.\nSchreibt Programme.",
          accent_color: "#00aa88",
          banner_url: BANNER,
          avatar_decoration: "stars",
        }}
        presence="o"
      />,
    );
    expect(html).toContain("<h3>Ada</h3>");
    expect(html).toContain("sie/ihr");
    expect(html).toContain("Rechnet gerade");
    expect(html).toContain("Über mich");
    expect(html).toContain("Mathematikerin.\nSchreibt Programme.");
    expect(html).toContain(`src="${BANNER}"`);
    expect(html).toContain("--profile-accent:#00aa88");
    expect(html).toContain("gel-deco gel-deco-stars");
    expect(html).toContain("Online");
  });

  it("leaves unset parts out and falls back to the banner color", () => {
    const html = renderToStaticMarkup(
      <ProfileCardView
        person={{ name: "Bob", avatar_url: null, banner_color: "#123456" }}
      />,
    );
    expect(html).not.toContain("Über mich");
    expect(html).not.toContain("profile-card__status");
    expect(html).not.toContain("profile-card__pronouns");
    expect(html).not.toContain("gel-deco");
    expect(html).not.toContain("<img");
    expect(html).toContain("background:#123456");
  });

  it("never puts an arbitrary stored color into a style", () => {
    const html = renderToStaticMarkup(
      <ProfileCardView
        person={{
          name: "Eve",
          avatar_url: null,
          accent_color: "red;background:url(https://evil.example/x)",
          banner_color: "url(https://evil.example/y)",
        }}
      />,
    );
    expect(html).not.toContain("evil.example");
  });

  it("ignores decorations it does not know", () => {
    const html = renderToStaticMarkup(
      <AvatarFrame decoration={"crown" as never}>
        <span>avatar</span>
      </AvatarFrame>,
    );
    expect(html).toBe("<span>avatar</span>");
  });
});
