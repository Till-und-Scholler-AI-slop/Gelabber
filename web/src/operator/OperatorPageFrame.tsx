// The frame of every operator page: the shell's empty header line, then a
// title and a one-line description above the page's content.

import type { ReactNode } from "react";

import { useT } from "../i18n/catalog.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import type { OperatorPage } from "./areas.ts";
import "./operator.css";

export function OperatorPageFrame({
  page,
  children,
}: {
  page: OperatorPage;
  children?: ReactNode;
}) {
  const t = useT(operator);
  return (
    <div className="operator-screen">
      {/* The shell's header line, empty here as in the reference. Not a
          conversation header, so a phone keeps its top bar in the flow. */}
      <div className="operator-header" aria-hidden="true" />
      <div className="operator-scroll">
        <section className="operator-page" aria-labelledby="operator-title">
          <header className="operator-page-head">
            <h1 id="operator-title">{t(`page.${page}.title`)}</h1>
            <p>{t(`page.${page}.description`)}</p>
          </header>
          {children}
        </section>
      </div>
    </div>
  );
}
