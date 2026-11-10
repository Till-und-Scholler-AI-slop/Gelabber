import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import type { Channel, ServerDetail, ServerRole } from "../servers/types.ts";

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}));
// Static markup never opens a popover: show the checklist's content.
vi.mock("./ui/Popover.tsx", () => ({
  Popover: ({ label, children }: { label: string; children: ReactNode }) => (
    <div role="dialog" aria-label={label}>
      {children}
    </div>
  ),
}));

import { channelRequest } from "../servers/channelRequest.ts";
import { ChannelDialog } from "./ServerDialogs.tsx";

const created_at = "2026-11-01T00:00:00Z";
const role = (
  id: string,
  name: string,
  position: number,
  color: ServerRole["color"],
): ServerRole => ({
  id,
  server_id: "s1",
  name,
  color,
  permissions: [],
  position,
  created_at,
});
const channel = (limited: boolean, role_ids: string[] = []): Channel => ({
  id: "spiele",
  server_id: "s1",
  category_id: null,
  name: "spiele",
  kind: "text",
  created_at,
  limited,
  role_ids,
});
const server = (roles: ServerRole[]): ServerDetail => ({
  id: "s1",
  name: "Wohnzimmer",
  owner_id: "lena",
  created_at,
  role: "owner",
  permissions: ["manage_channels"],
  member_permissions: [],
  categories: [],
  channels: [],
  members: [],
  roles,
});
const withRoles = server([
  role("r-reg", "Stammgäste", 1, "green"),
  role("r-mod", "Moderation", 0, "blue"),
]);

function render(
  detail: ServerDetail,
  state: Parameters<typeof ChannelDialog>[0]["state"],
) {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <ChannelDialog server={detail} state={state} onClose={() => {}} />
    </QueryClientProvider>,
  );
}

afterEach(() => setLocale("de"));

describe("Zugriff in the channel dialog", () => {
  it("follows Kategorie, reads the roles and keeps its hint under it", () => {
    const html = render(withRoles, {
      mode: "edit",
      channel: channel(true, ["r-reg", "r-mod"]),
    });
    expect(html.indexOf(">Kategorie</label>")).toBeLessThan(
      html.indexOf(">Zugriff</label>"),
    );
    // A button named by its label, like the select above it.
    expect(html).toMatch(
      /<label for="([^"]+)" class="ui-label">Zugriff<\/label><span class="ui-select"><button id="\1" type="button"/,
    );
    // The roles in position order.
    expect(html).toContain(">Moderation, Stammgäste</span></button>");
    expect(html).toContain(
      "Nur diese Rollen sehen den Kanal. Mit „Alle Mitglieder“ sieht ihn jeder auf dem Server.",
    );
  });

  it("lists Alle Mitglieder and then the roles, checked as the channel is", () => {
    const html = render(withRoles, {
      mode: "edit",
      channel: channel(true, ["r-reg"]),
    });
    const list = html.slice(html.indexOf('aria-label="Zugriff"'));
    const order = ["Alle Mitglieder", "Moderation", "Stammgäste"].map((name) =>
      list.indexOf(`</span>${name}</span>`),
    );
    expect(order.every((at) => at > 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    const checked = [...list.matchAll(/<input type="checkbox"[^>]*>/g)].map(
      ([input]) => input.includes("checked"),
    );
    expect(checked).toEqual([false, false, true]);
    expect(list).not.toContain("Servereinstellungen");
  });

  it("reads Alle Mitglieder for an open channel and Nur Inhaber without roles", () => {
    expect(
      render(withRoles, { mode: "edit", channel: channel(false) }),
    ).toContain(">Alle Mitglieder</span></button>");
    expect(
      render(withRoles, { mode: "edit", channel: channel(true) }),
    ).toContain(">Nur Inhaber</span></button>");
  });

  it("offers only Alle Mitglieder and where roles come from on a server without roles", () => {
    const html = render(server([]), { mode: "create", categoryId: null });
    const list = html.slice(html.indexOf('aria-label="Zugriff"'));
    expect(list.match(/type="checkbox"/g)).toHaveLength(1);
    expect(list).toContain("Alle Mitglieder");
    expect(list).toContain("Rollen legst du in den Servereinstellungen an.");
  });

  it("is in English too", () => {
    setLocale("en");
    const html = render(server([]), { mode: "create", categoryId: null });
    expect(html).toContain(">Access</label>");
    expect(html).toContain(">All members</span></button>");
    expect(html).toContain(
      "Only these roles see the channel. With “All members” everyone on the server sees it.",
    );
    expect(html).toContain("You create roles in the server settings.");
  });
});

describe("channelRequest", () => {
  const form = {
    name: "spiele",
    kind: "text" as const,
    categoryId: "cat",
    initialAccess: null,
    access: null,
  };

  it("sends the chosen roles with the other fields in one call", () => {
    expect(
      channelRequest({ id: "spiele" }, { ...form, access: ["r-mod", "r-reg"] }),
    ).toEqual({
      mode: "edit",
      id: "spiele",
      patch: {
        name: "spiele",
        category_id: "cat",
        role_ids: ["r-mod", "r-reg"],
      },
    });
    expect(
      channelRequest(null, { ...form, categoryId: "", access: ["r-mod"] }),
    ).toEqual({
      mode: "create",
      input: {
        name: "spiele",
        kind: "text",
        category_id: null,
        role_ids: ["r-mod"],
      },
    });
  });

  it("sends [] when a limited channel opens to Alle Mitglieder", () => {
    const request = channelRequest(
      { id: "spiele" },
      { ...form, initialAccess: ["r-mod"], access: null },
    );
    expect(request.mode === "edit" && request.patch.role_ids).toEqual([]);
  });

  it("leaves access out while it is unchanged", () => {
    const edit = channelRequest(
      { id: "spiele" },
      { ...form, initialAccess: [], access: [] },
    );
    expect(edit.mode === "edit" && edit.patch.role_ids).toBeUndefined();
    const create = channelRequest(null, form);
    expect(create.mode === "create" && create.input.role_ids).toBeUndefined();
  });
});
