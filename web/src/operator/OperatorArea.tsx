// The operator area's pages: one lazy chunk (`OperatorArea` in routes.tsx).
// Each page sits in `OperatorPageFrame`.

import type { OperatorPage } from "./areas.ts";
import { AccountsPage } from "./pages/AccountsPage.tsx";
import { FeedbackPage } from "./pages/FeedbackPage.tsx";
import { InvitationsPage } from "./pages/InvitationsPage.tsx";
import { ReportsPage } from "./pages/ReportsPage.tsx";
import { ServersPage } from "./pages/ServersPage.tsx";
import { UsagePage } from "./pages/UsagePage.tsx";
import "./operator.css";

export function OperatorArea({
  page,
  q,
  entry,
}: {
  page: OperatorPage;
  q?: string;
  /** `?entry=`: the feedback entry an operator mail links to. */
  entry?: string;
}) {
  switch (page) {
    case "server":
      // A new `?q=` ("Zum Server") starts the page over with that search.
      return <ServersPage key={q ?? ""} q={q} />;
    case "meldungen":
      return <ReportsPage />;
    case "konten":
      return <AccountsPage />;
    case "einladungen":
      return <InvitationsPage />;
    case "feedback":
      return <FeedbackPage entry={entry} />;
    case "auslastung":
      return <UsagePage />;
  }
}
