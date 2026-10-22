// The voice store: what the call UI renders. session.ts drives it and
// re-exports it; modules that session.ts uses read it from here, so they do
// not import session.ts back.

import { create } from "zustand";

import type { TrackKind } from "../ws/protocol.ts";

export type SourceAudioStatus =
  "off" | "sharing" | "unavailable" | "ended" | "unsupported";
/** What the status alone does not say: the desktop capture's own reason for
 * "unavailable", or the chosen application that plays nothing while
 * "sharing". */
export type SourceAudioNote = { failed?: string; silent?: string };

export type VoiceStatus = "idle" | "joined";
/** What the call's media connection is doing, for the call bar to say.
 * Observed only: the retry logic does not read it. `failed` outlives the
 * call it reports, until the user joins again or dismisses it. */
export type VoiceConnection =
  "idle" | "connecting" | "connected" | "reconnecting" | "failed";
/** Whether the own microphone reaches the call: captured and sending,
 * missing or refused (the call goes on listening), or held closed by
 * push-to-talk. */
export type VoiceMic = "live" | "unavailable" | "denied" | "ptt-idle";
/** The call a `failed` connection ended, to join it again. */
export type LostCall = {
  serverId: string;
  channelId: string;
  channelName: string;
};

export type VoiceParticipant = {
  pubs: TrackKind[];
};

export type RemoteVideo = {
  v?: MediaStream;
  s?: MediaStream;
};

export type VoiceState = {
  status: VoiceStatus;
  connection: VoiceConnection;
  mic: VoiceMic;
  lostCall: LostCall | null;
  serverId: string | null;
  channelId: string | null;
  channelName: string | null;
  muted: boolean;
  deafened: boolean;
  camera: boolean;
  /** The own Live: a screen or window. */
  sharing: boolean;
  sourceWatchSupported: boolean;
  /** The own Live's sound. */
  sourceAudio: Record<"s", SourceAudioStatus>;
  sourceAudioNote: Record<"s", SourceAudioNote | null>;
  /** Whose Live plays here. */
  sourceSubscriptions: Record<string, Partial<Record<"s", boolean>>>;
  localCamera: MediaStream | null;
  localScreen: MediaStream | null;
  playbackBlocked: boolean;
  remote: Record<string, RemoteVideo>;
  participants: Record<string, VoiceParticipant>;
};

export const IDLE_VOICE: VoiceState = {
  status: "idle",
  connection: "idle",
  mic: "live",
  lostCall: null,
  serverId: null,
  channelId: null,
  channelName: null,
  muted: false,
  deafened: false,
  camera: false,
  sharing: false,
  sourceWatchSupported: false,
  sourceAudio: { s: "off" },
  sourceAudioNote: { s: null },
  sourceSubscriptions: {},
  localCamera: null,
  localScreen: null,
  playbackBlocked: false,
  remote: {},
  participants: {},
};

export const useVoice = create<VoiceState>(() => ({ ...IDLE_VOICE }));
