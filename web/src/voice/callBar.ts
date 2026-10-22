// What the call bar says beyond its controls: the one notice it shows about
// the own Live's sound, and what a screen reader hears when the call
// changes.

import type { VoiceState } from "./session.ts";
import { SOURCE_AUDIO_CARRIES_CALL, sourceAudioScope } from "./settings.ts";

export type NoticeContext = {
  desktop: boolean;
  appAudio: boolean;
  /** In a call, with share sound switched on. */
  sharesSound: boolean;
  carriesCall: boolean;
  sourceAudioApp: string;
};

/** What the dock may have to say about the own Live's sound, most
 * important first. It shows one at a time; each is about a Live that
 * runs, never a warning ahead of one. */
export function dockNotices(
  voice: Pick<
    VoiceState,
    "sourceAudio" | "sourceAudioNote" | "localScreen" | "localLive"
  >,
  context: NoticeContext,
): { key: string; text: string }[] {
  const { sourceAudio, sourceAudioNote: notes } = voice;
  const failed = notes.s?.failed ?? notes.l?.failed;
  const silent = notes.s?.silent ?? notes.l?.silent;
  const running = sourceAudio.s === "sharing" || sourceAudio.l === "sharing";
  const notices: { id: string; text: string | null }[] = [
    {
      id: "unavailable",
      text:
        sourceAudio.s === "unavailable" || sourceAudio.l === "unavailable"
          ? `${
              !context.desktop
                ? "Der Browser hat keinen Stream-Ton freigegeben."
                : !context.appAudio
                  ? "Diese Desktop-App kann keinen Ton von Anwendungen teilen."
                  : failed
                    ? `Der Stream-Ton konnte nicht aufgenommen werden (${failed}).`
                    : "Der Stream-Ton konnte nicht aufgenommen werden."
            } Das Video läuft weiter.`
          : null,
    },
    {
      id: "unsupported",
      text:
        sourceAudio.s === "unsupported" || sourceAudio.l === "unsupported"
          ? "Dieser Medienserver unterstützt Stream-Ton noch nicht. Das Video läuft weiter."
          : null,
    },
    {
      id: "ended",
      text:
        sourceAudio.s === "ended" || sourceAudio.l === "ended"
          ? "Der Stream-Ton wurde beendet. Das Video läuft weiter."
          : null,
    },
    {
      id: "silent",
      text: silent ? `Von „${silent}“ kommt gerade kein Ton.` : null,
    },
    {
      // Without a chosen application the desktop app sends the sound of all
      // of them with the one window that was picked, and it does so unasked.
      id: "scope",
      text:
        running && context.sourceAudioApp === ""
          ? [
              sourceAudioScope({ sourceAudioApp: "" })
                ? `Dein Stream sendet den ${sourceAudioScope({ sourceAudioApp: "" })}.`
                : null,
              context.sharesSound && context.carriesCall
                ? SOURCE_AUDIO_CARRIES_CALL
                : null,
            ]
              .filter(Boolean)
              .join(" ") || null
          : null,
    },
    {
      // A browser hands sound over in its picker only: a Live that started
      // without it stays without it.
      id: "next-share",
      text:
        context.sharesSound &&
        !context.desktop &&
        ((voice.localScreen && sourceAudio.s === "off") ||
          (voice.localLive && sourceAudio.l === "off"))
          ? "Der Stream-Ton kommt beim nächsten Live gehen dazu."
          : null,
    },
  ];
  return notices.flatMap(({ id, text }) =>
    text ? [{ key: `${id}:${text}`, text }] : [],
  );
}

export type CallMoment = Pick<
  VoiceState,
  "status" | "connection" | "channelName"
>;

/** What a screen reader hears when the call moves on from `before`, if
 * anything. */
export function callAnnouncement(
  before: CallMoment,
  now: CallMoment,
): string | null {
  if (now.connection === "failed")
    return before.connection === "failed" ? null : "Verbindung verloren";
  if (now.status === "idle")
    return before.status === "joined" ? "Getrennt" : null;
  if (now.connection === "connected") {
    if (before.connection === "reconnecting")
      return "Verbindung wiederhergestellt";
    return before.connection !== "connected" ||
      before.channelName !== now.channelName
      ? `Mit ${now.channelName ?? "dem Sprachkanal"} verbunden`
      : null;
  }
  if (now.connection === "reconnecting" && before.connection !== "reconnecting")
    return "Verbindung wird wiederhergestellt";
  return null;
}
