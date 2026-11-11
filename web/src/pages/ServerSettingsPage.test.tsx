import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  DirectoryStatus,
  JoinRequest,
  ListingSettings,
} from "../discover/types.ts";
import { setLocale } from "../i18n/locale.ts";

import type {
  Member,
  Permission,
  ServerDetail,
  ServerRole,
} from "../servers/types.ts";

const fixture = vi.hoisted(() => ({
  server: null as unknown as ServerDetail,
  directory: undefined as DirectoryStatus | undefined,
  listing: undefined as ListingSettings | undefined,
  requests: [] as JoinRequest[],
  requestCount: undefined as number | undefined,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="#">{children}</a>,
  useNavigate: () => vi.fn(),
  useParams: () => ({ serverId: "s1" }),
}));
vi.mock("../servers/queries.ts", () => {
  const mutation = () => ({ isPending: false, mutate: vi.fn() });
  return {
    isPendingId: (id: string) => id.startsWith("tmp:"),
    readServer: vi.fn(),
    useServer: () => ({ data: fixture.server, error: null }),
    useBans: () => ({ data: [] }),
    useInvites: () => ({ data: [] }),
    useBanMember: mutation,
    useKickMember: mutation,
    useCreateRole: mutation,
    useDeleteRole: mutation,
    useDeleteServer: mutation,
    useLeaveServer: mutation,
    useRevokeInvite: mutation,
    useServerIcon: mutation,
    useSetMemberRoles: mutation,
    useUnbanMember: mutation,
    useUpdateRole: mutation,
    useUpdateServer: mutation,
  };
});
vi.mock("../discover/queries.ts", () => {
  const mutation = () => ({ isPending: false, mutate: vi.fn() });
  return {
    useDirectoryStatus: () => ({ data: fixture.directory }),
    useListingSettings: () => ({ data: fixture.listing, error: null }),
    useJoinRequests: () => ({
      data: fixture.requests,
      isPending: false,
      error: null,
    }),
    useJoinRequestCount: () => ({ data: fixture.requestCount }),
    useSaveListingSettings: mutation,
    useAnswerJoinRequest: mutation,
    useJoinListing: mutation,
    useWithdrawRequest: mutation,
  };
});
// Static markup never opens a popover: show the role picker's content.
vi.mock("../components/ui/Popover.tsx", () => ({
  Popover: ({ label, children }: { label: string; children: ReactNode }) => (
    <div role="dialog" aria-label={label}>
      {children}
    </div>
  ),
}));

import { serverSettingsSections } from "./serverSettingsSections.ts";
import { RolesEditor } from "./ServerRoles.tsx";
import { ServerSettingsPage } from "./ServerSettingsPage.tsx";

const created_at = "2026-09-13T00:00:00Z";
const role = (
  id: string,
  name: string,
  position: number,
  color: ServerRole["color"],
  permissions: Permission[] = [],
): ServerRole => ({
  id,
  server_id: "s1",
  name,
  color,
  permissions,
  position,
  created_at,
});
const member = (
  user_id: string,
  name: string,
  role_ids: string[] = [],
): Member => ({
  user_id,
  name,
  avatar_url: null,
  joined_at: created_at,
  role: user_id === "lena" ? "owner" : "member",
  role_ids,
});
// The server of roles-editor.png and roles-members.png.
const moderation = role("r-mod", "Moderation", 0, "blue", [
  "moderate_members",
  "manage_messages",
  "mention_everyone",
]);
const regulars = role("r-reg", "Stammgäste", 1, "green");
const BASE: Permission[] = [
  "send_messages",
  "send_files",
  "join_voice",
  "go_live",
];
const MEMBERS = [
  member("lena", "Lena"),
  member("jonas", "Jonas", ["r-mod", "r-reg"]),
  member("mira", "Mira", ["r-reg"]),
  member("tom", "Tom", ["r-reg"]),
  member("ada", "Ada"),
];

function detail(
  permissions: Permission[],
  overrides: Partial<ServerDetail> = {},
): ServerDetail {
  return {
    id: "s1",
    name: "Wohnzimmer",
    owner_id: "lena",
    created_at,
    role: "member",
    permissions,
    member_permissions: [],
    categories: [],
    channels: [],
    members: [],
    roles: [],
    ...overrides,
  };
}

function render(node: ReactNode) {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      {node}
    </QueryClientProvider>,
  );
}

function page(
  permissions: ServerDetail["permissions"],
  overrides: Partial<ServerDetail> = {},
) {
  fixture.server = detail(permissions, overrides);
  return render(<ServerSettingsPage />);
}

