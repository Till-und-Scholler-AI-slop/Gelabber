import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({
  overview: undefined as
    { open_reports: number; open_feedback?: number } | undefined,
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  // A plain anchor that says where it leads, without the router.
  Link: ({
    to,
    params,
    ...props
  }: {
    to: string;
    params?: Record<string, string>;
  } & Record<string, unknown>) => (
    <a
      data-to={to.replace(
        /\$(\w+)/g,
        (_, name: string) => params?.[name] ?? "",
      )}
      {...props}
    />
  ),
}));
vi.mock("../components/UserPanel.tsx", () => ({
  UserPanel: () => <footer data-user-panel="" />,
}));
vi.mock("./queries.ts", () => ({
  useOperatorOverview: () => ({ data: fixture.overview }),
}));

import { OperatorSidebar } from "./OperatorSidebar.tsx";

beforeEach(() => {
  fixture.overview = undefined;
});
afterEach(() => setLocale("de", { persist: false }));

const render = () => renderToStaticMarkup(<OperatorSidebar />);
const rows = (html: string) =>
  [...html.matchAll(/data-to="([^"]+)"[^>]*><span>([^<]+)<\/span>/g)].map(
    ([, to, label]) => [to, label],
  );

describe("operator sidebar", () => {
  it("lists the pages of this instance as in the reference", () => {
    const html = render();
    expect(html).toContain('<div class="sidebar-brand">Betrieb</div>');
    expect(html).toContain(">Diese Instanz</h2>");
    expect(rows(html)).toEqual([
      ["/betrieb/einladungen", "Beta-Einladungen"],
      ["/betrieb/konten", "Konten"],
      ["/betrieb/server", "Server"],
      ["/betrieb/meldungen", "Meldungen"],
      ["/betrieb/feedback", "Feedback"],
      ["/betrieb/auslastung", "Auslastung"],
    ]);
    // The user panel stays at the bottom, as in every sidebar.
    expect(html).toMatch(/<footer data-user-panel=""><\/footer><\/aside>$/);
  });

  it("counts open reports on Meldungen", () => {
    fixture.overview = { open_reports: 2 };
    const html = render();
    expect(html).toMatch(
      /data-to="\/betrieb\/meldungen"[^>]*><span>Meldungen<\/span><span class="ui-badge ui-badge--count operator-nav-count" aria-hidden="true">2<\/span><span class="sr-only">2 offene Meldungen<\/span>/,
    );
    expect(html.match(/ui-badge--count/g)).toHaveLength(1);
  });

  it("counts open feedback on Feedback, as on Meldungen", () => {
    fixture.overview = { open_reports: 0, open_feedback: 1 };
    let html = render();
    expect(html).toMatch(
      /data-to="\/betrieb\/feedback"[^>]*><span>Feedback<\/span><span class="ui-badge ui-badge--count operator-nav-count" aria-hidden="true">1<\/span><span class="sr-only">1 offener Eintrag im Feedback<\/span>/,
    );
    expect(html.match(/ui-badge--count/g)).toHaveLength(1);
    fixture.overview = { open_reports: 2, open_feedback: 120 };
    html = render();
    expect(html.match(/ui-badge--count/g)).toHaveLength(2);
    expect(html).toContain("120 offene Einträge im Feedback");
    expect(html).toContain('aria-hidden="true">99+</span>');
    setLocale("en", { persist: false });
    expect(render()).toContain("120 open feedback entries");
  });

  it("shows no count without open reports, and caps it at 99+", () => {
    fixture.overview = { open_reports: 0 };
    expect(render()).not.toContain("ui-badge");
    fixture.overview = { open_reports: 120 };
    expect(render()).toContain('aria-hidden="true">99+</span>');
  });

  it("speaks English after a switch", () => {
    setLocale("en", { persist: false });
    fixture.overview = { open_reports: 1 };
    const html = render();
    expect(html).toContain('<div class="sidebar-brand">Operations</div>');
    expect(rows(html).map(([, label]) => label)).toEqual([
      "Beta invitations",
      "Accounts",
      "Servers",
      "Reports",
      "Feedback",
      "Usage",
    ]);
    expect(html).toContain('<span class="sr-only">1 open report</span>');
  });
});
