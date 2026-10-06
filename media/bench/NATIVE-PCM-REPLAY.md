# Whole V2 archive/replay contract

`native-pcm-replay-contract.mjs` is a separate guard around the frozen causal
clock instrument. It imports real GPOPUS2 bytes and derives the entire expected
UID0/64 marker book from measurement duration. It supplies those immutable
markers to `qualifyNativeMarkers`; a caller cannot substitute a shortened
marker list, partial receiver group or independent run. Existing Clock13 files,
PN builder, native reader/pacer and Full-N adapters are unchanged by this guard.

This is not a native playback, clock calibration or SFU comparison result.
`comparison_available` and `pcm_latency_calibrated` always remain `false`,
including successful structural/interval qualification. Unit-test packets and
runtime records are synthetic contract fixtures, not playable/decoded Opus or
executed controls. Native V2 reader/pacer review and actual0/50/200/500-ms
controls remain separate gates. A passing guard test does not qualify a reader
binary, its actual replay scheduling or received browser PCM.

## Archive and whole-pair inputs

Call `readReplayPair({mic:{path,sha256},source:{path,sha256},provenance})` using
independently frozen artifact/tool hashes. The six required provenance hashes
are `library_sha256`, `script_sha256` (PN builder),
`fixed_opus_script_sha256`, `pcm_kernel_sha256`, `pn_inspector_sha256`,
`node_sha256`. The Node hash must match the pinned26.8.2 marker inspector.
Each expected archive SHA is checked against the actual file bytes. Files/header
JSON have bounded sizes; duplicate keys, including escaped key aliases, fail.

The guard checks complete320-byte packet framing, monotonic20-ms due times,
payload SHA, exact measurement20..360 whole seconds plus1-second tail,
48kHz/960-frame mono/CBR128k policy, retained312-sample lookahead, source gain
(Mic four tones each.07; Source440Hz.45; PN.35), PCM/decoded hash claims,
nonclipping/correlation claims and every offline marker/control record. It
recomputes both complete PN codebooks and their canonical shared SHA. Both
archives must have the same canonical run UUID, duration and full shared book.
The UUID/SHA are provenance bindings; PN waveform uniqueness is within this
finite run, not across all runs or an absolute clock accuracy guarantee.

The JS guard does **not decode Opus or regenerate PCM**. The independently
frozen native reader must actually regenerate the declared source, decode the
complete archive with the mapped libopus1.6.1, verify hashes/lookahead/markers
including the tail and only then emit `import_verified:true`. Native binary and
mapped decoder hashes must match the executed process provenance measured by
the runner. A hand-constructed greeting/JSON record cannot authenticate a
process or replace that actual reader/control evidence. This API validates the
captured evidence contract, and does not grant trust to arbitrary JSON.

The resulting pair and nested metadata/book are frozen. An internal import
identity prevents a caller's cloned/plain pair object bypassing the real
file/hash/header path. `replayRequest(pair,hold)` produces exactly:

```json
{"op":"start","peer":"publish","total_seconds":241,"audio_hold_ms":200}
```

`total_seconds` is the actual archive `duration_seconds`, including tail;
measurement240s thus means total241s. The reader rejects V1 `seconds`, looping,
rewinding, partial end/truncation and other hold values. The test flag is
`--allow-test-audio-hold`, **before `--peer0`** in the native CLI. The old
Clock13 bridge's future replay CLI arrangement is not used for these new V2
controls; a separate V2 adapter must use this actual parser order.

## Native evidence fields agreed with reader owner

`validateReplayRuntime(pair,runtime)` expects:

- `runtime.nativeBinarySha256`: independently measured immutable executed SHA;
  `runtime.audioHoldMs`: planned control from0/50/200/500.
