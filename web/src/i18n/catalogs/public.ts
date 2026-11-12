import { defineCatalog } from "../catalog.ts";

// The public pages an instance can publish (src/public): the landing page,
// the legal pages and the feedback form. They are static HTML built at build time in both
// languages, so every text here ends up in a file, never in the app.
// `public` is reserved in strict code, so the binding has another name and
// the export carries the file's.
const publicPages = defineCatalog(
  "public",
  {
    "meta.landingTitle":
      "Gelabber – Voice, Chat und Live-Bildschirme für deine Gruppe",
    "meta.pageTitle": "{page} – Gelabber",
    "meta.legalDescription": "{page} dieser Gelabber-Instanz.",

    "landing.nav.label": "Hauptnavigation",
    "landing.nav.selfHost": "Selbst hosten",
    "landing.nav.source": "Quellcode",
    "landing.nav.signIn": "Anmelden",
    "landing.cta.invite": "Einladung anfragen",
    "landing.invite.subject": "Einladung für Gelabber",
    // A non-breaking hyphen (U+2011): the heading does not break after "Live-".
    "landing.hero.title": "Voice, Chat und Live‑Bildschirme für deine Gruppe.",
    "landing.hero.lead":
      "Wer live geht, zeigt allen im Sprachkanal einen Bildschirm oder ein Fenster. Mehrere können gleichzeitig live sein, und du wählst, wessen Bild du schaust.",
    "landing.hero.note":
      "Die gehostete Instanz ist eine kostenlose Beta, nur mit Einladung. Auf Deutsch und Englisch.",
    "landing.screenshotAlt":
      "Gelabber im Browser: ein Server mit seinen Kanälen und dem Chat, während jemand im Sprachkanal live ist.",
    "landing.features.lives.title": "Mehrere Lives gleichzeitig",
    "landing.features.lives.body":
      "Jede Person, die live geht, bekommt ein eigenes Bild. Schau eins groß, alle nebeneinander oder eins über das ganze Fenster.",
    "landing.features.open.title": "Open Source",
    "landing.features.open.body":
      "Gelabber steht unter der MIT-Lizenz. Die Instanz, die wir betreiben, läuft mit demselben Code, den du selbst betreiben kannst.",
    "landing.features.private.title": "Privat von Anfang an",
    "landing.features.private.body":
      "Einer Community trittst du über einen Einladungslink bei. Nichts davon ist öffentlich, solange ihr Inhaber es nicht so entscheidet.",
    "landing.selfHost.title": "Auf deinem eigenen Server",
    "landing.selfHost.body":
      "Drei Befehle starten Gelabber mit Docker Compose, sobald die Passwörter in .env gesetzt sind. Die Betriebsanleitung behandelt TLS, TURN, Backups und Updates.",
    "landing.selfHost.guide": "Betriebsanleitung lesen",
    "landing.managed.title": "Oder eine eigene Instanz, von uns betrieben",
    "landing.managed.body":
      "Deine eigene Adresse und deine eigenen Daten, ohne dich um einen Server zu kümmern. Erzähl uns von deiner Gruppe, und wir melden uns.",
    "landing.managed.write": "Schreib an {email}",
    "landing.footer.label": "Rechtliches und Sprache",
    "landing.footer.licence": "Gelabber · MIT-Lizenz",

    "legal.imprint": "Impressum",
    "legal.privacy": "Datenschutz",
    "legal.terms": "Nutzungsbedingungen",
    "legal.navLabel": "Rechtliches",
    "legal.signIn": "Anmelden",
    "legal.imprintLead": "Angaben nach § 5 DDG.",
    "legal.operator": "Betreiber",
    "legal.contact": "Kontakt",
    "legal.email": "E-Mail",
    "legal.phone": "Telefon",
    "legal.responsible": "Verantwortlich für den Inhalt",
    "legal.report": "Inhalte melden",
    // The report action's label in the app. It moves to the discovery
    // catalog's key for that action once that package is in the base (#522).
    "legal.reportAction": "Melden",
    "legal.reportText":
      "Rechtswidrige Inhalte meldest du über „{action}“ am Server oder an <link>{email}</link>.",
    "legal.onlyOtherLanguage": "Dieser Text liegt nur auf Englisch vor.",

    "meta.feedbackTitle": "Feedback zu Gelabber",
    "meta.feedbackDescription":
      "Eine Idee, ein Problem oder Lob an die Betreiber dieser Gelabber-Instanz.",
    "feedback.link": "Feedback",
    "feedback.title": "Feedback zu Gelabber",
    "feedback.lead":
      "Kein Konto, aber etwas zu sagen? Schreib den Betreibern dieser Instanz eine Idee, ein Problem oder Lob. Wir schicken dir einen Bestätigungslink; erst damit kommt dein Feedback an.",
    "feedback.kind.legend": "Worum geht es?",
    "feedback.kind.idea": "Idee",
    "feedback.kind.problem": "Problem",
    "feedback.kind.praise": "Lob",
    "feedback.text.label": "Deine Nachricht",
    "feedback.text.hint": "Höchstens 4.000 Zeichen.",
    "feedback.email.label": "Deine E-Mail-Adresse",
    "feedback.email.hint":
      "Nur für die Bestätigung und eine Antwort. Sie wird nirgends gezeigt.",
    "feedback.honeypot": "Website (leer lassen)",
    "feedback.privacy": "Mehr dazu unter <link>Datenschutz</link>.",
    "feedback.submit": "Feedback senden",
    "feedback.sent.title": "Fast geschafft.",
    "feedback.sent.body":
      "Wir haben dir eine Mail geschickt. Öffne den Link darin innerhalb von 24 Stunden, dann kommt dein Feedback bei den Betreibern an. Keine Mail da? Schau im Spam-Ordner nach oder <link>schick das Formular noch einmal</link>.",
    "feedback.thanks.title": "Danke!",
    "feedback.thanks.body":
      "Dein Feedback ist bei den Betreibern angekommen. Wenn sie antworten, bekommst du eine Mail.",
    "feedback.expired.title": "Link abgelaufen.",
    "feedback.expired.body":
      "Dieser Link ist ungültig oder älter als 24 Stunden, und dein Feedback wurde nicht verschickt. <link>Schick es bitte noch einmal</link>.",
    "feedback.error.title": "Das hat nicht geklappt.",
    "feedback.error.body":
      "Dein Feedback wurde nicht verschickt. Vielleicht kamen von deinem Anschluss gerade zu viele Nachrichten, oder eine Angabe fehlte. <link>Versuch es später noch einmal</link>.",
  },
  {
    "meta.landingTitle":
      "Gelabber – voice, chat and live screens for your group",
    "meta.pageTitle": "{page} – Gelabber",
    "meta.legalDescription": "{page} of this Gelabber instance.",

    "landing.nav.label": "Main navigation",
    "landing.nav.selfHost": "Host it yourself",
    "landing.nav.source": "Source code",
    "landing.nav.signIn": "Sign in",
    "landing.cta.invite": "Request an invite",
    "landing.invite.subject": "Gelabber invite",
    "landing.hero.title": "Voice, chat and live screens for your group.",
    "landing.hero.lead":
      "Whoever goes live shows everyone in the voice channel a screen or a window. Several people can be live at the same time, and you choose whose picture you watch.",
    "landing.hero.note":
      "The hosted instance is in a free, invitation-only beta. In German and English.",
    "landing.screenshotAlt":
      "Gelabber in the browser: a server with its channels and chat while someone is live in a voice channel.",
    "landing.features.lives.title": "Several Lives at once",
    "landing.features.lives.body":
      "Each person who goes live gets a picture of their own. Watch one large, all of them side by side, or one across the whole window.",
    "landing.features.open.title": "Open source",
    "landing.features.open.body":
      "Gelabber is published under the MIT licence. The instance we host runs the same code you can run yourself.",
    "landing.features.private.title": "Private by default",
    "landing.features.private.body":
      "A community is joined through an invite link. Nothing about it is public unless its owner decides so.",
    "landing.selfHost.title": "Run it on your own server",
    "landing.selfHost.body":
      "Three commands start Gelabber with Docker Compose once the passwords in .env are set. The operator guide covers TLS, TURN, backups and updates.",
    "landing.selfHost.guide": "Read the operator guide",
    "landing.managed.title": "Or get an instance of your own, run by us",
    "landing.managed.body":
      "Your own address and your own data, without looking after a server. Tell us about your group and we will get back to you.",
    "landing.managed.write": "Write to {email}",
    "landing.footer.label": "Legal and language",
    "landing.footer.licence": "Gelabber · MIT licence",

    "legal.imprint": "Imprint",
    "legal.privacy": "Privacy",
    "legal.terms": "Terms",
    "legal.navLabel": "Legal",
    "legal.signIn": "Sign in",
    "legal.imprintLead": "Information according to § 5 DDG.",
    "legal.operator": "Operator",
    "legal.contact": "Contact",
    "legal.email": "Email",
    "legal.phone": "Phone",
    "legal.responsible": "Responsible for the content",
    "legal.report": "Report content",
    "legal.reportAction": "Report",
    "legal.reportText":
      "Report unlawful content with “{action}” on the server or to <link>{email}</link>.",
    "legal.onlyOtherLanguage": "This text is only available in German.",

    "meta.feedbackTitle": "Feedback on Gelabber",
    "meta.feedbackDescription":
      "An idea, a problem or praise for the operators of this Gelabber instance.",
    "feedback.link": "Feedback",
    "feedback.title": "Feedback on Gelabber",
    "feedback.lead":
      "No account, but something to say? Send the operators of this instance an idea, a problem or praise. We'll send you a confirmation link; your feedback only arrives once you open it.",
    "feedback.kind.legend": "What is it about?",
    "feedback.kind.idea": "Idea",
    "feedback.kind.problem": "Problem",
    "feedback.kind.praise": "Praise",
    "feedback.text.label": "Your message",
    "feedback.text.hint": "At most 4,000 characters.",
    "feedback.email.label": "Your email address",
    "feedback.email.hint":
      "Only for the confirmation and a reply. It is not shown anywhere.",
    "feedback.honeypot": "Website (leave empty)",
    "feedback.privacy": "More under <link>Privacy</link>.",
    "feedback.submit": "Send feedback",
    "feedback.sent.title": "Almost there.",
    "feedback.sent.body":
      "We've sent you an email. Open the link in it within 24 hours and your feedback reaches the operators. No email? Check your spam folder or <link>send the form again</link>.",
    "feedback.thanks.title": "Thank you!",
    "feedback.thanks.body":
      "Your feedback has reached the operators. If they reply, you'll get an email.",
    "feedback.expired.title": "Link expired.",
    "feedback.expired.body":
      "This link is invalid or older than 24 hours, and your feedback was not sent. <link>Please send it again</link>.",
    "feedback.error.title": "That didn't work.",
    "feedback.error.body":
      "Your feedback was not sent. Perhaps too many messages came from your connection just now, or something was missing. <link>Please try again later</link>.",
  },
);

export { publicPages as public };
