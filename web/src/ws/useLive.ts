// Bridge gateway frames into the live stores, plus per-tab idle + typing.

import { useEffect, useRef } from "react";

import { useSession } from "../auth/session.ts";
import { getGateway } from "./client.ts";
import {
  CLIENT_IDLE_MS,
  PRESENCE_ACTIVITY_EVENTS,
  applyPresence,
  applyPresenceSnap,
  applyTyping,
  createTypingSender,
  pruneTyping,
  resetLiveStores,
} from "./live.ts";
import { createIdlePresence } from "./idlePresence.ts";
import { applyVoiceSig, resetVoiceRoster } from "../voice/roster.ts";
import { useVoice, type VoiceState } from "../voice/voiceStore.ts";

export function useLiveBridge(): void {
  useEffect(() => {
    const gateway = getGateway();
    const offPresence = gateway.onPresence((frame) => {
      if (frame.snap) {
        applyPresenceSnap(frame.s, frame.snap);
        return;
      }
      if (frame.u && frame.st) {
        applyPresence(frame.s, frame.u, frame.st);
      }
    });
    const offTyping = gateway.onTyping((frame) => {
      applyTyping(frame.c, frame.u, frame.on, Date.now());
    });
    const offSig = gateway.onSig((event) => {
      applyVoiceSig(event);
    });
    const tick = window.setInterval(() => pruneTyping(Date.now()), 400);
    return () => {
      offPresence();
      offTyping();
      offSig();
      window.clearInterval(tick);
    };
  }, []);
}

export function useIdlePresence(userId: string | null): void {
  useEffect(() => {
    if (!userId) {
      resetLiveStores();
      resetVoiceRoster();
      return undefined;
    }
    const gateway = getGateway();
    const presence = createIdlePresence((st) => gateway.sendPresence(st));
    const inCall = (voice: VoiceState) =>
      voice.status === "joined" && !voice.deafened;
    presence.input();
    presence.setInCall(inCall(useVoice.getState()));
    const offVoice = useVoice.subscribe((voice) =>
      presence.setInCall(inCall(voice)),
    );
    const onActivity = () => presence.input();
    for (const type of PRESENCE_ACTIVITY_EVENTS) {
      window.addEventListener(type, onActivity);
    }
    return () => {
      presence.stop();
      offVoice();
      for (const type of PRESENCE_ACTIVITY_EVENTS) {
        window.removeEventListener(type, onActivity);
      }
    };
  }, [userId]);
}

/**
 * Tell the server while this page is visible and someone uses it: it then
 * holds Web Push back for the account (#243), as the page itself shows or
 * announces what arrives. Visible rather than focused, so a push never
 * reaches a service worker that must stay quiet for a visible window; idle
 * after `CLIENT_IDLE_MS` without input, so a screen nobody sits at does not
 * hold back the phone's pushes.
 */
export function useFocusReport(userId: string | null): void {
  useEffect(() => {
    if (!userId) return undefined;
    const gateway = getGateway();
    let idle = false;
    let idleTimer: ReturnType<typeof setTimeout> | null = null;
    const report = () => gateway.setInFront(!document.hidden && !idle);
    const goIdle = () => {
      idle = true;
      report();
    };
    const onActivity = () => {
      if (idleTimer) window.clearTimeout(idleTimer);
      idleTimer = window.setTimeout(goIdle, CLIENT_IDLE_MS);
      if (idle) {
        idle = false;
        report();
      }
    };
    onActivity();
    report();
    for (const type of PRESENCE_ACTIVITY_EVENTS)
      window.addEventListener(type, onActivity);
    document.addEventListener("visibilitychange", report);
    return () => {
      if (idleTimer) window.clearTimeout(idleTimer);
      for (const type of PRESENCE_ACTIVITY_EVENTS)
        window.removeEventListener(type, onActivity);
      document.removeEventListener("visibilitychange", report);
      gateway.setInFront(false);
    };
  }, [userId]);
}

export function useTypingInput(
  serverId: string,
  channelId: string,
  enabled: boolean,
): {
  onChange: (value: string) => void;
  stop: () => void;
} {
  const me = useSession((s) => s.user?.id);
  const sender = useRef<ReturnType<typeof createTypingSender> | null>(null);
  sender.current ??= createTypingSender((s, c, on) =>
    getGateway().sendTyping(s, c, on),
  );
  const stop = () => sender.current?.stop();

  useEffect(() => () => stop(), [serverId, channelId]);

  return {
    onChange: (value: string) => {
      if (!enabled || !me) return;
      sender.current?.input(serverId, channelId, value);
    },
    stop,
  };
}
