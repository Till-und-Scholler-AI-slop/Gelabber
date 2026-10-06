# Native PCM V2 short control evidence, 2026-10-06

The actual 0ms retry is **BLOCKED at the genuine zero initial Mic receiver
report**. The held Mic receiver's real `getStats()` table repeatedly returned
`[]`, so its inbound report and packet/sample/loss/PLC/discard/stretch counters
are unavailable. No value was replaced with zero. Sequential collection stopped
there; Source receiver and whole PeerConnection tables are **NOT_COLLECTED**,
rather than claimed unavailable. There is no actual received PCM, PN/tail or
latency result. Both comparison/calibration flags remain false.

The retry passed real CDP provenance: `HeadlessChrome/153.0.8010.12`, revision
`@971a7443b0c9b0a9b2860529b33331b76077ec62`, executed SHA
`ded93a9c9a53a1ae040f08124badcca95c938e9d5015ff340c3b5538c41bf39e`.
A passive `/proc` capture matched that newly owned browser PID, parent,
start-time ticks, detached group/session and executable hash. Its actual command
line contains `--enable-automation`. Chromium exposed one space-joined command
line entry, so the monitor's exact-NUL-element flag boolean was false; separate
token-boundary analysis records the correct presence without changing the raw
capture. Successful actual CDP command-line retrieval independently verifies the
runtime behavior. No additional browser was started for this diagnostic.

The complete native V2 greeting/import and mapped executable/libopus checks
passed. Native pre-start status retained a connected publisher and zero sources.
Actual RPC inventory was `status:2`, `create:1`, `offer:1`, `remote:1`,
`clock:3135`, **`start:0`**. No finite replay started and no common native source
start anchor exists. AudioContext, observer worker, receive nodes, PeerConnection
and video element were genuinely created during preparation, then disposed.
The context and PeerConnection closed, worker terminated, nodes disconnected
and video detached, with no disposal errors. The browser exited normally with
code 0/no signal and disconnected gracefully without force; HTTP closed and
native exited normally with code 0/no signal. Actual created-resource cleanup
was complete. It cannot qualify replay that never started.

The first actual 0ms attempt remains preserved as a separate earlier failure:
CDP `Browser.getBrowserCommandLine` failed because `--enable-automation` was
absent, before `prepareV2Replay` was invoked. Its actual native inventory was
`status:1`, `start:0`, with zero peers/sources. Initial receiver statistics were
**NOT_COLLECTED**; page, PeerConnection, context, worker and audio nodes were
**NOT_CREATED**. Native, browser and HTTP resources closed normally. The current
whole-control predicate additionally marked cleanup unqualified at that early
stage; the evidence does not invent closed audio resources. The installed
Playwright Chromium switch list and original explicit args omitted the flag.
The reviewed follow-up changed only that flag in the new owned V2 launcher;
the full first-run command line could not be retrieved by CDP.

For each attempt, all recorded helper hashes after execution matched the
original frozen hashes. The retry also matched the approved seven source hashes
before/after; Clock13 and Tail3 remain unchanged. Private reports retain real
signaling and available statistics with directory 0700/report 0600. The adjacent
JSON exports selected facts, the actual empty Mic table and report/summary hashes,
excluding tokens, SDP, command-line paths and process identities. Private
command-line capture/analysis is also 0600. Both raw failed reports are retained.

50/200/500ms holds and ring/clock/suspension/PLC/stretch failure controls remain
**NOT_RUN/BLOCKED** at the same initial-report requirement. Repeating them before
finite source start would provide no hold or replay behavior evidence. None was
run. The hour-long test is excluded by the user. Synthetic unit tests and native
archive integrity checks remain separate evidence; they do not establish actual
browser PCM acceptance.
