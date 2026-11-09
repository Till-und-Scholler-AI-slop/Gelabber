// What the call bar says beyond its controls: the one notice it shows about
// the own Live's sound, and what a screen reader hears when the call
// changes.

import type { VoiceState } from "./session.ts";

export type NoticeContext = {
  /** In a call, with the sound of a Live switched on. */
  sharesSound: boolean;
};

/** What the dock may have to say about the own Live's sound, most
 * important first. It shows one at a time; each is about a Live that
 * runs, never a warning ahead of one. */
export function dockNotices(
  voice: Pick<VoiceState, "sourceAudio" | "localScreen">,
  context: NoticeContext,
): { key: string; text: string }[] {
  const sound = voice.sourceAudio.s;
  const notices: { id: string; text: string | null }[] = [
    {
      id: "unavailable",
      text:
        sound === "unavailable"
          ? "Der Browser hat keinen Live-Ton freigegeben. Das Video läuft weiter."
          : null,
    },
    {
      id: "unsupported",
      text:
        sound === "unsupported"
          ? "Dieser Medienserver unterstützt Live-Ton noch nicht. Das Video läuft weiter."
          : null,
    },
    {
      id: "ended",
      text:
        sound === "ended"
          ? "Der Live-Ton wurde beendet. Das Video läuft weiter."
          : null,
    },
    {
      // A browser hands sound over in its picker only: a Live that started
      // without it stays without it.
      id: "next-share",
      text:
        context.sharesSound && voice.localScreen && sound === "off"
          ? "Ton ab dem nächsten Live."
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