const withRoles = {
  member_permissions: BASE,
  members: MEMBERS,
  roles: [regulars, moderation],
};

const marked = (html: string) =>
  [...html.matchAll(/<a [^>]*aria-current="location"[^>]*>([^<]*)</g)].map(
    (match) => match[1],
  );
const anchors = (html: string) =>
  [...html.matchAll(/<a href="#server-settings-[^"]*"[^>]*>([^<]*)</g)].map(
    (match) => match[1],
  );
/** The text of each row in the roles list, without its dot and lock. */
const roleRows = (html: string) =>
  [
    ...html.matchAll(
      /<button type="button" class="roles-editor__row"([^>]*)>.*?<span class="roles-editor__name">([^<]*)<\/span>.*?<span class="roles-editor__count">(\d+)<\/span><\/button>/g,
    ),
  ].map((match) => ({
    name: match[2],
    count: Number(match[3]),
    selected: match[1].includes('aria-current="true"'),
  }));
/** Each switch row of the panel: its label, the note and the input. */
const switches = (html: string) =>
  [
    ...html.matchAll(
      /<label class="[^"]*roles-permission">.*?<span class="ui-choice-text">([^<]*)<small>.*?<\/span>(.*?)(<input[^>]*>)<\/label>/g,
    ),
  ].map((match) => ({
    label: match[1],
    note: match[2].replace(/<[^>]+>/g, ""),
    checked: match[3].includes('checked=""'),
    disabled: match[3].includes('disabled=""'),
  }));

describe("server settings sections", () => {
  const sections = (permissions: Permission[], owner = false) =>
    serverSettingsSections(
      { role: owner ? "owner" : "member", permissions },
      false,
    );

  it("follow manage_server and moderate_members", () => {
    expect(sections(["manage_server", "moderate_members"])).toEqual([
      "allgemein",
      "rollen",
      "einladungen",
      "mitglieder",
      "sperren",
      "gefahrenzone",
    ]);
    // The ban list needs moderate_members, as unbanning does.
    expect(sections(["manage_server"])).toEqual([
      "allgemein",
      "rollen",
      "einladungen",
      "mitglieder",
      "gefahrenzone",
    ]);
    expect(sections(["moderate_members"])).toEqual([
      "mitglieder",
      "sperren",
      "gefahrenzone",
    ]);
    expect(sections(["send_messages"])).toEqual(["mitglieder", "gefahrenzone"]);
    expect(sections([], true)).toEqual(
      sections(["manage_server", "moderate_members"]),
    );
  });

  it("show in the anchor row, with Rollen where Rechte was", () => {
    expect(anchors(page(["manage_server", "moderate_members"]))).toEqual([
      "Allgemein",
      "Rollen",
      "Einladungen",
      "Mitglieder",
      "Sperren",
      "Gefahrenzone",
    ]);
    expect(anchors(page(["moderate_members"]))).toEqual([
      "Mitglieder",
      "Sperren",
      "Gefahrenzone",
    ]);
  });
});

describe("server settings page", () => {
  it("marks the first section in the anchor row, and only that one", () => {
    expect(marked(page(["manage_server"]))).toEqual(["Allgemein"]);
    // Members without manage_server start at Mitglieder.
    expect(marked(page([]))).toEqual(["Mitglieder"]);
  });

  it("says what applies at once and what waits for Speichern", () => {
    const html = page(["manage_server"]);
    expect(html).toContain(
      "Ein neues Bild gilt sofort, den Namen übernimmst du mit Speichern.",
    );
    expect(html).toContain(
      "Eine Rolle bündelt Rechte. Wer mehrere Rollen hat, bekommt die Rechte aller zusammen.",
    );
  });

  it("tells why Speichern waits while the name is unchanged", () => {
    const html = page(["manage_server"]);
    const hint = html.match(/<span id="([^"]+)"[^>]*>Keine Änderungen\.</);
    expect(hint).not.toBeNull();
    expect(html).toMatch(
      new RegExp(`<button[^>]*aria-describedby="${hint?.[1]}"[^>]*>Speichern`),
    );
  });
});

