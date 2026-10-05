# Capture restart and current receiver ownership

The restart fix is `06acba87b12a3518010803244d6d4d910f119adb`; its independent native fixture is `36fde19e830b0b76c7d5dc81ed79effd8b8f08cc`. Test correction `956131763c524a85974833a806e76722d1525865` asserts zero receiver seeds before source authorization and two after authorization. Runtime behavior is unchanged by that correction.

## Reproduce locally

Use the repository's locked Rust, Node and Playwright versions. Install the locked web dependencies and both Playwright browser engines. This opt-in test starts an owned in-process SFU on an ephemeral loopback port; no API, Redis or outside calls are involved.

```sh
GELABBER_LAYER_LIFECYCLE_ONLY=1 cargo test --locked -p gelabber-media sfu::layer_native_tests::native_browser_rid_feasibility -- --ignored --exact --nocapture
```

The default is twenty fresh capture cycles per direction, rotating Camera/Screen/Live. `GELABBER_LAYER_CAPTURE_CYCLES` accepts 3–100. The fixture uses actual canvas capture, a tone microphone, native peer connections, the product layered sender/MSID helper, and real RTP/decoder counters. It models the source lifecycle through the real SFU announcement, retraction, offer/answer and Watch operations. It does not run the full app session controller.

Every stop must end the old native capture, remove its SFU publication and remove its recovery scope. Every restart requires a distinct fresh captured track and an explicitly authorized new publication on the same publisher MID. Receiver acceptance matches the current SFU subscriber SSRC and requires at least five new decoded frames/packets over the previous matching-SSRC sample, plus progressing microphone audio. Publisher and subscriber native/SFU transceiver counts must remain unchanged throughout the run. Old decoder totals cannot satisfy the gate.

Two negative controls identify separate failure mechanisms:

```sh
# Keep ordinary header recovery enabled but omit current bound-receiver recovery.
GELABBER_LAYER_LIFECYCLE_ONLY=1 GELABBER_TEST_DISABLE_BOUND_RID_BINDING=1 cargo test --locked -p gelabber-media sfu::layer_native_tests::native_browser_rid_feasibility -- --ignored --exact --nocapture

# Reproduce Firefox's inactive removeTrack negotiation narrowing its q/f envelope.
GELABBER_LAYER_LIFECYCLE_ONLY=1 GELABBER_LAYER_STOP_STRATEGY=remove cargo test --locked -p gelabber-media sfu::layer_native_tests::native_browser_rid_feasibility -- --ignored --exact --nocapture
```

These controls are expected to fail functionally; they are not positive release gates. The first control was observed to lose the newly published matching decoder while microphone packets continued, rather than merely failing a recovery-counter assertion. The second exposed an offer without the prior two RID encodings and a codec conflict with the historical remote-track wrapper.

## Validated scope

The locked unchanged RTC 0.20.5 core passed twenty capture cycles each way (Chromium→Firefox and Firefox→Chromium), comprising seven Camera, seven Screen and six Live restarts per direction, in 26.34 seconds. Publisher MIDs remained 0/1/2 with the microphone at 3. Each stopped source reduced recovery to two scopes/four primary SSRCs; all restarted sources restored three scopes/six primary SSRCs with fresh decoded frames and continuing audio. The separate original layer comparison also passed initial three-source publication, small/full switching, legacy v2 default and explicit VP9 coexistence, and twenty foreign-source Watch cycles per direction in 107.02 seconds. Capture restart and Watch cycles are distinct scopes.

The final corrected source passed 103 media tests (74 library and 29 integration; the native fixture remains opt-in), 464 web tests, media Clippy with all targets and warnings denied, web compilation/build, and lint. Added fast tests cover unauthorized/stopped/replayed SDP, exact source and primary encoding contracts, pending recovery, replacement publication generations, Stop/Done/Close/Live expiry, conflicting/repair headers, a delayed same-source detach, other-source/event/ICE offers during detach, rejection/timeout retirement, old peer completions, and both separate source-audio senders.

Cargo.lock SHA-256 remained `2ea89bb21b628640d344d4c31611e46f2b73ee04d7443adf3d4133eceae15b80`; the unchanged Cargo-registry RTC `peer_connection/internal.rs` SHA-256 remained `1cbbadf77da0c03e86794e9959596537e095f5c896657a376d30171d1527082d`. No external core-patch experiment was active.

## App-controller recovery follow-up

The original in-process restart fixture did not cover the complete app-controller handshake. Integrated app controls on an earlier candidate reproduced Gateway-only and rejected-answer failures while microphone audio continued. `5bbadef` binds source cleanup and paired audio to the current source grant, and late subscriber detaches to the exact publication life. It prevents an old same-ID publication/read task from cancelling a fresh authenticated announcement. Eight deterministic SFU generation tests cover overlapping grants, paired audio, source retagging and pending/active subscription replacement. `3dbac3e` preserves the browser sender through rejected-offer cleanup and checks the originally negotiated q/f envelope before reuse, including Stop before rollback. The corrected stand passed 152 session tests, 467 web tests, build/lint and media Clippy.

Another Firefox control reached 34 successful Live captures before a registered full representation stopped producing complete frames at capture 35. Complete low keyframes were present; selecting only the registered full SSRC left the viewer without video. `a5c335e` adds bounded complete-frame availability fallback while retaining the viewer's full preference and requiring a full keyframe for upgrade. Thirteen layer tests and the full 114-test media suite passed. Three new regressions cover padding-only full, later full-keyframe upgrade, healthy full deltas versus lost references, and isolated new-publication availability.

Against the independently reviewed follow-up, two isolated full-app Firefox runs each passed 40 new Live captures and a later watcher with the unchanged 640px, correct-source and changing-frame assertions. Both kept one microphone acquisition and at most two publisher transceivers. The second run waited for the actual current capture/sender before its native stats baseline: each of its 40 windows had 6–13 new full-layer encoded frames. Unsupported native bandwidth fields were recorded as unavailable; these counters do not constitute a bandwidth estimate. A separate bounded test-only SFU transcript observed current complete full frames and publisher PLI requests; browser decoding remains the wire-delivery evidence. A small fallback image does not satisfy the 640px gate.

The local full-app Watch helper now waits for the specific source figure and mounted Watch control. It keeps the original first-frame deadline and selects the expected publisher during rejected-answer retry. An unrelated screen Stop control cannot count as selected Live. This addresses a reproduced helper no-op, rather than removing the native rejection or weakening image acceptance.

Gateway-only and malformed native-answer recovery passed in both Chromium and Firefox against this unchanged follow-up. The rejected-answer control retained progressing Camera/Screen/microphone media, rejected the attempted Live publication without a ghost, and then decoded the retried selected Live source at full fixture resolution. The local test-only snapshot additionally recorded the actual selected-owner Live Watch message and an alive current SFU Live subscription. The production SFU has no diagnostic endpoint. These are isolated local app controls; the full integrated suite and hour call below still require their own final-runtime evidence.

## Open release gates

The app's gateway recovery, rejected-answer and Live-cycle controls must be repeated against the integrated final runtime. A dedicated final-runtime one-hour call, physical devices/mobile behavior, actual microphone/video quality and TURN/WAN adaptation remain separate gates. Earlier successful hour calls used an older runtime and do not validate this restart fix. These controlled counters prove loopback RTP delivery and decoding only.
