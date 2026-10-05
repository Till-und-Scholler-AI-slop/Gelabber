# Native PCM callback clock instrument

This separate local instrument qualifies a **physical AudioWorklet input
callback availability interval**. It does not use output-device timestamps to
project `currentFrame` into a hardware clock. The clock-only entry point creates
zero native peers, SDP exchanges, ICE candidates or RTP streams. Its output
always retains `pcm_latency_calibrated:false` and `comparison_available:false`.
Native V2 replay and known-delay acceptance require a later, explicit reader
and pacing freeze; clock-only success cannot qualify that path.

The only owned implementation files are `native-pcm-{ring,clock-bounds,
observer,worklet,policy,evidence,bridge,control}.mjs` and the three
`tests/nativePcm*.test.mjs` files. Existing PCM helpers, native core, SFU adapters,
pins and product sources are unchanged. The worklet imports the actual frozen
`pcm-kernel.mjs`; the runner snapshots and hashes every helper before/after.

## Causal clock boundaries

All browser stamps use the **same DedicatedWorker performance origin**. The
worker publishes a heartbeat into a BigInt SharedArrayBuffer. The AudioWorklet
loads that heartbeat before processing its input and commits a ring record
after processing. The worker accepts that record only between two equal,
positive monotone sequence fences, and timestamps its later observation.
Slot overwrite, partial writes and sequence regression fail qualification.
No Worklet `performance` API or fixed128-frame quantum is assumed.

The worker also performs serialized synchronous native clock RPCs. A response
whose request has completed before the conservative callback lower endpoint
provides a native lower bound. A request that starts after the callback upper
endpoint provides its native upper bound. The remote timestamp is acquired
inside that request. This uses causal order directly; it does not fit an
offset, extrapolate a sample clock, assume a browser/Linux rate ratio or silently
correct drift. Callback scheduling, RPC queuing and delayed observation widen
the actual interval. Missing brackets or widths above25ms reject it.

The timing policy requires actual Node26.8.2 and Chromium153.0.8010.12 binary
hashes, Chromium revision`971a7443b0c9b0a9b2860529b33331b76077ec62`, and true
cross-origin isolation. That revision's
[TimeClamper](https://github.com/chromium/chromium/blob/971a7443b0c9b0a9b2860529b33331b76077ec62/third_party/blink/renderer/core/timing/time_clamper.h#L21)
uses5µs isolated buckets, with bounded rounding to an adjacent bucket in its
[implementation](https://github.com/chromium/chromium/blob/971a7443b0c9b0a9b2860529b33331b76077ec62/third_party/blink/renderer/core/timing/time_clamper.cc#L19).
The explicit0.1ms margin per endpoint conservatively includes that rounding and
float-to-nanosecond conversion. `Performance.now()` subtracts the separately
clamped time origin from clamped current monotonic time; both rounding terms
are covered by that margin. It does **not** include scheduling delays.
Every run additionally records100,000 Worker timer observations and rejects a
minimum positive step above10µs. This empirical observation checks the current
runtime; it is not a universal browser precision guarantee. A different
binary/revision/privacy policy requires a new timing policy and controls.

[High Resolution Time](https://www.w3.org/TR/hr-time-3/#the-time-origin)
permits implementation-dependent coarsening, including coarser clocks.
[Web Audio output timestamps](https://www.w3.org/TR/webaudio/#dom-audiocontext-getoutputtimestamp)
describe the output device position. They are recorded only as debug data;
neither those timestamps nor `baseLatency`/`outputLatency` determine a Worklet
input callback's physical clock interval here. No acoustic, DAC, wall-clock,
live capture, live encoder or DSP latency claim follows.

## Marker integrity and V2 contract

The optional marker helper is all-or-nothing: both direct-loopback roles bind
the actual held live/enabled receiver, Opus report and expected UID/SSRC
(mic0/`0x474d4943`, source64/`0x47534130`). SFU-rewritten SSRCs need their own
explicit publication/consumer mapping; this direct-loopback contract cannot
be copied to a rewritten edge based on a display label. Actual received-packet
and decoded-sample progress plus available zero loss/concealment counters are
required. Callback `inputFrames` alone does not establish absence of PLC.

Finite expected marker sequences must each match exactly once; no missing,
duplicate, unexpected or out-of-window marker is accepted as a partial result.
The full marker waveform plus tolerance must have contiguous valid callback
input. Clipping, nonfinite PCM, ring loss, frame gaps and AudioContext
suspension reject qualification. Bounded detector candidates avoid allocating
one detector per whole-run code; a marker delayed beyond the candidate window
becomes a missing-marker failure, never an aliased later cycle.

Source time is `startClockNs + sourceSampleOrdinal/48000`, with exact BigInt
floor/ceil and the **actual** native conversion bracket conservatively retained.
The detected start's ±96sample tolerance is propagated through every
intersecting callback. This is the existing offline detector rejection limit;
native/browser known-delay controls remain necessary. Codec lookahead remains
in the archive and measurement; it is not silently subtracted.

The agreed future V2 RPC is `start{total_seconds:<entire archive incl tail>,
audio_hold_ms:0|50|200|500}`. It must send identical unlooped UID0/64 archives
once, preserve their shared run/codebook hash, retain the common source anchor,
and expose actual before/after enqueue plus full-end/tail/hold evidence. Hold
is test-only and requires an explicit native CLI flag. The planned source PCM
time remains unshifted. The bridge rejects V1`seconds`, a truncated duration,
unsupported holds and foreign peer identities. Until the actual native reader
implements/freezes this contract, the shipped control runner is clock-only.

## Local execution

Use fresh output paths and actual frozen binaries/inputs. Without `--execute`,
the runner only hashes/prints the plan. It starts no native peer or browser.

```sh
/absolute/node-v26.8.2/bin/node media/bench/native-pcm-control.mjs --execute \
  --binary /absolute/native-peer --binary-sha256 <actual-sha256> \
  --video /absolute/video.rtpbin --mic /absolute/mic.opusbin \
  --source /absolute/source.opusbin --library /absolute/libopus.so.0 \
  --chromium /absolute/chrome-headless-shell --seconds 10 \
  --output /tmp/fresh-native-clock-control
```

The runner binds its private HTTP server to loopback port0 and uses a random
bearer token, retained only in
the browser worker. Exact static module names are allowlisted. The native child
has a bounded serialized RPC queue and response limits; its actual mapped
executable, imported archives and mapped Opus decoder are checked. Cleanup
closes the owned browser/server/child only. Raw callback/probe evidence goes to
`report.json`; `summary.json` contains its SHA256 and compact results.

Negative browser seams are `--clock-delay-ms 100`, `--observer-pause-ms 250
--capacity 2`, and `--suspend-ms 200`. A failed control must exit nonzero with
qualificationfalse; it must never print a fabricated narrow latency interval.
Run these only in the agreed local QA window, not alongside SFU/browser load.

```sh
/absolute/node-v26.8.2/bin/node --test media/bench/tests/nativePcm*.test.mjs
```

The tests cover causal clock ordering/stalls, exact sample ratios, ring
overwrite/torn writes/sequence wrap, real-kernel duplicate/wrongUID/missing PN,
PCM integrity, live/enabled receiver ownership, PLC counter availability,
suspension, private HTTP scope and serialized RPC failure. They do not qualify
a native V2 source-to-browser measurement or an SFU comparison.