describe("roles editor", () => {
  it("lists Inhaber, the roles by position and Alle Mitglieder, with counts", () => {
    const html = page(["manage_server"], withRoles);
    expect(roleRows(html)).toEqual([
      { name: "Inhaber", count: 1, selected: false },
      { name: "Moderation", count: 1, selected: true },
      { name: "Stammgäste", count: 3, selected: false },
      { name: "Alle Mitglieder", count: 5, selected: false },
    ]);
    expect(html).toMatch(/>Inhaber<\/span><svg[^>]*roles-editor__lock/);
    expect(html).toMatch(/<button[^>]*>.*Neue Rolle<\/button>/);
  });

  it("edits the first custom role: name, colour and the nine switches", () => {
    const html = page(["manage_server"], withRoles);
    expect(html).toContain("<h3>Moderation</h3>");
    expect(html).toMatch(/<button[^>]*>Rolle löschen<\/button>/);
    expect(html).toMatch(
      /<input[^>]*id="role-name-r-mod"[^>]*maxLength="32"[^>]*value="Moderation"/,
    );
    const swatches = [
      ...html.matchAll(/<input type="radio"[^>]*aria-label="([^"]+)"[^>]*>/g),
    ].map((match) => [match[1], match[0].includes('checked=""')]);
    expect(swatches).toEqual([
      ["Bernstein", false],
      ["Blau", true],
      ["Grün", false],
      ["Rosa", false],
      ["Violett", false],
      ["Koralle", false],
    ]);
    const via = "über „Alle Mitglieder“";
    expect(switches(html)).toEqual([
      { label: "Server verwalten", note: "", checked: false, disabled: false },
      {
        label: "Mitglieder moderieren",
        note: "",
        checked: true,
        disabled: false,
      },
      { label: "Kanäle verwalten", note: "", checked: false, disabled: false },
      {
        label: "Nachrichten löschen",
        note: "",
        checked: true,
        disabled: false,
      },
      {
        label: "@everyone und @here",
        note: "",
        checked: true,
        disabled: false,
      },
      {
        label: "Nachrichten schreiben",
        note: via,
        checked: true,
        disabled: true,
      },
      { label: "Dateien senden", note: via, checked: true, disabled: true },
      {
        label: "Sprachkanälen beitreten",
        note: via,
        checked: true,
        disabled: true,
      },
      { label: "Live gehen", note: via, checked: true, disabled: true },
    ]);
  });

  it("opens on Alle Mitglieder without custom roles: no name, colour or delete", () => {
    const html = page(["manage_server"], {
      member_permissions: BASE,
      members: MEMBERS,
    });
    expect(roleRows(html).filter((row) => row.selected)).toEqual([
      { name: "Alle Mitglieder", count: 5, selected: true },
    ]);
    expect(html).toContain("<h3>Alle Mitglieder</h3>");
    expect(html).not.toContain("Rolle löschen");
    expect(html).not.toContain('type="radio"');
    expect(html).not.toContain('id="role-name-');
    const rows = switches(html);
    expect(rows.map((row) => row.checked)).toEqual([
      false,
      false,
      false,
      false,
      false,
      true,
      true,
      true,
      true,
    ]);
    expect(rows.every((row) => !row.disabled && row.note === "")).toBe(true);
  });

  it("shows the owner read-only, with every switch on", () => {
    const html = render(
      <RolesEditor
        server={detail(["manage_server"], withRoles)}
        initial="owner"
      />,
    );
    expect(html).toContain("<h3>Inhaber</h3>");
    expect(html).toContain(
      "Der Inhaber hat immer alle Rechte und sieht jeden Kanal.",
    );
    expect(html).not.toContain("Rolle löschen");
    const rows = switches(html);
    expect(rows).toHaveLength(9);
    expect(rows.every((row) => row.checked && row.disabled)).toBe(true);
  });

  it("falls back to the first custom role when the chosen one is gone", () => {
    const html = render(
      <RolesEditor
        server={detail(["manage_server"], withRoles)}
        initial="r-deleted"
      />,
    );
    expect(roleRows(html).filter((row) => row.selected)).toEqual([
      { name: "Moderation", count: 1, selected: true },
    ]);
    expect(html).toContain("<h3>Moderation</h3>");
  });
});

