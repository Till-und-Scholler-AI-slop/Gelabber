# Smoke packet-loss boundary

The original whole-eth0 netem loss also delayed Redis TCP authority checks. In PR121
CI36431643110/job108959039292, initial relay/video/audio passed, then authority
checks hit the 500 ms deadline and fail-closed teardown removed audio. This was a
combined media/control fault, not a valid isolated media-loss acceptance result.

The smoke now reads the actual selected SFU remote ICE UDP ports from both native
browser peers. A temporary PRIO root maps all priorities to an unshaped band;
exact IPv4 UDP source-port filters alone enter its 8% netem child. TCP (including
Redis and signaling), DNS and unrelated UDP never match. The Compose IPv4 path
is asserted explicitly. A missing selected port or non-UDP path fails.

The owned child must report a positive drop delta. Root counters do not qualify.
`qdisc add` refuses an existing root; no replacement is attempted. Once created,
the owned root is removed in `finally`, including setup and assertion failures.
The relay, bidirectional Opus/FEC packet progress, post-loss recovery, camera,
screen budget and byte-exact JPEG/PNG/WebP assertions are retained. Product
fail-closed behavior and its deadlines are unchanged.

Run deterministic boundary checks with the pinned toolchain:

```sh
node --test web/scripts/smoke-netem.checks.mjs
```

A separate kernel control creates a disposable user/network namespace. It must
never run against a shared/host namespace; the script checks the launching
namespace ID before changing `lo`:

```sh
GELABBER_NETEM_HOST_NS="$(readlink /proc/self/ns/net)" unshare -Urn sh -c \
  'ip link set lo up; node web/scripts/smoke-netem.namespace.mjs'
```

Observed local control: 1000 UDP sends, 910 receives, 90 owned-child drops;
100/100 TCP echoes on the same numerical source port, maximum 1 ms (<500 ms).
Focused checks and lint PASS. The existing local browser smoke on control
Web5175/API8082/SFU8081 PASS, including native selected-port extraction, forced
relay audio, camera/screen and uploads. Its packet-loss segment remains explicitly
unexecuted because no Compose media container PID is available locally.

**BLOCKED:** complete SFU/coturn/browser packet-loss and recovery acceptance in
CI with this commit. No remote push/run is authorized. The namespace control
proves the kernel fault boundary, not audible speech quality or SFU acceptance.
