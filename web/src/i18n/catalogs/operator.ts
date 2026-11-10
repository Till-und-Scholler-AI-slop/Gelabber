import { defineCatalog } from "../catalog.ts";

// The instance operators' area "Betrieb" (web/src/operator, v0.10).
export const operator = defineCatalog(
  "operator",
  {
    title: "Betrieb",
    "sidebar.label": "Betrieb",
    "nav.label": "Bereiche",
    "nav.instance": "Diese Instanz",
    "nav.reportsLabel": {
      one: "{count} offene Meldung",
      other: "{count} offene Meldungen",
    },
    "page.einladungen.title": "Beta-Einladungen",
    "page.einladungen.description":
      "Einladungen zur Registrierung. Nur für Betreiber sichtbar.",
    "page.konten.title": "Konten",
    "page.konten.description":
      "Alle Konten auf dieser Instanz. Nur für Betreiber sichtbar.",
    "page.server.title": "Server",
    "page.server.description":
      "Alle Server auf dieser Instanz. Nur für Betreiber sichtbar.",
    "page.meldungen.title": "Meldungen",
    "page.meldungen.description":
      "Gemeldete Server. Nur für Betreiber sichtbar.",
    "page.auslastung.title": "Auslastung",
    "page.auslastung.description":
      "Was diese Instanz gerade trägt. Nur für Betreiber sichtbar.",
    lazyPartFailed: "Der Bereich Betrieb konnte nicht geladen werden.",
  },
  {
    title: "Operations",
    "sidebar.label": "Operations",
    "nav.label": "Sections",
    "nav.instance": "This instance",
    "nav.reportsLabel": {
      one: "{count} open report",
      other: "{count} open reports",
    },
    "page.einladungen.title": "Beta invitations",
    "page.einladungen.description":
      "Invitations to register. Only operators can see this.",
    "page.konten.title": "Accounts",
    "page.konten.description":
      "Every account on this instance. Only operators can see this.",
    "page.server.title": "Servers",
    "page.server.description":
      "Every server on this instance. Only operators can see this.",
    "page.meldungen.title": "Reports",
    "page.meldungen.description":
      "Servers people have reported. Only operators can see this.",
    "page.auslastung.title": "Usage",
    "page.auslastung.description":
      "What this instance is carrying right now. Only operators can see this.",
    lazyPartFailed: "The operations area couldn’t be loaded.",
  },
);