describe("members section", () => {
  it("shows role chips, the role picker and no Mitglied badge", () => {
    const html = page(["manage_server", "moderate_members"], withRoles);
    expect(html).toContain(
      "Hier vergibst du Rollen. Was eine Rolle darf, stellst du unter „Rollen“ ein.",
    );
    expect(html).not.toContain(">Mitglied<");
    const jonas = html.slice(html.indexOf(">Jonas<"), html.indexOf(">Mira<"));
    expect(
      [...jonas.matchAll(/<li class="role-chip">.*?<\/span>([^<]+)</g)].map(
        (match) => match[1],
      ),
    ).toEqual(["Moderation", "Stammgäste"]);
    expect(jonas).toMatch(/<button[^>]*aria-label="Rollen für Jonas"/);
    // The owner keeps the badge and gets no picker.
    const lena = html.slice(html.indexOf(">Lena<"), html.indexOf(">Jonas<"));
    expect(lena).toContain("Inhaber");
    expect(lena).not.toContain("Rollen für");
  });

  it("lists the custom roles in the picker, checked where held", () => {
    const html = page(["manage_server"], withRoles);
    const picker = html.match(
      /<div role="dialog" aria-label="Rollen für Mira">(.*?)<\/div><\/li>/,
    )?.[1];
    expect(picker).toBeDefined();
    const rows = [
      ...picker!.matchAll(
        /<label class="ui-choice-row role-picker__row"><input type="checkbox"([^>]*)>.*?<\/span>([^<]+)<\/span>/g,
      ),
    ].map((match) => [match[2], match[1].includes('checked=""')]);
    expect(rows).toEqual([
      ["Moderation", false],
      ["Stammgäste", true],
    ]);
    expect(picker).toContain("Rollen verwalten");
    expect(picker).not.toContain("Noch keine Rollen.");
  });

  it("says there are no roles yet when the server has none", () => {
    const html = page(["manage_server"], { members: MEMBERS });
    expect(html).toContain("Noch keine Rollen.");
    expect(html).toContain("Rollen verwalten");
  });

  it("shows chips to every member, but the picker only with manage_server", () => {
    const html = page(["moderate_members"], withRoles);
    expect(html).toContain('<li class="role-chip">');
    expect(html).not.toContain("Rollen für");
    expect(html).not.toContain("Hier vergibst du Rollen.");
  });
});

describe("server settings page in English", () => {
  afterEach(() => setLocale("de", { persist: false }));

  it("labels the sections, the roles and the permissions in English", () => {
    setLocale("en", { persist: false });
    const html = page(["manage_server"], withRoles);
    expect(marked(html)).toEqual(["General"]);
    expect(anchors(html)).toContain("Roles");
    expect(html).toContain("Server settings");
    expect(html).toContain("A role bundles permissions.");
    expect(html).toContain("via “All members”");
    expect(html).toContain("Roles for Jonas");
    expect(html).toContain("Go Live");
    expect(html).toContain("Danger zone");
    expect(html).not.toContain("Servereinstellungen");
  });
});

