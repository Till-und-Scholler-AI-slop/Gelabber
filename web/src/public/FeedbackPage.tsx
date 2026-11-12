// The public feedback form (#542) and the four pages the API's answer leads
// to: sent, thanks, expired and error. Static HTML without script like the
// other public pages; the form posts to /api/public/feedback, which answers
// with a 303 to one of the four. The container publishes them only with the
// landing page, account mail and operators (PAGE_FEEDBACK).

import { richPieces, translateIn } from "../i18n/catalog.ts";
import { public as publicPages } from "../i18n/catalogs/public.ts";
import type { Locale } from "../i18n/locale.ts";
import { KEY } from "./keys.ts";
import { LegalTop } from "./LegalPage.tsx";
import { PublicFrame } from "./PublicFrame.tsx";
import { If } from "./Template.tsx";
import { textsIn } from "./text.ts";
import {
  feedbackUrl,
  legalUrl,
  otherLocale,
  type FeedbackState,
} from "./urls.ts";

const KINDS = ["idea", "problem", "praise"] as const;

type AnswerState = Exclude<FeedbackState, "form">;

/** A text with one `<link>…</link>`, the link going to `href`. */
function Linked({
  locale,
  k,
  href,
}: {
  locale: Locale;
  k:
    | "feedback.privacy"
    | "feedback.sent.body"
    | "feedback.expired.body"
    | "feedback.error.body";
  href: string;
}) {
  return (
    <>
      {richPieces(translateIn(locale, publicPages, k)).map((piece, index) =>
        typeof piece === "string" ? (
          piece
        ) : (
          <a key={index} href={href}>
            {piece.text}
          </a>
        ),
      )}
    </>
  );
}

function Form({ locale }: { locale: Locale }) {
  const text = textsIn(locale);
  return (
    <form method="post" action="/api/public/feedback" acceptCharset="utf-8">
      <input type="hidden" name="lang" value={locale} />
      <fieldset className="feedback-kinds">
        <legend>{text("feedback.kind.legend")}</legend>
        {KINDS.map((kind) => (
          <label key={kind}>
            <input
              type="radio"
              name="kind"
              value={kind}
              defaultChecked={kind === "idea"}
            />{" "}
            {text(`feedback.kind.${kind}`)}
          </label>
        ))}
      </fieldset>
      <div className="feedback-field">
        <label htmlFor="feedback-text">{text("feedback.text.label")}</label>
        <textarea
          id="feedback-text"
          name="text"
          required
          maxLength={4000}
          rows={8}
          aria-describedby="feedback-text-hint"
        />
        <p id="feedback-text-hint" className="feedback-hint">
          {text("feedback.text.hint")}
        </p>
      </div>
      <div className="feedback-field">
        <label htmlFor="feedback-email">{text("feedback.email.label")}</label>
        <input
          id="feedback-email"
          name="email"
          type="email"
          required
          maxLength={254}
          autoComplete="email"
          aria-describedby="feedback-email-hint"
        />
        <p id="feedback-email-hint" className="feedback-hint">
          {text("feedback.email.hint")}
        </p>
      </div>
      {/* For bots: off-screen in public.css, not display:none, which some
          of them skip. The API drops any post that fills it in. */}
      <div className="feedback-hp" aria-hidden="true">
        <label htmlFor="feedback-website">{text("feedback.honeypot")}</label>
        <input
          id="feedback-website"
          name="website"
          type="text"
          tabIndex={-1}
          autoComplete="off"
        />
      </div>
      <If when={KEY.privacy}>
        <p className="feedback-hint">
          <Linked
            locale={locale}
            k="feedback.privacy"
            href={legalUrl(locale, "privacy")}
          />
        </p>
      </If>
      <button type="submit" className="public-button">
        {text("feedback.submit")}
      </button>
    </form>
  );
}

function Answer({ locale, state }: { locale: Locale; state: AnswerState }) {
  const text = textsIn(locale);
  return (
    <>
      <h1>{text(`feedback.${state}.title`)}</h1>
      <p className="feedback-lead">
        {state === "thanks" ? (
          text("feedback.thanks.body")
        ) : (
          <Linked
            locale={locale}
            k={`feedback.${state}.body`}
            href={feedbackUrl(locale)}
          />
        )}
      </p>
    </>
  );
}

export function FeedbackPage({
  locale,
  state,
  stylesheet,
}: {
  locale: Locale;
  state: FeedbackState;
  stylesheet: string;
}) {
  const text = textsIn(locale);
  return (
    <PublicFrame
      locale={locale}
      title={text("meta.feedbackTitle")}
      description={text("meta.feedbackDescription")}
      stylesheet={stylesheet}
      paths={{ de: feedbackUrl("de", state), en: feedbackUrl("en", state) }}
      noindex
    >
      <LegalTop
        locale={locale}
        otherPath={feedbackUrl(otherLocale(locale), state)}
      />
      <main
        className={`public-container feedback-form${state === "form" ? "" : " feedback-answer"}`}
      >
        {state === "form" ? (
          <>
            <h1>{text("feedback.title")}</h1>
            <p className="feedback-lead">{text("feedback.lead")}</p>
            <Form locale={locale} />
          </>
        ) : (
          <Answer locale={locale} state={state} />
        )}
      </main>
    </PublicFrame>
  );
}
