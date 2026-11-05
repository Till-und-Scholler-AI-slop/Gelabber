# Active media UI and playback (08a)

VoiceRoom renders a remote source only for its exact active Voice server/channel,
or the exact Watch server/channel/publisher. Watch keeps its selected publisher
and channel label, accepts only identified Live video for that selection and
does not write into the Voice remote store. Other publishers' unpublish/leave
events cannot stop the selected Watch. Untagged video cannot establish identity.

AppShell owns the global Voice/Live/Watch controls across channel navigation, DMs,
profile and settings. New Live permission comes from the active Voice server;
Live stop, Leave and Stop-Watch remain available independently of that permission
or the displayed server. Logout explicitly stops both media sessions. The channel
sidebar no longer applies its own server's permission to another active session.

Rejected video playback displays a retry button. Audio rejection sets a visible
global playback state and a click retries all active playback elements. The calls
are synchronous in the click handler to retain user activation. Late play results
for detached streams cannot restore stale blocked state. Cleanup removes blocked
elements; Voice teardown retains a separate Watch session and its identity.

## Observed 2026-09-28

Node 26.8.2, Chromium 153.0.8010.12, Firefox 155.0; locked dependencies.

* 255 Web tests passed, including the exact room/publisher rendering boundary,
  active-server permission selection, permission-independent Live stop/Leave,
  separated Voice/Watch stores, wrong publisher events, blocked audio retry and
  detached promise completion. Web lint/build passed, with existing bundle and
  dynamic-import warnings. The room-binding test fails on the prior VoiceRoom.
* Actual Chromium app: Go Live/Watch with normal autoplay, SPA navigation to another
  voice channel, no stale video under that channel, global Stop-Watch and Voice
  stop/leave, plus synthetic NotAllowedError and successful visible click retry.
* Actual Chromium app: SPA navigation through settings, profile and DM retained
  the connected publisher peer and the same single display capture, kept Watch
  receive-only with zero microphone calls, and exposed working global controls.
* Actual Firefox app: Go Live/Watch, channel navigation and visible playback retry
  also passed. The SFU used the documented real-local-interface ICE adapter.

The actual app checks used a static own-worktree Web build, real API/Gateway
`3a30d44924708d9303eed778cd10d276742a5911` on 8082 and own SFU
`2485865e0c3e04dff7c789049acefc7844a440b1`. UI-created test accounts/server are
isolated. No simulated Gateway, autoplay override or shared-service interruption.
Frontend 04 is integrated by the coordinator separately; these probes used this
feature branch's Web, not a claim of the final combined release tree.

The coordinator's existing E2E scripts were copied into an own temporary adapter;
foreign source was not edited. The copy adjusts its repository path, counts
`requestVideoFrameCallback` presentations when Firefox reports zero
`getVideoPlaybackQuality().totalVideoFrames` for MediaStream, and avoids reading
the Firefox SDP getter after close. Before those adapter corrections, Firefox
already decoded 28 frames/one keyframe with changing canvas pixels, but its
rendered-frame assertion stayed zero; the navigation exception pointed exactly
to the closed-peer `localDescription` getter. These are test environment limits.

Redacted app evidence: `/tmp/gelabber-media-ui-app.json`,
`/tmp/gelabber-media-ui-routes.json`, `/tmp/gelabber-media-ui-firefox.json` and
`/tmp/gelabber-media-ui-firefox-nav.json`. Test logs:
`/tmp/gelabber-media-ui-{all-final,identity-final,playback-final,lint,build}.log`.

08b still owns SFU Watch-source filtering, all associated room audio and binding
Live publication to the exact Gateway claim/media peer. Integrated lifecycle,
native capture, perceptual audio, relay/WAN and production acceptance remain open.
