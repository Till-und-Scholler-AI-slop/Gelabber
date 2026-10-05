export const NATIVE_PCM_POLICY = Object.freeze({
  nodeVersion: 'v26.8.2', nodeSha256: '8a22a371fd85aecf5411636574309f6380fbc42694aaf0651a089a8ef9c44e52',
  chromiumSha256: 'ded93a9c9a53a1ae040f08124badcca95c938e9d5015ff340c3b5538c41bf39e',
  chromiumRevision: '@971a7443b0c9b0a9b2860529b33331b76077ec62', chromiumVersion: '153.0.8010.12',
  // Pinned Performance.now() subtracts clamped origin from clamped current
  // monotonic time. Each TimeClamper term rounds within a 5-us bucket under
  // cross-origin isolation; .1 ms covers both terms and numeric conversion.
  // Scheduling delays are NOT inside epsilon: causal intervals retain them.
  epsilonMs: .1, maxIntervalWidthMs: 25, markerErrorFrames: 96,
  sampleRate: 48000, maxGroups: 2, maxMarkers: 180,
  timeClamperSource: 'https://github.com/chromium/chromium/blob/971a7443b0c9b0a9b2860529b33331b76077ec62/third_party/blink/renderer/core/timing/time_clamper.h#L21',
  performanceSource: 'https://github.com/chromium/chromium/blob/971a7443b0c9b0a9b2860529b33331b76077ec62/third_party/blink/renderer/core/timing/performance.cc',
});
export function validateBrowserClock({ browser, crossOriginIsolated, precision }) {
  if (browser.revision !== NATIVE_PCM_POLICY.chromiumRevision || !browser.product?.endsWith(NATIVE_PCM_POLICY.chromiumVersion) || crossOriginIsolated !== true) throw Error('browser timing policy is unqualified');
  if (!precision || precision.samples !== 100000 || !Number.isFinite(precision.minimumStepMs) || precision.minimumStepMs <= 0 || precision.minimumStepMs > .01) throw Error('current worker timer precision is unqualified');
}
