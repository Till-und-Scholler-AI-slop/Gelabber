import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { scopeGeneration } from "../auth/scope.ts";
import type { PublicProfile } from "../auth/types.ts";
import { profileKeys } from "../profile/queries.ts";
import { UserProfileDialog } from "./UserProfileDialog.tsx";
import { withCard } from "./profileStyle.ts";

const PEER = "22222222-2222-4222-8222-222222222222";
const OTHER = "33333333-3333-4333-8333-333333333333";

const card = (id: string, extra: Partial<PublicProfile>): PublicProfile => ({
  id,
  name: "Bob",
  avatar_url: null,
  created_at: "2026-10-01T00:00:00Z",
  ...extra,
});

describe("card merge (member rows and DM peers)", () => {
  const known = { name: "Bob", avatar_url: null, status_text: "Alt" };

  it("shows what is known until the card is loaded", () => {
    expect(withCard(known, undefined, PEER)).toEqual(known);
  });

  it("lets the fetched card win, including cleared fields", () => {
    const merged = withCard(
      known,
      card(PEER, { name: "Bobby", status_text: null, bio: "Hi" }),
      PEER,
    );
    expect(merged.name).toBe("Bobby");
    expect(merged.status_text).toBeNull();
    expect(merged.bio).toBe("Hi");
  });

  it("ignores a cached card of someone else", () => {
    expect(withCard(known, card(OTHER, { bio: "Fremd" }), PEER)).toEqual(known);
  });
});

describe("DM peer profile dialog", () => {
  function render(cached?: PublicProfile) {
    const client = new QueryClient();
    // Server rendering reads the session store's initial state (nobody
    // signed in), so the card's query key carries an empty user id.
    if (cached)
      client.setQueryData(
        profileKeys.detail("", scopeGeneration(), PEER),
        cached,
      );
    return renderToStaticMarkup(
      <QueryClientProvider client={client}>
        <UserProfileDialog
          userId={PEER}
          known={{ name: "Bob", avatar_url: null }}
          presence="i"
          open
          onClose={() => {}}
        />
      </QueryClientProvider>,
    );
  }

  it("opens with the peer's known name and presence", () => {
    const html = render();
    expect(html).toContain("Profil von Bob");
    expect(html).toContain("<h3>Bob</h3>");
    expect(html).toContain("Abwesend");
    expect(html).not.toContain("Über mich");
  });

  it("fills in the fetched card", () => {
    const html = render(
      card(PEER, {
        pronouns: "er/ihm",
        status_text: "Unterwegs",
        bio: "Baut Brücken.",
      }),
    );
    expect(html).toContain("er/ihm");
    expect(html).toContain("Unterwegs");
    expect(html).toContain("Baut Brücken.");
  });
});