- `runtime.greeting`: `ready:true, provenance:{binary_sha256,mic,source,decoder,
  comparison_available:false,pcm_latency_calibrated:false}`. Mic/source each
  contain `archive_sha256,metadata,import_verified:true`, bound exactly to actual
  imported bytes. Decoder contains `version:'libopus 1.6.1',sha256` matching
  the frozen mapped library. The native optional `archives`/decoder aliases
  are unnecessary for this guard.
- `runtime.start` and `runtime.status`: `measurement_seconds,total_seconds,
  measurement_end_sample_ordinal,tail_samples,test_hold_enabled,audio_hold_ms,
  comparison_available:false,pcm_latency_calibrated:false,timeline`.
- `timeline`: `clock:'CLOCK_MONOTONIC',startClockNs` as canonical decimal ns,
  `conversionBracketNs` as an integer in0..100000, and
  `pcm_latency_calibrated:false`. Start, status and both audio sources must
  contain the identical common three-track anchor.
- `status.sources.mic/source`: exact `source_uid,ssrc,expected_packet_count,
  packets_enqueued,last_source_sample_ordinal` plus `completed:true,running:false,
  source_policy_valid:true,end_reached:true`. Last ordinal must be
  `(packetCount−1)*960`, never a wrapped10-second position. Video may be present
  in the status, but this audio guard does not qualify its contents.
- Per audio source: `last_planned_mono_ns,last_enqueue_before_ns,
  last_enqueued_mono_ns,last_enqueue_bracket_ns,max_schedule_lateness_ns,
  min_actual_hold_ns,max_actual_hold_ns` are canonical decimal strings;
  `test_hold_enabled:true,audio_hold_ms,hold_applied_packets` are actual runtime
  records. `hold_applied_packets` must equal all archive packets.

Hold extrema come from actual enqueues against the **unshifted** source plan:
minimum is min(before−originalDue), maximum is max(after−originalDue).
Lateness is max(after−(originalDue+hold)), so `maxActualHold=hold+maxLateness`.
The source anchor stays fixed; the hold is not secretly subtracted from latency.
Lateness must remain within the existing20-ms enqueue budget. A last enqueue
before its plan, reversed/bracket-mismatched time, fictitious extrema or
incomplete replay rejects qualification. Minimum actual hold may fall below
the requested hold only within the measured Instant/CLOCK_MONOTONIC conversion
bracket. These actual scheduling extrema do not claim exactly the requested
hold duration. V2 audio finish waits through total+hold and complete tail;
video finishes at its original total. Prefix status is never complete evidence.

## Received PCM and qualification

`qualifyReplayPair({pair,runtime,groups,observer,contextStates,browserClock})`
requires both distinct groups. Each contains `role,uid,archive_sha256,run_id,
codebook_sha256,tap,receiver`, using the captured source-to-receiver binding.
Their actual held receiver UID/SSRC must be Mic0/0x474d4943 and
Source64/0x47534130, live/enabled with stable identity, real decoded/packet
progress and available zero lost/concealed/silent-concealed sample counters.
The initial and final `initialInboundReportId`/`inboundReportId` must be the
same nonempty actual inbound RTP report ID, with distinct IDs for the two held
receivers. The collector must keep the same actual `RTCRtpReceiver`, live and
enabled track, source UID/SSRC and report throughout the run; `identityStable`
records that observation. A claimed stable ID does not authenticate arbitrary
JSON or establish a held receiver by itself.

Capture `initialPacketsReceived:0` and `initialTotalSamplesReceived:0` from
that report **before** the finite replay starts. Final `packetsReceived` must
equal all archive packets and `totalSamplesReceived` all offline decoded
samples, including the one-second tail. Prefixes, larger totals, nonzero initial
counts and unavailable/malformed counters fail. `totalSamplesReceived` includes
concealment, so these totals cannot replace actual available zero
`packetsLost`, `concealedSamples`, `silentConcealedSamples` and
`packetsDiscarded`. Their corresponding `initialPacketsLost`,
`initialConcealedSamples`, `initialSilentConcealedSamples` and
`initialPacketsDiscarded` must be available and zero in the same initial report.
Source `completed`/`end_reached` only proves enqueues and
cannot replace this whole received-stream evidence.

