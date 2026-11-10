import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../../i18n/locale.ts";
import type { Usage } from "../types.ts";
import { usageGroups } from "../usage.ts";

const fixture = vi.hoisted(() => ({
  result: {} as {
    data?: unknown;
    isError?: boolean;
    isFetching?: boolean;
  },
}));

vi.mock("../queries.ts", () => ({
  useOperatorUsage: () => ({ refetch: vi.fn(), ...fixture.result }),
}));

import { UsagePage } from "./UsagePage.tsx";

// Every field distinct, so each number can be found where it belongs.
// Sizes come with a no-break space before the unit; the tests read spaces.
const usage: Usage = {
  generated_at: "2026-11-10T09:30:00Z",
  registration: "invitation",
  accounts: {
    total: 1234,
    pending: 3,
    disabled: 2,
    new_7d: 11,
    new_30d: 40,
    with_live_session: 87,
  },
  servers: { total: 56, blocked: 1, delisted: 4 },
  messages: { server_24h: 2500, server_7d: 17_000, dm_7d: 900 },
  storage: {
    attachment_bytes: 6_800_000_000,
    attachment_count: 4321,
    uploaded_today_bytes: 340_000_000,
  },
  now: { sockets_this_process: 61, in_voice: 9, live: 2 },
};

beforeEach(() => {
  fixture.result = {};
});
afterEach(() => setLocale("de", { persist: false }));

const render = () => renderToStaticMarkup(<UsagePage />);
const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();

describe("Auslastung", () => {
  it("shows every number of the response once, and nothing else", () => {
    const groups = usageGroups(usage);
    expect(groups.map((group) => group.title)).toEqual([
      "Konten",
      "Server",
      "Nachrichten",
      "Speicher",
      "Jetzt",
    ]);
    expect(
      groups.flatMap((group) =>
        group.stats.map((stat) => [stat.label, stat.value.replace(/\s/g, " ")]),
      ),
    ).toEqual([
      ["Konten insgesamt", "1.234"],
      ["Neu in den letzten 7 Tagen", "11"],
      ["Neu in den letzten 30 Tagen", "40"],
      ["Konten mit einer nicht abgelaufenen Sitzung", "87"],
      ["Registriert, Adresse noch nicht bestätigt", "3"],
      ["Gesperrte Konten", "2"],
      ["Server insgesamt", "56"],
      ["Gesperrte Server", "1"],
      ["Aus dem Verzeichnis genommen", "4"],
      ["In Serverkanälen, letzte 24 Stunden", "2.500"],
      ["In Serverkanälen, letzte 7 Tage", "17.000"],
      ["In Direktnachrichten, letzte 7 Tage", "900"],
      ["Anhänge gesendeter Nachrichten", "6,8 GB"],
      ["Anzahl dieser Anhänge", "4.321"],
      ["Heute hochgeladen (UTC)", "340 MB"],
      ["Sockets dieses API-Prozesses", "61"],
      ["Personen in Sprachkanälen", "9"],
      ["Davon live", "2"],
    ]);
    // 6 + 3 + 3 + 3 + 3: every number field of `Usage`.
    const numbers = Object.values(usage).flatMap((value) =>
      typeof value === "object" ? Object.keys(value) : [],
    );
    expect(groups.flatMap((group) => group.stats)).toHaveLength(numbers.length);
  });

  it("draws the groups as labelled numbers with the mode and the time", () => {
    fixture.result = { data: usage };
    const html = render();
    const body = text(html);
    expect(html).toContain(">Auslastung</h1>");
    expect(body).toContain("Registrierung: nur mit Einladung · Stand:");
    expect(html).toContain('<h2 id="operator-usage-now">Jetzt</h2>');
    expect(html).toContain("<dt>Sockets dieses API-Prozesses</dt><dd>61</dd>");
    expect(html.match(/<dd>/g)).toHaveLength(18);
    expect(body).toContain("Prometheus und Grafana");
    expect(html).not.toMatch(/<svg|<canvas/);
  });

  it("has a load error", () => {
    fixture.result = { isError: true };
    const html = render();
    expect(html).toContain('role="alert"');
    expect(html).toContain("Die Auslastung konnte nicht geladen werden.");
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    fixture.result = { data: { ...usage, registration: "open" } };
    const body = text(render());
    expect(body).toContain("Usage");
    expect(body).toContain("Registration: open to everyone · As of");
    expect(body).toContain("Attachments of sent messages 6.8 GB");
    expect(body).toContain("Accounts in total 1,234");
    expect(body).toContain("Sockets of this API process 61");
  });
});
