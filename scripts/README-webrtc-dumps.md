# WebRTC internals dump diagnostics

Measurement-only tooling for Gelabber voice quality triage (issue #53 and similar).
**Report measured facts only. Do not change product audio / SDP / capture code without dump data.**

## Export a dump (Chrome)

1. Open `chrome://webrtc-internals` **before** joining the voice channel (the page only observes peer connections created after it loads).
2. Join / reproduce the call.
3. On the internals page, use **Download the PeerConnection updates and stats data** (wording may be “Create Dump” / download webrtc-internals dump depending on Chrome version).
4. Save the JSON file.

## Run the parser

From the repo root:

```bash
python3 scripts/parse-webrtc-dump.py path/to/webrtc_internals_dump.json
```

Smoke test (synthetic fixture, no real call required):

```bash
python3 scripts/parse-webrtc-dump.py scripts/fixtures/minimal-webrtc-dump.json
```

Help:

```bash
python3 scripts/parse-webrtc-dump.py --help
```

Stdlib only (Python 3). No npm/pip deps.

## What it reports

For each PeerConnection in the dump, when present on the relevant stats:

| Field | Source (typical) |
|---|---|
| Active codec (`mimeType` + `sdpFmtpLine`) | `codec` linked from `outbound-rtp` / `inbound-rtp` via `codecId` |
| Bitrate (kbps) | Δ`bytesSent` / Δ`bytesReceived` over consecutive samples (or span). If only one snapshot exists: **no invented rate** — prints cumulative bytes instead |
| `packetsLost` | `inbound-rtp` / `remote-inbound-rtp` |
| `jitter` (seconds) | `inbound-rtp` / `remote-inbound-rtp` |
| Concealment | `concealedSamples`, `silentConcealedSamples`, `concealmentEvents` on inbound audio RTP when present |
| Levels / energy | `audioLevel`, `totalAudioEnergy`, `totalSamplesDuration` on `media-source` / `track` / RTP when present |

Missing fields print as `N/A`. The parser does not guess root causes or suggest product fixes.

## Related client code (read-only context)

`web/src/voice/session.ts` drives `RTCPeerConnection` but does **not** call `getStats()` or log bitrate / loss / concealment / audioLevel. Use this dump workflow for measurements until in-app stats exist.
