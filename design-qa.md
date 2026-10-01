# Living Room production integration

final result: passed

Reviewed on 2026-10-01. This is the approved Living Room design integrated into
Gelabber's existing API, WebSocket gateway and SFU.

## Current iteration: compact layout and bottom call bar

Rafael requested smaller UI throughout and no controls in the middle of a call.
This direction supersedes the original concept's large sizing. The comparison
and measurements below under “Visual evidence” document the initial integration;
the current iteration uses the following sizes and evidence:

- Desktop rail/sidebar/members widths: 64 / 224 / 248 px. Welcome heading:
  36 px maximum instead of 54 px. Room avatars: 84 px instead of 124 px.
  Navigation rows and virtualizer estimates were reduced together; chat keeps
  readable 14 px text and phone inputs retain 16 px text.
- Joined-room controls and the duplicate watch-stop button were removed from
  the center. One persistent bar holds the real controls across navigation.
  It sits against the bottom edge, aligned after the sidebar on desktop, and
  spans the phone viewport. The measured 53 px desktop / 81 px phone height is
  reserved above the composer. On phones, playback volume is available through
  the bar's Voice settings button. Diagnostics are also in Voice settings.
- Current 1:1 CSS-density captures: `compact-overview-desktop.png` (1487 × 1058)
  and `compact-overview-mobile.png` (390 × 844) in the existing evidence folder;
  `compact/desktop-active-call.png` and `compact/call-chat-390x844.png` show the
  actual connected call. Compare with the earlier empty-room and call-chat
  captures at the same sizes and interaction states.
- Typography and spacing are intentionally denser. Palette, landscape asset,
  real-state copy and account imagery behavior remain as reviewed below.
  Desktop/mobile captures have no clipping, overlap or horizontal overflow.
  The room remains clear while joined; call actions stay at the bottom.
- Regression evidence: `compact/media-regression.json` passed four selected
  real-backend scenarios for watching, navigation, camera/screen/live identity
  and responsive controls. The last checks a single control set in the room,
  bottom alignment, accessible call buttons and an uncovered chat composer at
  five viewport sizes. Browser console: zero errors or warnings.

Current visual result: no actionable P0/P1/P2 findings. Changes to scale and
control placement are intentional responses to the user's requested iteration.

## Visual evidence

- Source: `.tools/living-room-preview/reference/living-room.png`.
- Implementation: `.tools/living-room-integration/evidence/desktop-dark-room.png`
  and `desktop-occupied-room.png` in the same evidence directory.
- Source and desktop captures: 1487 × 1058 pixels, 1487 × 1058 CSS viewport,
  screenshots at CSS density (1:1); no browser chrome or scaling.
- The source and implementation were opened together in the same comparison
  input. Both show a signed-in community overview before joining voice, in dark
  mode. Dynamic content differs deliberately: the source contains five example
  people and a staged broadcast; production captures show actual local test
  accounts, real room occupancy and no broadcast when nobody is streaming.
  Initials are the existing avatar fallback when an account has no image URL.
  This is a comparison of the implemented design, not a pixel-identical content
  claim. Generated people, activity and stream previews are not seeded into the
  product.
- Focused checks: the 1:1 captures were also inspected for sidebar navigation,
  heading, room avatars and join controls. All are readable at this resolution;
  no additional magnification was needed. Call controls and the composer were
  checked separately in `call-chat-1487x1058.png` and `call-chat-390x844.png`.
- Other evidence: `mobile-dark-room.png` (390 × 844),
  `desktop-empty-room.png` / `mobile-empty-room.png` (light mode),
  `short-before.png` / `short-after.png` (1366 × 600), and
  `landscape-navigation.png` (780 × 390).

## Findings and fixes

1. **P2, fixed — first visit used the OS light theme.** The approved experience
   is dark. Missing or invalid saved preferences now default to dark in both
   the initial HTML and the theme store. Explicit light and system preferences
   remain supported. Dark captures above and browser theme switching confirm
   the result.
2. **P2, fixed — short windows collapsed channel navigation.** Independent
   review identified this; at 1366 × 600 the channel list measured 19.5 px.
   The list now retains a 156 px minimum and the containing sidebar scrolls.
   Post-fix captures and browser checks confirm that account and management
   controls remain reachable, including in the landscape navigation drawer.
3. **P2, fixed — active calls covered the desktop chat send button.** Browser
   hit testing reproduced the overlap at 1487 × 1058, 1366 × 600 and 844 × 390.
   The shell now reserves the actual dock height on every viewport, measured
   by ResizeObserver. Post-fix screenshots and the permanent
   `active-call-responsive-controls` E2E scenario confirm an unobstructed send
   button, no horizontal overflow and reachable navigation at five sizes.

No actionable P0/P1/P2 visual findings remain.

## Required fidelity surfaces

- **Typography:** the existing sans-serif stack retains the source's large,
  tightly spaced welcome heading, smaller room heading and muted supporting
  text. Names wrap or truncate without widening columns. The source's exact
  font is not available; the product uses its existing system font stack.
- **Spacing:** desktop keeps the 88 px server rail, 260 px navigation column,
  flexible center and 304 px presence column. Narrow screens use native modal
  drawers. Room and account content scroll without hiding persistent controls.
- **Colors:** charcoal surfaces, warm amber actions and active navigation,
  muted secondary text and semantic presence colors carry through the shell,
  chat, voice, dialogs and account screens. Light mode remains usable.
- **Images:** the approved landscape asset is reused with a cover crop; there
  are no invented users or static broadcast images in production. Real profile
  URLs and MediaStreams supply user and video imagery. Server initials remain
  appropriate because the current server model has no image field.
- **Copy:** community and room names, counts, people and activity use real
  state. Empty rooms invite joining; the interface does not claim that someone
  is speaking without an actual speaking signal. Auth, chat, settings and
  moderation retain their existing product behavior.

## Browser interaction checks

- Registration, community creation, channel creation and rename through the UI.
- Desktop and mobile navigation; nested create-channel dialog Escape closes
  only that dialog, followed by Escape restoring focus to the navigation opener.
- Member profile dialog and DM entry; light/dark switching; compact room and
  reduced-motion preferences survive reload.
- Eight selected core E2E scenarios cover account reload/login, chat edits and
  deletes across users, DMs, image attachments, paging and retry recovery.
- Five selected media E2E scenarios cover live viewing without mic access,
  navigation during watching, cancelled capture, distinct camera/screen/live
  sources and autoplay retry. A sixth scenario checks responsive call controls.
- Final shared-browser console check: zero errors and zero warnings.

Local reports are in `.tools/living-room-integration/evidence/`:
`core-run-1.json`, `media-run-1.json`, `responsive-regression.json`.
Media tests use native browser WebRTC against the local SFU with synthetic
capture. Physical-device audio quality, native screen-picker UX and WAN
conditions require a separate device test; they are not claimed here.

## Implementation checklist

- [x] Real overview, member profiles, chat/DM navigation and account settings.
- [x] Voice, camera, screen sharing, live viewing and persistent call controls.
- [x] Local microphone test with capture cleanup and saved display preferences.
- [x] Responsive drawers, keyboard interactions and short-viewport fixes.
- [x] Rechecked visual fixes against browser captures.

Follow-up polish: exact font matching and server artwork uploads can be separate
iterations. The current backend model and real community content determine
the remaining visual differences from the concept.
