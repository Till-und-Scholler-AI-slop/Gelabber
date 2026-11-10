// What the call bar says beyond its controls: the one notice it shows about
// the own Live's sound, and what a screen reader hears when the call
// changes.

import { translate } from "../i18n/catalog.ts";
import { voice as texts } from "../i18n/catalogs/voice.ts";
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
          ? translate(texts, "notice.soundUnavailable")
          : null,
    },
    {
      id: "unsupported",
      text:
        sound === "unsupported"
          ? translate(texts, "notice.soundUnsupported")
          : null,
    },
    {
      id: "ended",
      text: sound === "ended" ? translate(texts, "notice.soundEnded") : null,
    },
    {
      // A browser hands sound over in its picker only: a Live that started
      // without it stays without it.
      id: "next-share",
      text:
        context.sharesSound && voice.localScreen && sound === "off"
          ? translate(texts, "notice.soundNextLive")
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
    return before.connection === "failed"
      ? null
      : translate(texts, "announce.lost");
  if (now.status === "idle")
    return before.status === "joined"
      ? translate(texts, "announce.disconnected")
      : null;
  if (now.connection === "connected") {
    if (before.connection === "reconnecting")
      return translate(texts, "announce.restored");
    return before.connection !== "connected" ||
      before.channelName !== now.channelName
      ? now.channelName
        ? translate(texts, "announce.connectedTo", { channel: now.channelName })
        : translate(texts, "announce.connectedToVoice")
      : null;
  }
  if (now.connection === "reconnecting" && before.connection !== "reconnecting")
    return translate(texts, "announce.reconnecting");
  return null;
}
