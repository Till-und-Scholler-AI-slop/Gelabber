# Unlooped PN Opus sources

`pn-opus-fixture.py` creates and strictly imports two offline inputs for the
native measurement fixture. Use exactly the same pair of archive bytes for
every SFU. This is instrument integrity checking, **not** native/browser clock
calibration, acoustic latency, or an end-to-end PCM acceptance result.

The instrument requires actual mapped **libopus 1.6.1** and Node 26.8.2. It calls
the public libopus C API through the existing `opus-fixture.py` binding without
altering that file or any RTC/stack pin. Version, mapped library hash, executed
Python/Node hashes, script dependencies and PCM kernel/inspector hashes are
recorded. Encoder CTLs are queried back: mono 48 kHz, audio application 2049,
20 ms / 960 samples, CBR 128 kbit/s / 320 bytes, VBR off, DTX off, complexity 10,
FEC config 1 and 1% configured loss. Actual FEC presence in a packet is not asserted.

Create fresh outputs with **one shared canonical UUID** for both inputs:

```sh
python3 media/bench/pn-opus-fixture.py --library /absolute/libopus.so.0 \
  --node /absolute/node-v26.8.2/bin/node build mic /tmp/run-mic.opusbin \
  --seconds 120 --run-id 2d616d44-906e-434d-94b8-553d8d903ee6 \
  --output /tmp/run-mic.json
python3 media/bench/pn-opus-fixture.py --library /absolute/libopus.so.0 \
  --node /absolute/node-v26.8.2/bin/node build source /tmp/run-source.opusbin \
  --seconds 120 --run-id 2d616d44-906e-434d-94b8-553d8d903ee6 \
  --output /tmp/run-source.json
```

`inspect mic ARCHIVE` and `inspect source ARCHIVE` use the same leading library
and Node options. They regenerate the expected source PCM/codebook, decode
every packet and run the actual PCM marker detector. Output paths must be fresh
and distinct. `--pcm FILE` additionally retains generated float32le source PCM.
Archives remain in the run evidence directory; do not commit the large inputs.

The V2 reader contract is separate from the existing ten-second V1 input:

| Field | Contract |
| --- | --- |
| Framing | Eight-byte `GPOPUS2\n`, BEu32 JSON length, JSON, repeated BEu64 dueNs/BEu32 packet size/Opus bytes |
| Bounds | At most 8 MiB per archive, 256 KiB JSON header; fixed 320-byte Opus payloads and at most 18,050 packets |
| Policy | `schema:2`, `codec:"opus"`, `channels:1`, `sample_rate_hz:48000`, `packets:(measurement_seconds+1)*50` |
| Duration | `measurement_seconds` is integer20..360; `duration_seconds` includes the explicit one-second tail |
| End | `measurement_end_sample_ordinal=measurement_seconds*48000`; `tail_samples=48000`; archive end is exact, never modulo/looped |
| Packet time | Recordordinal×20,000,000ns; timestamp/sampleordinal=recordordinal×960; time denotes the **first stored source PCM sample** of that preencoded packet |
| Source binding | Mic UID0/SSRC`0x474d4943`; separate source audio UID64/SSRC`0x47534130`; compare the actual received role, source identity and SSRC binding, not a sender label alone |
| Shared run | Top-level `run_id`, and `pn.shared_run`: UUID, end/tail, marker policy and both role codebooks; both archives have the same `pn.codebook_sha256` |
| Markers | First ordinal48000, then+96000; complete63×96sample marker lies before measurement end; sequence0..N with no repeated or sign-inverted code across either UID within the run |
| Gains | Mic317/719/1249/2027Hz each.07; source440Hz.45; PNcarrier2000Hz/amplitude.35; unclipped float32le mono |
| Integrity | Hashes of original PCM, complete decoded PCM, packet concatenation, shared codebook and archive bytes; actual per-marker decoded position/score/amplitude |
| Qualification | `pcm_latency_calibrated:false` and `comparison_available:false`; reader must require actual JSON booleans |

The shared codebook hash is SHA256 of `pn.shared_run` serialized as sorted-key
JSON with separators`,` and`:` and no whitespace/newline. The hash includes the
run UUID and both UID/SSRC contracts. It is identical in both archives. Codes
are the unchanged `pcm-kernel.mjs` xorshift32 `markerCode(uid, sequence)`.
Uniqueness means only the finite expected codebook within this run; other runs
reuse those deterministic codes. The UUID/hash is a provenance fence and does
not make the waveform globally unique.

Source regeneration first rounds the tone sum to float32, adds the double
precision PN sample, then rounds back to float32. Keep this two-step rounding
in a native reader that verifies the source hash. Python's float summation
behavior/version is retained in provenance; the existing native V1 reader
already uses compensated summation to match the source binding.

The actual configured codec lookahead is retained in the output. Every marker
must appear exactly once at sourceordinal+lookahead within96samples in this
**offline** inspection. The96sample tolerance is an import rejection limit,
not a claimed wall-clock error bound. The complete one-second tail is decoded;
all markers are verified, and no sample-exact final codec flush is claimed.
Missing/duplicate markers, foreign UID, forged manifest, nonfinite/clipped
samples, wrong channels, altered bytes/deadlines and provenance mismatches fail.

Native V2 replay must send the full archive **once**, including tail, and stop
at its end. It must extend video through any partial final ten-second period,
use the shared CLOCK_MONOTONIC start anchor plus sampleordinal/48000, and retain
the actual before/after enqueue bracket. This input excludes capture, live
encoding/DSP waiting, output-device and acoustic latency. Native clock→browser
performance→AudioWorklet frame calibration and delayed replay controls remain
separate gates; the older browser-source calibration cannot qualify this path.

Run the instrument checks locally with the intended binaries:

```sh
GELABBER_PN_OPUS_LIBRARY=/absolute/libopus.so.0 \
GELABBER_PN_NODE=/absolute/node-v26.8.2/bin/node \
  python3 -m unittest discover -s media/bench/tests -p test_pn_opus_fixture.py -v
```

The tests encode/decode actual Opus for both roles and negative marker cases.
They do not start a native peer, browser, server or network listener.
