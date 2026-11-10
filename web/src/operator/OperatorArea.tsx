// The operator area's pages: one lazy chunk (`OperatorArea` in routes.tsx).
// Each page sits in `OperatorPageFrame`.

import type { OperatorPage } from "./areas.ts";
import { OperatorPageFrame } from "./OperatorPageFrame.tsx";
import { ReportsPage } from "./pages/ReportsPage.tsx";
import { ServersPage } from "./pages/ServersPage.tsx";
import "./operator.css";

export function OperatorArea({ page, q }: { page: OperatorPage; q?: string }) {
  switch (page) {
    case "server":
      // A new `?q=` ("Zum Server") starts the page over with that search.
      return <ServersPage key={q ?? ""} q={q} />;
    case "meldungen":
      return <ReportsPage />;
    default:
      return <OperatorPageFrame page={page} />;
  }
}
