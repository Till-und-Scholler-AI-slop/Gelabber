# Load tests

Scenarios that put a running Gelabber under load and a sampler that records
how much memory and CPU each process uses meanwhile. They measure; they do
not pass or fail. The first results (v0.11.0, 4 vCPU) are in issue #610; rerun
the same scenarios before and after a change to compare.

| File                                      | What                                                                                                                                                                                          |
| ----------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sampler.py`                              | Once per second: RSS, PSS and private memory plus CPU time of the API, the media gateway, Postgres, Redis, the desktop app and every browser a scenario labels, tagged with the current phase |
| `chat.mjs`                                | Steps of connected users in one channel; per step idle, a message burst, then waiting for the deliveries                                                                                      |
| `connections.mjs`                         | Open WebSockets only, from users signed in beforehand                                                                                                                                         |
| `call.mjs`                                | The real web client in Chromium joins one voice channel step by step, with fake microphone and, for video, fake camera                                                                        |
| `summarize.py`                            | Averages per phase from the sampler's file                                                                                                                                                    |
| `client.mjs`, `phases.mjs`, `browser.mjs` | Shared code: API/gateway client without a browser, phase labels, Chromium launch                                                                                                              |

## Setup

Linux, Node from `engines`, Python 3, and the stack running locally:

- API from a release build (`cargo build --release -p gelabber-api` in
  `backend/`), with the test services (`.claude/skills/test-services/`). Turn
  the rate limits off for the API process, or the scenarios measure the
  limiter: `API_RATE_AUTH_PER_MIN=0 API_RATE_API_PER_MIN=0
API_RATE_MSG_PER_MIN=0 API_RATE_LOGIN_FAILURES_PER_ACCOUNT=0
API_LINK_PREVIEWS=false`.
- Media gateway for `call.mjs`: a release build or the published image, e.g.
  `docker run --network host -e MEDIA_ADDR=127.0.0.1:8081 -e REDIS_URL=redis://127.0.0.1:6379 -e MEDIA_ICE_BIND=127.0.0.1:10000 -e MEDIA_ICE_PORT_MAX=10000 -e MEDIA_ADVERTISED_IP=127.0.0.1 ghcr.io/till-und-scholler-ai-slop/gelabber/media:<version>`.
  Its `/metrics` must be reachable (`GELABBER_LOAD_MEDIA_METRICS`).
- Web client for `call.mjs` and the chat observer: `npm run build`, then
  `npx vite preview --host 127.0.0.1 --port 4173` (proxies `/api`, `/ws` and
  `/media` like Caddy does).
- Restart the API and the media gateway before each scenario. Neither gives
  memory back to the system after a peak (#614), so a second scenario would
  start from the first one's peak.

Environment: `GELABBER_LOAD_API` (default `http://127.0.0.1:8080`),
`GELABBER_LOAD_WEB` (`http://127.0.0.1:4173`), `GELABBER_LOAD_BROWSER_EXECUTABLE`
(a Chromium other than Playwright's; by default Playwright's full Chromium in
its new headless mode, not the headless shell, so the process layout matches a
desktop Chrome), `GELABBER_LOAD_PHASE_FILE` and
`GELABBER_LOAD_LABEL_FILE` (default in the temp directory; the sampler reads
the same ones).

## Run

From `web/`, the sampler in a second shell for the whole session:

```sh
python3 scripts/load/sampler.py /tmp/samples.jsonl &

# Chat: 20 messages per second, 60 s per step; OBSERVER=1 adds a measured web client.
STEPS=100,250,500,1000 RATE=20 BURST_S=60 IDLE_S=30 OBSERVER=1 npm run test:load-chat

# Connections only: prepare, restart the API, then run.
npm run test:load-connections -- prepare
npm run test:load-connections

# Calls: an observer plus bots; MODE=video turns the cameras on.
MODE=audio STEPS=4,6,8,12 npm run test:load-call

python3 scripts/load/summarize.py /tmp/samples.jsonl api media postgres redis chrome:web-chat
```

Each scenario appends its results (`chat-results.jsonl`,
`connection-results.jsonl`, `call-results.jsonl`, or `RESULTS=<file>`) and
leaves its accounts and servers behind: run it against a throwaway database.

## Reading the numbers

- **RSS** counts every page a process has mapped, shared libraries included.
  Fine for a single process like the API.
- **PSS** splits shared pages between the processes that map them. Use it for
  Postgres (shared buffers would otherwise count once per backend) and for
  browsers (every Chromium process maps the same binary). A browser's PSS
  drops when other Chromium instances run beside it, so measure a client
  alone, or add its private memory (USS) to its shared part measured alone.
- **cores** in `summarize.py`: CPU time per second over the phase; 1.0 is
  one fully used core.
- The chat result's `onTime` counts deliveries within `ON_TIME_MS` (3 s) of
  sending; `deliveredPerSecond` is the gateway's real throughput.
  `closedByServer` counts sockets the server closed during the burst, for
  example because their outbound queue overflowed. The load client does not
  reconnect, so `expected` still counts them: when it is above zero, part of
  the gap between `expected` and `received` is closed sockets, not slow
  delivery.
- Everything on one machine shares its CPUs: the load generator and the
  browsers compete with the server. Large calls end on the machine's CPU long
  before the media gateway's memory matters. Say which numbers were limited
  by the machine when you report them.