`insertedSamplesForDeceleration` and `removedSamplesForAcceleration` must also
be actually available and zero, together with
`initialInsertedSamplesForDeceleration` and
`initialRemovedSamplesForAcceleration`. Their totals do not locate individual inserted
or removed samples relative to PN peaks. Even balanced nonzero totals leave the
PCM ordinal mapping unknown, so this guard rejects them until an actual
per-sample mapping exists. The proposed receiver fields are exercised by
synthetic fixtures here; a future V2 browser collector must measure them from
the held report. Missing browser statistics never default to zero.
If the browser cannot expose a genuine zero pre-replay snapshot of that report,
this whole-stream contract remains unqualified; a manufactured initial record
or subtracting warmup packets cannot replace it.

The browser must be the frozen Chromium153 binary/revision with the verified
COI timer precision; Node must be the frozen26.8.2 binary. The caller's
`browserClock` contains the existing precision/revision evidence plus actual
`chromium_sha256,node_sha256` captured by the runner.

The guard supplies the entire derived book itself. Every expected marker must
appear once; missing/duplicate/wrong-UID markers, clipped/nonfinite PCM, ring
loss, decoder PLC/loss, suspension, missing clock brackets or callback intervals
over25ms reject the whole pair and clear all partial intervals. It retains the
existing conservative causal clock/callback intervals and96-sample offline
marker-position tolerance, not an old±2-ms absolute accuracy claim.

Whole-stream callback coverage is derived from **every genuine matched PN
peak**, the immutable source ordinal and the retained actual codec lookahead:

```text
startEstimate = peak.receivedFrame - marker.source_sample_ordinal - lookahead_samples
requiredFirstFrame = max(0, min(startEstimate) - 96)
requiredEndFrameExclusive = max(startEstimate) + 96 + decode_control.samples
```

All peak-derived start uncertainties must share an intersection:
`max(startEstimate) - min(startEstimate) <= 192`. With no insertion/removal,
peaks that cannot describe the same decoded ordinal mapping invalidate the
whole pair, even if callback rows cover their broader union. The coverage range
above conservatively retains that full union once the mapping is consistent.

The decoded sample count already includes the codec delay within the finite
archive; lookahead is subtracted when mapping the detected PN peak to decoded
frame zero. Callback rows must cover every frame from `requiredFirstFrame`
through `requiredEndFrameExclusive - 1`, including the decoded beginning,
spaces between markers and the entire received tail. No invented end marker,
caller-provided shortened end, extrapolated frame or source completion flag is
counted as actual input. Negative estimated bounds may only be clipped at the
actual AudioContext frame-zero boundary; an estimate whose full uncertainty
precedes frame zero fails.

Every intersecting callback must retain consecutive safe frame/sequence
ordinals, zero actual flags, complete `inputFrames === frames` and ordered
physical observer times. The existing causal native clock probes must bracket
each such callback within25ms, including callbacks after the last PN marker.
The guard searches those validated probes without fitting or extrapolating a
clock. The successful group exposes `receiverTail` with derived frame bounds,
observed coverage, complete receiver totals, held report ID and callback count.
These fields describe validated captured evidence, with calibration and
comparison still false.

This contract is for an owned native-to-browser **direct-loopback instrument**.
Full-N engines with rewritten receiver SSRCs need explicit producer/consumer
edge mapping and independently checked graph coverage; the raw source SSRC
check must not be bypassed to rank those engines. Browser-to-native decode and
physical target devices are also separate gates. All roles/archives/PN policy,
actual reader/decoder/browser binaries must stay identical across future engine
cases before any comparison can be considered.

Run the focused offline checks with the pinned Node:

```sh
/tmp/gelabber-v04-toolchain/node-v26.8.2-linux-x64/bin/node --test media/bench/tests/nativePcmReplay.test.mjs
```
