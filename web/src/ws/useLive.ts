// Bridge gateway frames into the live stores, plus per-tab idle + typing.

import { useEffect, useRef } from "react";

import { useSession } from "../auth/session.ts";
import { getGateway } from "./client.ts";
import {
  CLIENT_IDLE_MS,
  PRESENCE_ACTIVITY_EVENTS,
  PRESENCE_PULSE_MS,
  TYPING_REFRESH_MS,
  applyPresence,
  applyPresenceSnap,
  applyTyping,
  pruneTyping,
  resetLiveStores,
} from "./live.ts";

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
    const tick = window.setInterval(() => pruneTyping(Date.now()), 400);
    return () => {
      offPresence();
      offTyping();
      window.clearInterval(tick);
    };
  }, []);
}

export function useIdlePresence(authenticated: boolean): void {
  useEffect(() => {
    if (!authenticated) {
      resetLiveStores();
      return undefined;
    }
    const gateway = getGateway();
    let idle = false;
    let lastPulse = 0;
    let idleTimer: ReturnType<typeof setTimeout> | null = null;

    const goIdle = () => {
      if (idle) return;
      idle = true;
      gateway.sendPresence("i");
    };
    const goActive = () => {
      const now = Date.now();
      if (idle) {
        idle = false;
        gateway.sendPresence("o");
        lastPulse = now;
      } else if (now - lastPulse >= PRESENCE_PULSE_MS) {
        gateway.sendPresence("o");
        lastPulse = now;
      }
      if (idleTimer) window.clearTimeout(idleTimer);
      idleTimer = window.setTimeout(goIdle, CLIENT_IDLE_MS);
    };

    goActive();
    const onActivity = () => goActive();
    for (const type of PRESENCE_ACTIVITY_EVENTS) {
      window.addEventListener(type, onActivity);
    }
    return () => {
      if (idleTimer) window.clearTimeout(idleTimer);
      for (const type of PRESENCE_ACTIVITY_EVENTS) {
        window.removeEventListener(type, onActivity);
      }
    };
  }, [authenticated]);
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
  const typing = useRef(false);
  const refresh = useRef<ReturnType<typeof setInterval> | null>(null);
  const sent = useRef<{ s: string; c: string } | null>(null);

  const stop = () => {
    if (!typing.current) return;
    typing.current = false;
    if (refresh.current) {
      window.clearInterval(refresh.current);
      refresh.current = null;
    }
    const target = sent.current;
    sent.current = null;
    if (target) {
      getGateway().sendTyping(target.s, target.c, false);
    }
  };

  useEffect(() => () => stop(), [serverId, channelId]);

  return {
    onChange: (value: string) => {
      if (!enabled || !me) return;
      if (value.length === 0) {
        stop();
        return;
      }
      if (!typing.current) {
        typing.current = true;
        sent.current = { s: serverId, c: channelId };
        getGateway().sendTyping(serverId, channelId, true);
        refresh.current = window.setInterval(() => {
          const target = sent.current;
          if (target) getGateway().sendTyping(target.s, target.c, true);
        }, TYPING_REFRESH_MS);
      }
    },
    stop,
  };
}
