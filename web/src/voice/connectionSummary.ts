// The figures the connection views draw from the diagnostics samples: the
// ping of the last samples, and the flows grouped the way people think of
// them (everyone's voice in one row, a camera's layers in one row). Shared
// by the call bar's connection popover and the diagnostics tab.

import {
  CALL_QUALITY_LIMITS,
  SOURCE_LABEL,
  callQuality,
  type ConnectionRole,
  type ConnectionSnapshot,
  type DiagnosticSample,
  type FlowStats,
} from "./diagnostics.ts";

/** The ping of the last `count` samples, oldest first, padded with null at
 * the start until that many were taken; null also where a sample measured
 * none. */
export function pingHistory(
  samples: readonly DiagnosticSample[],
  count: number,
): (number | null)[] {
  const recent = samples
    .slice(-count)
    .map((sample) => callQuality(sample.voice)?.rttMs ?? null);
  return [...Array<null>(count - recent.length).fill(null), ...recent];
}

export type QualityTone = "good" | "weak" | "bad";

function tone(value: number, limits: readonly number[]): QualityTone {
  const [, weak, bad] = limits;
  return value >= bad! ? "bad" : value >= weak! ? "weak" : "good";
}

/** The call bar's colours for a ping: weak and bad where the quality drops
 * to two bars and one bar. */
export function pingTone(ms: number): QualityTone {
  return tone(ms, CALL_QUALITY_LIMITS.rttMs);
}

/** The same for audio loss, 0–1. */
export function lossTone(ratio: number): QualityTone {
  return tone(ratio, CALL_QUALITY_LIMITS.loss);
}

export type FlowGroup = {
  key: string;
  direction: "send" | "recv";
  kind: "audio" | "video";
  name: string;
  /** Flows in the group: people for received media, layers for sent video. */
  count: number;
  /** The one codec of the group; null when unknown or mixed. */
  codec: string | null;
  /** Sum of the known bitrates; null when none is known. */
  bitrateBps: number | null;
  /** The worst loss and jitter in the group. */
  loss: number | null;
  jitterMs: number | null;
  /** The largest layer's picture, for video. */
  width: number | null;
  height: number | null;
  fps: number | null;
  /** What the browser says holds a sent video back; null for nothing. */
  limitedBy: "cpu" | "bandwidth" | "other" | null;
};

function flowName(role: ConnectionRole, flow: FlowStats): string {
  if (role === "watch") return flow.kind === "video" ? "Live" : "Live-Ton";
  // Received video on the call's own connection is the others' cameras.
  if (flow.kind === "video" && flow.direction === "recv") return "Video";
  return SOURCE_LABEL[flow.source];
}

function worst(values: (number | null)[]): number | null {
  const known = values.filter(
    (value): value is number => value !== null && Number.isFinite(value),
  );
  return known.length > 0 ? Math.max(...known) : null;
}

function limitOf(flow: FlowStats): FlowGroup["limitedBy"] {
  const reason = flow.qualityLimitationReason;
  return reason === "cpu" || reason === "bandwidth" || reason === "other"
    ? reason
    : null;
}

/** One row per kind of media and direction: sent before received, audio
 * before video, in the order the flows first appear within that. */
export function flowGroups(
  snapshot: ConnectionSnapshot | null,
  role: ConnectionRole,
): FlowGroup[] {
  const groups = new Map<string, FlowStats[]>();
  for (const flow of snapshot?.flows ?? []) {
    const key = `${role}:${flow.direction}:${flow.kind}:${flowName(role, flow)}`;
    groups.set(key, [...(groups.get(key) ?? []), flow]);
  }
  const order = (group: FlowGroup) =>
    (group.direction === "send" ? 0 : 2) + (group.kind === "audio" ? 0 : 1);
  return [...groups.entries()]
    .map(([key, flows]): FlowGroup => {
      const first = flows[0]!;
      const codecs = new Set(flows.map((flow) => flow.codec));
      const rates = flows
        .map((flow) => flow.measuredBitrateBps)
        .filter(
          (value): value is number => value !== null && Number.isFinite(value),
        );
      const largest = flows.reduce((best, flow) =>
        (flow.width ?? 0) * (flow.height ?? 0) >
        (best.width ?? 0) * (best.height ?? 0)
          ? flow
          : best,
      );
      return {
        key,
        direction: first.direction,
        kind: first.kind,
        name: flowName(role, first),
        count: flows.length,
        codec: codecs.size === 1 ? first.codec : null,
        bitrateBps: rates.length > 0 ? rates.reduce((a, b) => a + b, 0) : null,
        loss: worst(flows.map((flow) => flow.packetLoss)),
        jitterMs: worst(flows.map((flow) => flow.jitterMs)),
        width: largest.width,
        height: largest.height,
        fps: largest.fps,
        limitedBy: flows.map(limitOf).find((limit) => limit !== null) ?? null,
      };
    })
    .sort((a, b) => order(a) - order(b));
}

/** Everything sent or received right now over the call and a Watch;
 * null when no flow in that direction knows its bitrate. */
export function directionTotal(
  snapshots: readonly (ConnectionSnapshot | null)[],
  direction: "send" | "recv",
): number | null {
  const rates = snapshots
    .flatMap((snapshot) => snapshot?.flows ?? [])
    .filter((flow) => flow.direction === direction)
    .map((flow) => flow.measuredBitrateBps)
    .filter(
      (value): value is number => value !== null && Number.isFinite(value),
    );
  return rates.length > 0 ? rates.reduce((a, b) => a + b, 0) : null;
}
