// Signing in and the pages around it (src/auth, the auth pages, the invite
// page, "not found" and the error page), plus the copy for API error and
// field codes that every form shows: `error.<code>` and `field*.<…>`.
import { defineCatalog } from "../catalog.ts";

export const auth = defineCatalog(
  "auth",
  {
    // Form-level API error codes (auth/rules.ts `errorMessage`).
    "error.invalidCredentials": "E-Mail oder Passwort ist falsch.",
    "error.themeConflict":
      "Auf einem anderen Gerät geändert. Bitte erneut versuchen.",
    "error.emailTaken": "Diese E-Mail-Adresse ist schon registriert.",
    "error.emailUnverified":
      "Bitte bestätige zuerst deine E-Mail-Adresse. Wir haben dir gerade einen neuen Link geschickt.",
    "error.tokenInvalid":
      "Dieser Link ist abgelaufen oder wurde schon benutzt.",
    "error.unauthenticated": "Bitte melde dich erneut an.",
    "error.csrfInvalid":
      "Sicherheits-Token abgelaufen. Bitte noch einmal versuchen.",
    "error.insecureTransport":
      "Anmelden geht nur über eine verschlüsselte Verbindung. Öffne Gelabber mit https:// (oder auf diesem Rechner über http://localhost).",
    "error.forbidden": "Dafür fehlt dir die Berechtigung.",
    "error.notFound": "Das gibt es nicht (mehr).",
    "error.inviteInvalid": "Diese Einladung ist abgelaufen oder aufgebraucht.",
    "error.banned": "Du bist auf diesem Server gesperrt.",
    "error.rateLimited":
      "Zu viele Anfragen. Bitte kurz warten und noch einmal versuchen.",
    "error.quotaExceeded": "Tageslimit für Uploads erreicht.",
    "error.upstreamUnavailable":
      "Der GIF-Dienst antwortet gerade nicht. Bitte gleich noch einmal versuchen.",
    "error.upstreamQuota":
      "Dieser Server hat sein GIF-Kontingent gerade aufgebraucht. Bitte später noch einmal versuchen.",
    "error.validationFailed": "Bitte die markierten Felder prüfen.",
    "error.timeout": "Der Server antwortet nicht. Bitte noch einmal versuchen.",
    "error.network": "Keine Verbindung zum Server.",
    "error.internal":
      "Da ist etwas schiefgelaufen. Bitte noch einmal versuchen.",
    "error.invitationRequired": "Zum Registrieren brauchst du eine Einladung.",
    "error.invitationInvalid":
      "Diese Einladung ist unbekannt, abgelaufen oder schon benutzt.",
    "error.accountDisabled": "Dieses Konto ist gesperrt.",
    "error.serverBlocked": "Dieser Server wurde vom Betreiber gesperrt.",

    // Field names inside field messages (auth/rules.ts `FIELD_LABEL`).
    "fieldLabel.email": "E-Mail-Adresse",
    "fieldLabel.password": "Passwort",
    "fieldLabel.currentPassword": "Aktuelles Passwort",
    "fieldLabel.name": "Name",
    "fieldLabel.avatarUrl": "Avatar-URL",
    "fieldLabel.bio": "Über mich",
    "fieldLabel.pronouns": "Pronomen",
    "fieldLabel.statusText": "Status",
    "fieldLabel.accentColor": "Akzentfarbe",
    "fieldLabel.bannerColor": "Bannerfarbe",
    "fieldLabel.bannerUrl": "Banner",
    "fieldLabel.avatarDecoration": "Avatar-Rahmen",
    "fieldLabel.kind": "Kanaltyp",
    "fieldLabel.categoryId": "Kategorie",
    "fieldLabel.memberPermissions": "Rechte",
    "fieldLabel.maxUses": "Maximale Nutzungen",
    "fieldLabel.expiresInHours": "Gültigkeit",
    "fieldLabel.content": "Nachricht",
    "fieldLabel.cursor": "Cursor",
    "fieldLabel.limit": "Anzahl",
    "fieldLabel.userId": "Nutzer",
    "fieldLabel.filename": "Dateiname",
    "fieldLabel.contentType": "Dateityp",
    "fieldLabel.size": "Datei",
    "fieldLabel.attachmentIds": "Anhang",
    "fieldLabel.gif": "GIF",
    "fieldLabel.q": "Suche",
    "fieldLabel.token": "Link",
    "fieldLabel.other": "Eingabe",
    "fieldLabel.color": "Farbe",
    "fieldLabel.roles": "Rollen",
    "fieldLabel.roleIds": "Zugriff",
    "fieldLabel.invitation": "Einladungscode",

    // Field codes (auth/rules.ts `fieldMessage`).
    "fieldError.required": "{label} fehlt.",
    "fieldError.passwordTooShort": "Mindestens {min} Zeichen.",
    "fieldError.tooShort": "{label} ist zu kurz.",
    "fieldError.fileTooLarge": "Die Datei ist zu groß (max. 25 MB).",
    "fieldError.tooManyFiles": "Höchstens 10 Dateien pro Nachricht.",
    "fieldError.tooManyPixels":
      "Das Bild hat zu viele Pixel (max. 16384 px pro Seite, 64 Megapixel).",
    "fieldError.tooLong": "{label} ist zu lang.",
    "fieldError.tooLarge": "Alle Dateien zusammen sind zu groß (max. 100 MB).",
    "fieldError.mismatch": "Der Inhalt der Datei passt nicht zu ihrem Typ.",
    "fieldError.taken": "Diese E-Mail-Adresse ist schon registriert.",
    "fieldError.wrong": "Das Passwort stimmt nicht.",
    "fieldError.stale":
      "Dein Bild wurde inzwischen woanders geändert. Lade die Seite neu.",
    "fieldError.exclusive": "Ein GIF wird ohne Text und Anhang gesendet.",
    "fieldError.unavailable": "GIFs sind auf diesem Server nicht eingerichtet.",
    "fieldError.notEditable": "Ein GIF lässt sich nicht bearbeiten.",
    "fieldError.emailInvalid": "Das sieht nicht wie eine E-Mail-Adresse aus.",
    "fieldError.avatarUrlInvalid": "Bitte eine vollständige https-URL angeben.",
    "fieldError.colorInvalid": "Bitte eine Farbe im Format #rrggbb wählen.",
    "fieldError.bioInvalid":
      "Nur Text und Zeilenumbrüche, keine Steuerzeichen.",
    "fieldError.lineInvalid": "Nur eine Zeile Text, keine Steuerzeichen.",
    "fieldError.contentTypeInvalid": "Dieser Dateityp ist nicht erlaubt.",
    "fieldError.invalid": "{label} ist ungültig.",
    "fieldError.roleNameTaken": "Diesen Namen hat schon eine Rolle.",
    "fieldError.tooManyRoles": "Ein Server hat höchstens 50 Rollen.",
    "fieldError.roleIdsInvalid":
      "Eine der Rollen gibt es nicht mehr. Lade die Seite neu.",

    // A rate limit with a known wait (auth/useFormErrors.ts).
    "form.rateLimited": {
      one: "Zu viele Versuche. Bitte {count} Sekunden warten.",
      other: "Zu viele Versuche. Bitte {count} Sekunden warten.",
    },

    // auth/avatarImage.ts
    "avatarImage.tooLarge": "Das Bild ist zu groß (höchstens 20 MB).",
    "avatarImage.unreadable":
      "Dieses Bild kann der Browser nicht lesen. PNG, JPEG, WebP oder GIF gehen.",

    // auth/session.ts: problems the session notice shows.
    "session.checkFailed":
      "Deine Sitzung konnte nicht geprüft werden. Prüfe deine Verbindung und versuche es erneut.",
    "session.logoutUnconfirmed":
      "Die Abmeldung auf dem Server konnte nicht bestätigt werden. Bitte versuche es erneut.",

    // The auth frame.
    "layout.inviteBanner":
      "Einladung erhalten. Melde dich an oder registriere dich, um beizutreten.",

    // Shared by the auth pages.
    "page.passwordHint": "Mindestens {min} Zeichen.",
    "page.sending": "Wird gesendet…",
    "page.newLinkSent": "Neuer Link gesendet.",
    "page.resendIn": "Erneut senden ({seconds} s)",
    "page.checkInbox": "Postfach prüfen",
    "page.noMail":
      "Keine Mail erhalten? Prüfe den Spam-Ordner oder sende den Link erneut.",
    "page.otherAddress": "Andere Adresse verwenden",
    "page.toSignIn": "Zur Anmeldung",
    "page.backToSignIn": "Zurück zur Anmeldung",
    "page.forgotPassword": "Passwort vergessen?",
    "page.linkExpired": "Link abgelaufen",
    "page.linkUsed": "Dieser Link ist abgelaufen oder wurde schon benutzt.",

    // /login
    "login.sessionExpired": "Sitzung abgelaufen. Melde dich erneut an.",
    "login.title": "Anmelden",
    "login.newHere": "Neu bei Gelabber?",
    "login.createAccount": "Konto anlegen",
    "login.noAccount": "Noch kein Konto?",
    "login.register": "Registrieren",
    "login.submit": "Anmelden",
    "login.submitting": "Anmelden…",
    "login.forgotPasswordNoMail":
      "Passwort vergessen? Frag die Person, die diesen Gelabber-Server betreibt.",

    // /register and the mail it sends
    "register.title": "Konto anlegen",
    "register.haveAccount": "Schon registriert?",
    "register.signIn": "Anmelden",
    "register.nameHint": "Für andere sichtbar.",
    "register.submit": "Registrieren",
    "register.submitting": "Konto wird angelegt…",
    "register.mailSent":
      "Bestätigungslink an <b>{email}</b> gesendet. Er gilt 24 Stunden.",
    "register.thenInvite": "Danach geht es weiter zur Einladung.",
    "register.resend": "Link erneut senden",
    "register.invitationOnly": "Gelabber ist gerade nur mit Einladung offen.",

    // /forgot-password
    "forgot.title": "Passwort vergessen",
    "forgot.lead": "Du erhältst einen Link, um ein neues Passwort festzulegen.",
    "forgot.submit": "Link schicken",
    "forgot.mailSent":
      "Falls zu <b>{email}</b> ein Konto existiert, wurde ein Link gesendet. Er gilt eine Stunde.",
    "forgot.resend": "Erneut senden",

    // /reset-password
    "reset.title": "Neues Passwort",
    "reset.lead": "Danach bist du auf allen anderen Geräten abgemeldet.",
    "reset.passwordLabel": "Neues Passwort",
    "reset.submit": "Passwort speichern",
    "reset.submitting": "Wird gespeichert…",
    "reset.done": "Passwort geändert. Andere Geräte sind abgemeldet.",
    "reset.requestNew": "Neuen Link anfordern",

    // /verify-email
    "verify.title": "E-Mail-Adresse bestätigen",
    "verify.lead": "Gib zur Bestätigung dein Passwort ein.",
    "verify.submit": "Bestätigen",
    "verify.submitting": "Wird bestätigt…",
    "verify.done": "E-Mail-Adresse bestätigt.",
    "verify.signInForNew": "Melde dich an, um einen neuen zu erhalten.",

    // /invite/$code
    "invite.loadFailed": "Einladung konnte nicht geladen werden",
    "invite.toServers": "Zu deinen Servern",
    "invite.toServer": "Zum Server",
    "invite.join": "Beitreten",
    "invite.joining": "Beitreten…",
    "invite.members": { one: "{count} Mitglied", other: "{count} Mitglieder" },
    "invite.alreadyMember": "Du bist bereits Mitglied von",
    "invite.invitedTo": "Einladung zu",
    "invite.bannedTitle": "Kein Zutritt",
    "invite.deadTitle": "Einladung nicht nutzbar",
    "invite.notFound":
      "Dieser Einladungslink existiert nicht oder wurde widerrufen.",
    "invite.askForNew": "Frag nach einem neuen Link.",

    // Unknown paths and pages that throw
    "notFound.title": "Seite nicht gefunden",
    "notFound.body": "Diese Adresse gibt es in Gelabber nicht.",
    "notFound.home": "Zur Startseite",
    "routeError.title": "Etwas ist schiefgelaufen",
    "routeError.body": "Diese Seite konnte nicht angezeigt werden.",
    "routeError.bodyInCall":
      "Diese Seite konnte nicht angezeigt werden. Dein Anruf läuft weiter. Neu laden beendet ihn.",
    "routeError.reload": "Seite neu laden",
  },
  {
    "error.invalidCredentials": "Wrong email or password.",
    "error.themeConflict": "Changed on another device. Please try again.",
    "error.emailTaken": "This email address is already registered.",
    "error.emailUnverified":
      "Please confirm your email address first. We’ve just sent you a new link.",
    "error.tokenInvalid": "This link has expired or has already been used.",
    "error.unauthenticated": "Please sign in again.",
    "error.csrfInvalid": "Security token expired. Please try again.",
    "error.insecureTransport":
      "Signing in only works over an encrypted connection. Open Gelabber with https:// (or on this computer via http://localhost).",
    "error.forbidden": "You don’t have permission for that.",
    "error.notFound": "That doesn’t exist (any more).",
    "error.inviteInvalid": "This invite has expired or been used up.",
    "error.banned": "You are banned from this server.",
    "error.rateLimited":
      "Too many requests. Please wait a moment and try again.",
    "error.quotaExceeded": "Daily upload limit reached.",
    "error.upstreamUnavailable":
      "The GIF service isn’t responding right now. Please try again in a moment.",
    "error.upstreamQuota":
      "This server has used up its GIF quota for now. Please try again later.",
    "error.validationFailed": "Please check the highlighted fields.",
    "error.timeout": "The server isn’t responding. Please try again.",
    "error.network": "No connection to the server.",
    "error.internal": "Something went wrong. Please try again.",
    "error.invitationRequired": "You need an invitation to register.",
    "error.invitationInvalid":
      "This invitation is unknown, expired or already used.",
    "error.accountDisabled": "This account has been disabled.",
    "error.serverBlocked": "This server has been blocked by the operator.",

    "fieldLabel.email": "Email address",
    "fieldLabel.password": "Password",
    "fieldLabel.currentPassword": "Current password",
    "fieldLabel.name": "Name",
    "fieldLabel.avatarUrl": "Avatar URL",
    "fieldLabel.bio": "About me",
    "fieldLabel.pronouns": "Pronouns",
    "fieldLabel.statusText": "Status",
    "fieldLabel.accentColor": "Accent colour",
    "fieldLabel.bannerColor": "Banner colour",
    "fieldLabel.bannerUrl": "Banner",
    "fieldLabel.avatarDecoration": "Avatar frame",
    "fieldLabel.kind": "Channel type",
    "fieldLabel.categoryId": "Category",
    "fieldLabel.memberPermissions": "Permissions",
    "fieldLabel.maxUses": "Maximum uses",
    "fieldLabel.expiresInHours": "Validity",
    "fieldLabel.content": "Message",
    "fieldLabel.cursor": "Cursor",
    "fieldLabel.limit": "Count",
    "fieldLabel.userId": "User",
    "fieldLabel.filename": "File name",
    "fieldLabel.contentType": "File type",
    "fieldLabel.size": "File",
    "fieldLabel.attachmentIds": "Attachment",
    "fieldLabel.gif": "GIF",
    "fieldLabel.q": "Search",
    "fieldLabel.token": "Link",
    "fieldLabel.other": "Input",
    "fieldLabel.color": "Colour",
    "fieldLabel.roles": "Roles",
    "fieldLabel.roleIds": "Access",
    "fieldLabel.invitation": "Invitation code",

    "fieldError.required": "{label} is missing.",
    "fieldError.passwordTooShort": "At least {min} characters.",
    "fieldError.tooShort": "{label} is too short.",
    "fieldError.fileTooLarge": "The file is too large (max. 25 MB).",
    "fieldError.tooManyFiles": "At most 10 files per message.",
    "fieldError.tooManyPixels":
      "The image has too many pixels (max. 16384 px per side, 64 megapixels).",
    "fieldError.tooLong": "{label} is too long.",
    "fieldError.tooLarge": "All files together are too large (max. 100 MB).",
    "fieldError.mismatch": "The file’s content doesn’t match its type.",
    "fieldError.taken": "This email address is already registered.",
    "fieldError.wrong": "The password is wrong.",
    "fieldError.stale":
      "Your image was changed somewhere else in the meantime. Reload the page.",
    "fieldError.exclusive": "A GIF is sent without text or attachments.",
    "fieldError.unavailable": "GIFs aren’t set up on this server.",
    "fieldError.notEditable": "A GIF can’t be edited.",
    "fieldError.emailInvalid": "That doesn’t look like an email address.",
    "fieldError.avatarUrlInvalid": "Please enter a full https URL.",
    "fieldError.colorInvalid": "Please pick a colour in the format #rrggbb.",
    "fieldError.bioInvalid":
      "Only text and line breaks, no control characters.",
    "fieldError.lineInvalid": "Only one line of text, no control characters.",
    "fieldError.contentTypeInvalid": "This file type isn’t allowed.",
    "fieldError.invalid": "{label} is invalid.",
    "fieldError.roleNameTaken": "A role already has this name.",
    "fieldError.tooManyRoles": "A server has at most 50 roles.",
    "fieldError.roleIdsInvalid":
      "One of the roles no longer exists. Reload the page.",

    "form.rateLimited": {
      one: "Too many attempts. Please wait {count} second.",
      other: "Too many attempts. Please wait {count} seconds.",
    },

    "avatarImage.tooLarge": "The image is too large (at most 20 MB).",
    "avatarImage.unreadable":
      "The browser can’t read this image. PNG, JPEG, WebP or GIF work.",

    "session.checkFailed":
      "Your session couldn’t be checked. Check your connection and try again.",
    "session.logoutUnconfirmed":
      "The server couldn’t confirm the sign-out. Please try again.",

    "layout.inviteBanner": "You’ve been invited. Sign in or register to join.",

    "page.passwordHint": "At least {min} characters.",
    "page.sending": "Sending…",
    "page.newLinkSent": "New link sent.",
    "page.resendIn": "Send again ({seconds} s)",
    "page.checkInbox": "Check your inbox",
    "page.noMail": "No email? Check your spam folder or send the link again.",
    "page.otherAddress": "Use a different address",
    "page.toSignIn": "Go to sign-in",
    "page.backToSignIn": "Back to sign-in",
    "page.forgotPassword": "Forgot your password?",
    "page.linkExpired": "Link expired",
    "page.linkUsed": "This link has expired or has already been used.",

    "login.sessionExpired": "Session expired. Please sign in again.",
    "login.title": "Sign in",
    "login.newHere": "New to Gelabber?",
    "login.createAccount": "Create account",
    "login.noAccount": "No account yet?",
    "login.register": "Register",
    "login.submit": "Sign in",
    "login.submitting": "Signing in…",
    "login.forgotPasswordNoMail":
      "Forgot your password? Ask the person who runs this Gelabber server.",

    "register.title": "Create account",
    "register.haveAccount": "Already registered?",
    "register.signIn": "Sign in",
    "register.nameHint": "Visible to others.",
    "register.submit": "Register",
    "register.submitting": "Creating account…",
    "register.mailSent":
      "Confirmation link sent to <b>{email}</b>. It is valid for 24 hours.",
    "register.thenInvite": "After that, you’ll continue to the invite.",
    "register.resend": "Send link again",
    "register.invitationOnly": "Gelabber is open by invitation only for now.",

    "forgot.title": "Forgot password",
    "forgot.lead": "You’ll get a link to set a new password.",
    "forgot.submit": "Send link",
    "forgot.mailSent":
      "If an account exists for <b>{email}</b>, a link has been sent. It is valid for one hour.",
    "forgot.resend": "Send again",

    "reset.title": "New password",
    "reset.lead": "Afterwards you’ll be signed out on all other devices.",
    "reset.passwordLabel": "New password",
    "reset.submit": "Save password",
    "reset.submitting": "Saving…",
    "reset.done": "Password changed. Other devices are signed out.",
    "reset.requestNew": "Request a new link",

    "verify.title": "Confirm email address",
    "verify.lead": "Enter your password to confirm.",
    "verify.submit": "Confirm",
    "verify.submitting": "Confirming…",
    "verify.done": "Email address confirmed.",
    "verify.signInForNew": "Sign in to get a new one.",

    "invite.loadFailed": "The invite couldn’t be loaded",
    "invite.toServers": "Go to your servers",
    "invite.toServer": "Go to server",
    "invite.join": "Join",
    "invite.joining": "Joining…",
    "invite.members": { one: "{count} member", other: "{count} members" },
    "invite.alreadyMember": "You’re already a member of",
    "invite.invitedTo": "Invitation to",
    "invite.bannedTitle": "No entry",
    "invite.deadTitle": "Invite can’t be used",
    "invite.notFound": "This invite link doesn’t exist or has been revoked.",
    "invite.askForNew": "Ask for a new link.",

    "notFound.title": "Page not found",
    "notFound.body": "This address doesn’t exist in Gelabber.",
    "notFound.home": "Go to the start page",
    "routeError.title": "Something went wrong",
    "routeError.body": "This page couldn’t be shown.",
    "routeError.bodyInCall":
      "This page couldn’t be shown. Your call is still running. Reloading ends it.",
    "routeError.reload": "Reload page",
  },
);
