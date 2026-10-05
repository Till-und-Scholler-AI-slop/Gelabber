# Finite native peer0 PN replay

The separate V2 importer accepts `GPOPUS2\n` whole-run UID0/64 PN archives from
`pn-opus-fixture.py`. It independently regenerates both complete codebooks and
the source float32 waveform, decodes every actual mono Opus packet, checks the
actual mapped libopus1.6.1 and frozen producer/kernel/helper hashes, and repeats
the decoded marker filter. Missing, duplicated or shifted peaks fail import.
The two inputs must share their complete run UUID/codebook and duration.
Header/total limits are256KiB/8MiB; measurement20..360seconds has an additional
one-second codec tail. No V1 loop or old browser-clock calibration is reused.
`prepare-native-runtime.py --pn-seconds20` builds and strictly imports both
archives in the actual pinned Python/Node/libopus generator environment.

V2 `start` requires `total_seconds` exactly equal to the archive's total,
including its tail. Legacy `seconds` or truncation/loop requests fail. Video
reuses the same immutable clip through that common total end, including a
partial final clip period. Audio enqueues each saved packet exactly once.
Start/status expose `measurement_seconds`, `total_seconds`,
`measurement_end_sample_ordinal`, `tail_samples` and their common bracketed
timeline. Actual completion additionally requires both full packet counts,
last source ordinal, `end_reached`, normal exit and receiver-tail evidence.

For direct-loopback calibration only, the explicit CLI flag
`--allow-test-audio-hold` must precede `--peer0`. Without it even a zero-valued
`audio_hold_ms` request fails. Allowed test values are0/50/200/500ms; they shift
each audio enqueue deadline, including the last tail packet, while retaining
the original stored-PCM sample plan and video schedule. Completion waits through
total+hold. Each source records actual before/after enqueue brackets and
min/max delay against the original source plan, rather than treating the
requested delay as proof. V2 nanosecond fields are canonical decimal strings;
existing V1 duration fields retain their historical numeric format.
The actual maximum after-enqueue lateness budget remains20ms against the shifted
deadline. A failure preserves the prefix and attempted packet/frame,
planned target, actual bracket, lateness and missing count where available.
This test seam changes no product source policy and grants no PCM calibration.

```sh
GELABBER_PN_NODE=/path/to/pinned/node-v26.8.2-linux-x64/bin/node \
  python3 -m unittest discover -s media/bench/tests -p test_native_pn_import.py
# Private stdin RPC, after a successful normal native WebRTC connection:
# native-peer --allow-test-audio-hold --peer0 VIDEO MIC_V2 SOURCE_V2 BIND_IPV4
# {"op":"start","peer":"publish","total_seconds":21,"audio_hold_ms":500}
```

The importer compares actual decoded peak/score calculations with the original
Node control within1e-10 numerical error, actual source/decode float32 SHA256
exactly, correlation within1e-12 and decoded marker alignment within96samples.
All source codes and sample ordinals are recomputed, including the second role
in each complete shared codebook. The native binary includes the frozen fixture
scripts/kernel hashes; modified producer code needs a new binary/instrument
freeze. These are offline sample-integrity checks, not wall-clock calibration.
The independent replay guard in NATIVE-PCM-REPLAY.md binds actual runtime
completion to imported archive bytes; real0/50/200/500ms clock/receiver controls
remain mandatory before any native PCM qualification.
