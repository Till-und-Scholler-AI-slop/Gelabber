# Living Room production integration

final result: passed

Reviewed on 2026-10-01. This is the approved Living Room design integrated into
Gelabber's existing API, WebSocket gateway and SFU.

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
