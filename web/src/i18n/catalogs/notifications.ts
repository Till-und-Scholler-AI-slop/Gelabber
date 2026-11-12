// Notifications: the settings area (Nachrichten, Sperrbildschirm), the
// browser permission, and the per-server, channel or DM dialog with its
// mute and level choices, which the row menus show too.
import { defineCatalog } from "../catalog.ts";

export const notifications = defineCatalog(
  "notifications",
  {
    "alerts.title": "Nachrichten",
    "alerts.toasts": "Popup bei neuen Nachrichten in anderen Chats",
    "alerts.desktop":
      "Browser-Benachrichtigung, wenn der Tab nicht im Vordergrund ist",
    "alerts.sound": "Ton bei Benachrichtigungen am Computer",
    "alerts.soundHint":
      "Ein Ton für mehrere Nachrichten kurz hintereinander, keiner, während du taubgeschaltet bist. Auf dem Handy gilt die Geräteeinstellung.",

    "permission.pushOn":
      "Direktnachrichten und @Erwähnungen kommen auch an, wenn Gelabber geschlossen ist.",
    "permission.pushAfterAllow":
      "Danach kommen Direktnachrichten und @Erwähnungen auch bei geschlossener App an, sofern der Server das unterstützt.",
    "permission.whileRunning":
      "Auf diesem Gerät kommen Benachrichtigungen nur an, solange Gelabber im Hintergrund läuft.",
    "permission.allow": "Benachrichtigungen erlauben",
    "permission.failed":
      "Die Berechtigung konnte nicht abgefragt werden. Versuche es erneut.",

    "privacy.title": "Sperrbildschirm",
    "privacy.hideContent": "Inhalt in Benachrichtigungen verbergen",
    "privacy.hideContentHint":
      "Benachrichtigungen zeigen nur „Neue Nachricht von …“, ohne Text und ohne Chat. Gilt für alle Geräte deines Kontos.",
    "privacy.failed":
      "Die Einstellung konnte nicht gespeichert werden. Versuche es erneut.",

    "dialog.title": "Benachrichtigungen",
    "dialog.titleFor": "Benachrichtigungen: {name}",
    "dialog.gone": "Nicht mehr verfügbar.",
    "dialog.notSaved": "Nicht gespeichert. Versuch es noch einmal.",
    "dialog.mute": "Stummschalten",
    "dialog.unmute": "Aufheben",
    "dialog.muteHint.dm":
      "Stumm: kein Zähler, keine Benachrichtigung und kein Ton.",
    "dialog.muteHint.channel":
      "Stumm: kein Punkt, keine Benachrichtigung und kein Ton. @Erwähnungen zählen weiter.",
    "dialog.serverMuted": "Der ganze Server ist gerade stumm.",
    "dialog.levelLegend": "Benachrichtigen bei",
    "dialog.markRead": "Als gelesen markieren",

    "bell.label": "Benachrichtigungen für {name}",
    "bell.labelMuted": "Benachrichtigungen für {name} (stumm)",
    "bell.title": "Benachrichtigungen",
    "bell.titleMuted": "Stummgeschaltet",

    "mute.quarterHour": "15 Minuten",
    "mute.quarterHour.short": "15 Min.",
    "mute.hour": "1 Stunde",
    "mute.hour.short": "1 Std.",
    "mute.eightHours": "8 Stunden",
    "mute.eightHours.short": "8 Std.",
    "mute.day": "24 Stunden",
    "mute.day.short": "24 Std.",
    "mute.forever": "Bis ich es wieder einschalte",
    "mute.forever.short": "Unbegrenzt",

    "level.all": "Alle Nachrichten",
    "level.all.hint": "Jede neue Nachricht",
    "level.mentions": "Nur @Erwähnungen",
    "level.mentions.hint":
      "Wenn dich jemand erwähnt oder @everyone/@here schreibt",
    "level.none": "Nichts",
    "level.none.hint": "Keine Benachrichtigungen, kein Zähler",
    "level.inherit": "Wie der Server ({level})",

    "muted.until": "Stumm bis {until}",
    "muted.untilUnmuted": "Stumm, bis du es aufhebst",
    "until.today": "{time} Uhr",
    "until.later": "{date}, {time} Uhr",
  },
  {
    "alerts.title": "Messages",
    "alerts.toasts": "Pop-up for new messages in other chats",
    "alerts.desktop": "Browser notification when the tab is in the background",
    "alerts.sound": "Sound for notifications on a computer",
    "alerts.soundHint":
      "One sound for several messages in a row, none while you are deafened. On a phone, the device setting applies.",

    "permission.pushOn":
      "Direct messages and @mentions arrive even when Gelabber is closed.",
    "permission.pushAfterAllow":
      "Then direct messages and @mentions arrive even when the app is closed, if the server supports it.",
    "permission.whileRunning":
      "On this device, notifications only arrive while Gelabber runs in the background.",
    "permission.allow": "Allow notifications",
    "permission.failed": "Could not ask for permission. Try again.",

    "privacy.title": "Lock screen",
    "privacy.hideContent": "Hide content in notifications",
    "privacy.hideContentHint":
      "Notifications only show “New message from …”, without text or chat. Applies to all devices on your account.",
    "privacy.failed": "The setting could not be saved. Try again.",

    "dialog.title": "Notifications",
    "dialog.titleFor": "Notifications: {name}",
    "dialog.gone": "No longer available.",
    "dialog.notSaved": "Not saved. Try again.",
    "dialog.mute": "Mute",
    "dialog.unmute": "Unmute",
    "dialog.muteHint.dm": "Muted: no count, no notification and no sound.",
    "dialog.muteHint.channel":
      "Muted: no dot, no notification and no sound. @mentions still count.",
    "dialog.serverMuted": "The whole server is muted right now.",
    "dialog.levelLegend": "Notify me about",
    "dialog.markRead": "Mark as read",

    "bell.label": "Notifications for {name}",
    "bell.labelMuted": "Notifications for {name} (muted)",
    "bell.title": "Notifications",
    "bell.titleMuted": "Muted",

    "mute.quarterHour": "15 minutes",
    "mute.quarterHour.short": "15 min",
    "mute.hour": "1 hour",
    "mute.hour.short": "1 hr",
    "mute.eightHours": "8 hours",
    "mute.eightHours.short": "8 hrs",
    "mute.day": "24 hours",
    "mute.day.short": "24 hrs",
    "mute.forever": "Until I turn it back on",
    "mute.forever.short": "Forever",

    "level.all": "All messages",
    "level.all.hint": "Every new message",
    "level.mentions": "Only @mentions",
    "level.mentions.hint":
      "When someone mentions you or writes @everyone/@here",
    "level.none": "Nothing",
    "level.none.hint": "No notifications, no count",
    "level.inherit": "Same as the server ({level})",

    "muted.until": "Muted until {until}",
    "muted.untilUnmuted": "Muted until you unmute",
    "until.today": "{time}",
    "until.later": "{date}, {time}",
  },
);
