// The Impressum's own sections (docs/design/v0.10/hosted-imprint.png),
// filled with the operator's values. The container publishes it only when
// name, street, city and email are all set, so no placeholder is ever shown;
// the reference's note about yellow placeholders is a design note.

import { richPieces, translateIn } from "../i18n/catalog.ts";
import { public as publicPages } from "../i18n/catalogs/public.ts";
import type { Locale } from "../i18n/locale.ts";
import { KEY, value } from "./keys.ts";
import { If } from "./Template.tsx";
import { textsIn } from "./text.ts";

export function ImprintBody({ locale }: { locale: Locale }) {
  const text = textsIn(locale);
  const report = richPieces(
    translateIn(locale, publicPages, "legal.reportText", {
      action: text("legal.reportAction"),
      email: value(KEY.reportEmail),
    }),
  );
  return (
    <>
      <p className="legal-lead">{text("legal.imprintLead")}</p>
      <section>
        <h2>{text("legal.operator")}</h2>
        <p>
          {value(KEY.operatorName)}
          <br />
          {value(KEY.operatorStreet)}
          <br />
          {value(KEY.operatorCity)}
        </p>
      </section>
      <section>
        <h2>{text("legal.contact")}</h2>
        <p>
          {text("legal.email")}:{" "}
          <a href={`mailto:${value(KEY.operatorEmail)}`}>
            {value(KEY.operatorEmail)}
          </a>
        </p>
        <If when={KEY.operatorPhone}>
          <p>
            {text("legal.phone")}: {value(KEY.operatorPhone)}
          </p>
        </If>
      </section>
      <section>
        <h2>{text("legal.responsible")}</h2>
        <p>{value(KEY.contentResponsible)}</p>
      </section>
      <section>
        <h2>{text("legal.report")}</h2>
        <p>
          {report.map((piece, index) =>
            typeof piece === "string" ? (
              piece
            ) : (
              <a key={index} href={`mailto:${value(KEY.reportEmail)}`}>
                {piece.text}
              </a>
            ),
          )}
        </p>
      </section>
    </>
  );
}
