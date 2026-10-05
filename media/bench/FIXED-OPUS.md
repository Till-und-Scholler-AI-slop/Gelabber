# Fixed native Opus audio fixture

`opus-fixture.py` prepares offline test data for native peer0's microphone and
its separate screen-source audio. It uses the installed libopus C API through
Python ctypes, without changing pinned product dependencies or starting a
server. It records the actual mapped library, Python executable and script
hashes, library version and every configured encoder setting read back from
the encoder. Use the same frozen archives for all media engines.

```sh
python media/bench/opus-fixture.py build mic /tmp/mic.opusbin --output /tmp/mic.json --pcm /tmp/mic.f32
python media/bench/opus-fixture.py build source /tmp/source.opusbin --output /tmp/source.json --pcm /tmp/source.f32
python media/bench/opus-fixture.py inspect /tmp/mic.opusbin
python -m unittest discover -s media/bench/tests -p test_opus_fixture.py
```

Both fixtures have exactly 500 Opus packets of 20 ms at 48 kHz mono. Every
packet is checked with `opus_packet_get_nb_samples`, then decoded by a fresh
decoder. Actual TOC channel count must be one; a mono decoder silently downmixing
stereo packets cannot qualify. The importer regenerates the named fixed PCM
waveform, checks its hash/settings, queries the configured codec lookahead and
checks decoded samples, hash, peak and correlation against the real decoder.
It refuses altered producer labels or a decoder/platform whose exact float
control differs; generate and independently inspect a new fixed artifact for
that environment. CBR is 128 kbit/s: each actual Opus payload is 320 bytes, with no RTP
headers, padding or transport overhead counted. Application is `audio`, VBR
and DTX are off, complexity is 10, inband FEC is configured on and expected
loss is 1%. Configuring FEC does not prove an individual packet contains FEC.
See the [official encoder controls](https://opus-codec.org/docs/opus_api-1.6/group__opus__encoderctls.html).

The microphone input has 317/719/1249/2027 Hz sine tones, each gain 0.07, as
browser peer0 uses. The separate source input is 440 Hz at gain 1.0, matching
the existing browser fixture without PCM markers. Generated float32 PCM and
decoded PCM have separate hashes; a lookahead-aligned correlation checks that
the encoded audio retains the input waveform. Decoded full-scale sine peaks
may exceed 1.0 slightly; the tool does not clamp the decoder output.

Archive layout is `GPOPUS1\n`, a big-endian u32 UTF-8 JSON header length, the
JSON header, then 500 records. Each record contains a big-endian u64 nanosecond
deadline relative to clip start, a big-endian u32 payload length and one Opus
payload. Deadlines are 0, 20,000,000, ..., 9,980,000,000 ns. The future native
sender must use a 48 kHz RTP clock, advance timestamp by 960 and sequence by
one per packet, and continue both across loops. Microphone and source audio
must have independent tracks, SSRCs, counters and deadlines.

These are ten seconds of encoded frames without container pre-skip or a flush
tail. Actual codec lookahead is queried and recorded; it remains in replay.
Reusing the frozen bytes is not a sample-exact continuous oscillator loop.
No browser/native sample-clock calibration, acoustic delay, WAN latency,
subscriber graph, resource comparison or replacement acceptance follows from
this offline fixture. A complete native peer must also receive and decode the
other voice peers before the full topology can qualify.
