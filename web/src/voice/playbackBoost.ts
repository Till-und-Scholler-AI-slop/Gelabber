// Playback above 100 %. An audio element's volume stops at 1, so a boosted
// element keeps playing its stream, muted (Chromium feeds remote WebRTC audio
// to Web Audio only while an element plays it), and its tracks run through
// one shared graph instead:
//   track sources -> gain per element -> limiter -> output element.
// The output element keeps output-device selection and autoplay handling on
// the element path the session already uses. The desktop app's native
// outputs scale playout themselves and never come here.

type Boost = {
  gain: GainNode;
  sources: Map<MediaStreamTrack, MediaStreamAudioSourceNode>;
};
type Graph = {
  ctx: AudioContext;
  limiter: DynamicsCompressorNode;
  out: HTMLAudioElement;
};

const boosts = new Map<HTMLAudioElement, Boost>();
let graph: Graph | null = null;

function ensureGraph(): Graph | null {
  if (graph) return graph;
  if (
    typeof AudioContext === "undefined" ||
    typeof MediaStream === "undefined" ||
    typeof Audio === "undefined"
  )
    return null;
  let ctx: AudioContext | undefined;
  try {
    ctx = new AudioContext({ latencyHint: "interactive" });
    // Peaks the gain pushes past full scale are held just below it instead
    // of clipping; normal speech stays below the threshold untouched.
    const limiter = ctx.createDynamicsCompressor();
    limiter.threshold.value = -3;
    limiter.knee.value = 0;
    limiter.ratio.value = 20;
    limiter.attack.value = 0.003;
    limiter.release.value = 0.1;
    const destination = ctx.createMediaStreamDestination();
    limiter.connect(destination);
    const out = new Audio();
    out.autoplay = true;
    out.setAttribute("playsinline", "true");
    out.srcObject = destination.stream;
    graph = { ctx, limiter, out };
    return graph;
  } catch {
    void ctx?.close().catch(() => {});
    return null;
  }
}

function closeGraph(): void {
  const shared = graph;
  graph = null;
  if (!shared) return;
  shared.out.pause?.();
  shared.out.srcObject = null;
  void shared.ctx.close().catch(() => {});
}

/**
 * Plays `el` at `gain` (above 1) through the shared graph and follows its
 * current audio tracks. False when this browser cannot: the caller then
 * stays on the element at 100 %.
 */
export function boost(el: HTMLAudioElement, gain: number): boolean {
  const stream = el.srcObject as MediaStream | null;
  if (!stream || typeof stream.getAudioTracks !== "function") {
    releaseBoost(el);
    return false;
  }
  const shared = ensureGraph();
  if (!shared) return false;
  let entry = boosts.get(el);
  if (!entry) {
    const node = shared.ctx.createGain();
    node.connect(shared.limiter);
    entry = { gain: node, sources: new Map() };
    boosts.set(el, entry);
  }
  entry.gain.gain.setTargetAtTime(gain, shared.ctx.currentTime, 0.015);
  const tracks = stream
    .getAudioTracks()
    .filter((track) => track.readyState !== "ended");
  for (const [track, node] of entry.sources) {
    if (tracks.includes(track)) continue;
    node.disconnect();
    entry.sources.delete(track);
  }
  for (const track of tracks) {
    if (entry.sources.has(track)) continue;
    try {
      // One source per track: a stream source only reads its first track.
      const node = shared.ctx.createMediaStreamSource(new MediaStream([track]));
      node.connect(entry.gain);
      entry.sources.set(track, node);
    } catch {
      // Left out; the element still carries it once the boost is released.
    }
  }
  if (tracks.length > 0 && entry.sources.size === 0) {
    releaseBoost(el);
    return false;
  }
  return true;
}

/** Back to the element's own output. True if `el` was boosted. */
export function releaseBoost(el: HTMLAudioElement): boolean {
  const entry = boosts.get(el);
  if (!entry) return false;
  boosts.delete(el);
  for (const node of entry.sources.values()) node.disconnect();
  entry.gain.disconnect();
  if (boosts.size === 0) closeGraph();
  return true;
}

export function isBoosted(el: HTMLAudioElement): boolean {
  return boosts.has(el);
}

/** The element that plays every boosted stream, for output-device changes. */
export function boostOutput(): HTMLAudioElement | null {
  return graph?.out ?? null;
}

/**
 * Starts the shared output. Rejects like `HTMLMediaElement.play()` while the
 * browser still blocks audio, so the caller can ask for a click.
 */
export async function startBoostOutput(): Promise<void> {
  const shared = graph;
  if (!shared) return;
  if (shared.ctx.state !== "running") {
    // resume() stays pending without user activation instead of failing.
    await Promise.race([
      shared.ctx.resume(),
      new Promise((resolve) => setTimeout(resolve, 1000)),
    ]);
    if (graph !== shared) return;
    // Re-read: TypeScript keeps the narrowing from before the await.
    if ((shared.ctx.state as AudioContextState) !== "running")
      throw new DOMException("audio output is blocked", "NotAllowedError");
  }
  await shared.out.play();
}

/** Tests: drop every boost and the shared graph. */
export function resetPlaybackBoostForTests(): void {
  for (const el of [...boosts.keys()]) releaseBoost(el);
  closeGraph();
}