describe("server settings Verzeichnis", () => {
  const LANGUAGES = [
    { code: "de", name: "Deutsch", listings: 3 },
    { code: "en", name: "English", listings: 1 },
  ];
  const listing = (overrides: Partial<ListingSettings> = {}) => ({
    listed: true,
    description: "Gemütliche Abende mit Freunden.",
    language: "de",
    join_mode: "request" as const,
    delisted: false,
    listed_at: created_at,
    ...overrides,
  });
  const kai: JoinRequest = {
    user_id: "kai",
    name: "Kai",
    avatar_url: null,
    message: "Ich bin Jonas’ Bruder.",
    created_at,
  };
  const owner = (overrides: Partial<ServerDetail> = {}) =>
    page([], { role: "owner", members: MEMBERS, ...overrides });
  const text = (html: string) =>
    html
      .replace(/<[^>]+>/g, " ")
      .replace(/\s+/g, " ")
      .trim();

  beforeEach(() => {
    fixture.directory = { enabled: true, languages: LANGUAGES };
    fixture.listing = listing();
    fixture.requests = [kai];
    fixture.requestCount = undefined;
  });
  afterEach(() => {
    fixture.directory = undefined;
    setLocale("de", { persist: false });
  });

  it("is a section after Einladungen for the owner and moderate_members, while the directory is on", () => {
    const sections = (permissions: Permission[], owner = false, on = true) =>
      serverSettingsSections(
        { role: owner ? "owner" : "member", permissions },
        on,
      );
    expect(sections([], true)).toEqual([
      "allgemein",
      "rollen",
      "einladungen",
      "verzeichnis",
      "mitglieder",
      "sperren",
      "gefahrenzone",
    ]);
    expect(sections(["moderate_members"])).toEqual([
      "verzeichnis",
      "mitglieder",
      "sperren",
      "gefahrenzone",
    ]);
    // manage_server alone does not answer requests.
    expect(sections(["manage_server"])).not.toContain("verzeichnis");
    expect(sections(["send_messages"])).not.toContain("verzeichnis");
    expect(sections([], true, false)).not.toContain("verzeichnis");
  });

  it("shows in the anchor row only when the directory is on", () => {
    expect(anchors(owner())).toEqual([
      "Allgemein",
      "Rollen",
      "Einladungen",
      "Verzeichnis",
      "Mitglieder",
      "Sperren",
      "Gefahrenzone",
    ]);
    expect(anchors(page(["send_messages"]))).not.toContain("Verzeichnis");
    fixture.directory = { enabled: false, languages: [] };
    expect(anchors(owner())).not.toContain("Verzeichnis");
  });

  it("gives the owner the listing form, filled from the saved listing", () => {
    const html = owner();
    expect(html).toMatch(
      /Im Verzeichnis zeigen<\/span><input type="checkbox" role="switch"[^>]*checked=""/,
    );
    expect(html).toContain(">Gemütliche Abende mit Freunden.</textarea>");
    expect(html).toContain("31 / 300 Zeichen");
    expect(html).toMatch(/<option value="de" lang="de" selected="">Deutsch/);
    expect(html).toContain(">English</option>");
    const radios = [...html.matchAll(/<input type="radio"[^>]*>/g)].map(
      ([input]) => [
        input.match(/value="([^"]+)"/)?.[1],
        input.includes('checked=""'),
      ],
    );
    expect(radios).toEqual([
      ["open", false],
      ["request", true],
    ]);
    expect(html).toContain("Jeder kann direkt beitreten.");
    expect(html).toContain(
      "Du oder wer Mitglieder moderieren darf, nimmt Anfragen an.",
    );
    expect(html).not.toContain(
      "Der Betreiber hat diesen Server aus dem Verzeichnis genommen.",
    );
  });

  it("previews the row as Entdecken shows it, without working buttons", () => {
    const html = owner();
    const preview = html.slice(html.indexOf(">Vorschau<"));
    expect(preview).toMatch(/<ul class="discover-list" inert="">/);
    expect(text(preview)).toContain(
      "Wo Wohnzimmer Gemütliche Abende mit Freunden. Deutsch · 5 Mitglieder Beitritt anfragen",
    );
    // A preview has no "…" menu.
    expect(preview).not.toContain("Mehr zu Wohnzimmer");
  });

  it("starts an unsaved listing off, without a language", () => {
    fixture.listing = listing({
      listed: false,
      description: "",
      language: null,
      listed_at: null,
    });
    const html = owner();
    expect(html).toMatch(
      /<input type="checkbox" role="switch"(?![^>]*checked)/,
    );
    expect(html).toMatch(
      /<option value="" disabled="" selected="">Sprache wählen/,
    );
    expect(html).toContain("0 / 300 Zeichen");
  });

  it("tells the owner when the operator took the server out", () => {
    fixture.listing = listing({ delisted: true });
    expect(owner()).toContain(
      "Der Betreiber hat diesen Server aus dem Verzeichnis genommen.",
    );
  });

  it("lists the requests with Annehmen and Ablehnen", () => {
    const html = owner();
    expect(html).toContain(">Beitrittsanfragen</h3>");
    const queue = text(html.slice(html.indexOf(">Beitrittsanfragen<")));
    expect(queue).toMatch(
      /^>Beitrittsanfragen K Kai · \S.* Ich bin Jonas’ Bruder\. Annehmen Ablehnen/,
    );
    expect(html).toContain(`<time dateTime="${created_at}">`);
  });

  it("says when no request waits", () => {
    fixture.requests = [];
    expect(owner()).toContain("Keine offenen Anfragen.");
  });

  it("gives moderate_members the requests but not the listing", () => {
    const html = page(["moderate_members"], { members: MEMBERS });
    expect(html).toContain(">Beitrittsanfragen</h3>");
    expect(html).not.toContain("Im Verzeichnis zeigen");
    expect(html).not.toContain(">Vorschau<");
  });

  it("counts open requests in the anchor row", () => {
    fixture.requestCount = 3;
    const html = owner();
    expect(html).toMatch(
      /Verzeichnis<span class="ui-badge ui-badge--count"><span aria-hidden="true">3<\/span><span class="sr-only">3 offene Anfragen<\/span><\/span><\/a>/,
    );
    fixture.requestCount = 0;
    expect(owner()).not.toContain("offene Anfrage");
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    fixture.requestCount = 1;
    fixture.listing = listing({ delisted: true });
    const body = text(owner());
    expect(body).toContain("Invites Directory 1 1 open request Members");
    expect(body).toContain("Show in the directory");
    expect(body).toContain("31 / 300 characters");
    expect(body).toContain("Joining Open Anyone can join straight away.");
    expect(body).toContain(
      "The operator has removed this server from the directory.",
    );
    expect(body).toContain("Deutsch · 5 members Request to join");
    expect(body).toContain("Join requests");
    expect(body).toContain("Accept Decline");
  });
});
