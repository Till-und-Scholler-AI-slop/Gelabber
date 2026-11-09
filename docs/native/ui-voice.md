# Spec: ui-voice (VoiceRoom, stage, tiles, call bar, settings, native media)

Source of truth: `web/src` at branch `claude/native-foundation`. All paths below are relative to `web/src/` unless they start with `desktop/`. Every value was read from the CSS/TS, not estimated. "dark"/"light" = the built-in presets `dark` ("Gelabber Dunkel") and `light` ("Gelabber Hell"), style `clear` (`theme/presets.ts:5-21`, `theme/defaults.json`), passed through `fixContrast` (`theme/presets.ts:227`) and `themeVariables` (`theme/model.ts:295-391`). Hex values for derived tokens were computed by running `themeVariables()` itself.

Conventions for implementers:
- 1 CSS px = 1 egui point at UI scale 1 ("Kompakt"; `--lr-ui-scale` `index.css:111`, html `zoom`). Desktop pointer is fine, so `(pointer: coarse)` rules do NOT apply; they are listed only where they change behaviour.
- `color-mix(in srgb, A p%, transparent)` = A with alpha p. `color-mix(in srgb, A p%, B)` = per-channel linear mix in sRGB gamma space (same as `mix()` in `theme/model.ts:124`).
- Font: "Gelabber Inter" = Inter 4.1 variable (`index.css:41-76`, files `fonts/InterVariable-*.woff2`). Load the same TTF/woff2 into egui. Weights used: 400, 500, 600, 650, 700.
- Icons: all from `components/Icons.tsx`, 24x24 viewBox, `fill:none; stroke:currentColor; stroke-width:2; round caps/joins` (`Icons.tsx:5-19`), drawn at the given `size` (stroke scales with size: stroke = 2*size/24). SVG data in section 14.

---

## 1. Tokens (both presets)

### 1.1 Colours (`theme/model.ts:187-238, 347-369`; `index.css:95`)

| Token | Dark | Light | Notes |
|---|---|---|---|
| `--lr-bg` | `#17191a` | `#f8f5f0` | |
| `--lr-panel` | `#1d2021` | `#f4f0e9` | call bar bg, chip-group bg |
| `--lr-surface` | `#272b2d` | `#ece7df` | person cards/chips |
| `--lr-rail` | `#131617` | `#eee9e1` | |
| `--lr-elevated` | `#252829` | `#fcfbf9` | popovers, modal |
| `--lr-text` | `#eef0ee` | `#252829` | |
| `--lr-muted` | `#a4abb1` | `#5c646b` | |
| `--lr-subtle` | `#93989b` | `#616467` | |
| `--lr-border` | `#2c3032` | `#dcd6cc` | |
| `--lr-border-strong` | `#717475` | `#86837e` | |
| `--lr-accent` | `#f3b752` | `#e8a638` | |
| `--lr-accent-ink` | `#111111` | `#111111` | text on accent |
| `--lr-accent-text` / `--lr-focus` | `#f3b752` | `#836026` | focus ring colour too |
| `--lr-hover` | `#2e3131` | `#e3e0da` | |
| `--lr-selected` | `#443b2a` | `#f2e3c9` | |
| `--lr-online` = `--lr-speaking` | `#85d69b` | `#287a45` | speaking ring |
| `--lr-online-text` | `#85d69b` | `#277643` | |
| `--lr-warning` | `#d8a657` | `#a27e44` | |
| `--lr-danger` | `#f39b98` | `#a12b29` | |
| `--lr-danger-bg` | `#8f302d` | `#b33b38` | leave button |
| `--lr-live` / `--lr-on-live` | `#ff7b5c` / `#111111` | `#db5b24` / `#111111` | Live badge, Live button |
| `--lr-on-danger` | `#ffffff` | `#ffffff` | |
| `--lr-backdrop` | `rgba(9,12,13,0.72)` | `rgba(37,40,41,0.45)` | modal backdrop |
| `--lr-tile-1..8` (avatar tints) | `#634336 #635636 #4e6336 #366349 #365d63 #364963 #433663 #633653` | `#ead2c8 #eae0c8 #daeac8 #c8ead6 #c8e5ea #c8d6ea #d2c8ea #eac8dd` | picked by `personTint(name)` (`components/initials.ts:43-53`, FNV-1a 32-bit, `0x45d9f3b` scramble, `% 8 + 1`) |

Derived composites used in this area (pre-computed):

| Use | Formula | Dark | Light |
|---|---|---|---|
| `.voice-control-compact.is-off` bg in dock (`room.css:1189-1191`) | danger-bg @16% over panel | `rgba(143,48,45,.16)` ≈ `#2f2323` | `rgba(179,59,56,.16)` ≈ `#ead3cd` |
| Leave hover (`room.css:1238-1240`) | danger-bg 88% + black 12% | `#7e2a28` | `#9e3431` |
| `.voice-room-primary:hover` (`room.css:783-785`) | accent 86% + white 14% | `#f5c16a` | `#ebb254` |
| Reconnecting text (`room.css:1098-1100`) | warning 70% + text 30% | `#dfbc84` | `#7d643c` |
| Playback row bg (`room.css:1543`) | accent 14% + panel | `#3b3528` | `#f2e6d0` |
| Kino strip person bg (`room.css:307`) | panel @92% alpha | `rgba(29,32,33,.92)` | `rgba(244,240,233,.92)` |
| Speaking glow inside video (`room.css:1728-1729`) | speaking @50% | `#85d69b80` | `#287a4580` |

Theme-independent constants (identical in light and dark): video bg `#0b0d0e`; stage overlay pills `rgba(10,12,13,0.62)` / hover `0.82` / Kino `0.72` / Kino hover `0.88`; mute marks on pictures `#f39b98` (hard-coded, `room.css:173`); Kino secondary text `#c9cdd0`; watch strip `rgba(18,20,21,0.92)`.

### 1.2 Type scale (`index.css:98-106`), `font: weight size/line-height`

| Token | Value |
|---|---|
| `--lr-type-caps` | 600 11px / 1.3 |
| `--lr-type-meta` | 400 12px / 1.4 |
| `--lr-type-ui` | 500 13px / 1.35 |
| `--lr-type-body` | 400 14px / 1.5 |
| `--lr-type-title-sm` | 600 16px / 1.3 |
| `--lr-type-title` | 600 20px / 1.25 |

### 1.3 Radii (style `clear`, `theme/model.ts:290-294, 374-381`)

| Token | px |
|---|---|
| `--lr-radius-xs` | 3 |
| `--lr-radius-sm` / `--lr-radius-small` | 5 |
| `--lr-radius-md` / `--lr-radius` | 7 |
| `--lr-radius-lg` | 10 |
| `--lr-radius-xl` | 14 |
| `--lr-radius-full` | 999 |

### 1.4 Sizes, shadows, motion (`index.css:108-172`, `theme/model.ts:382-389`)

| Token | Value |
|---|---|
| `--lr-control-xs/sm/md` | 28 / 32 / 36 px (44 under `pointer:coarse`) |
| `--lr-rail-width` + `--lr-sidebar-width` = `--lr-navigation-width` | 64 + 224 = 288 px |
| `--lr-members-width` | 248 px |
| `--lr-header-height` | 52 px |
| `--lr-dock-height` | 56 px |
| `--lr-shadow-1` | dark `0 8px 24px rgba(0,0,0,.32)`; light `0 8px 24px rgba(60,45,20,.12)` |
| `--lr-shadow-2` | dark `0 18px 48px rgba(0,0,0,.5)`; light `0 18px 48px rgba(60,45,20,.18)` |
| durations | micro 100ms, fast 120ms, std 180ms, large 240ms, exit 130ms, large-exit 170ms |
| easings | `--lr-ease` cubic-bezier(.2,.8,.2,1); standard (.2,0,0,1); exit (.4,0,1,1) |
| `--lr-press-scale` | 0.97 (buttons scale on :active) |
| Reduced motion | all durations 0, press scale 1, speaking glow / spinner / connecting pulse stop (`index.css:150-172`, `room.css:1632-1646, 1741-1748`) |

### 1.5 Speaking indicators (shared)

| Where | Rule | Source |
|---|---|---|
| Avatar (`.gel-avatar.is-speaking`) | `box-shadow: 0 0 0 2px speaking` + `::after` (same circle) `box-shadow: 0 0 0 4px speaking@30%, 0 0 8px 4px speaking@45%`, opacity animates 0.45→1→0.45, 1.2s ease-in-out infinite (`@keyframes lr-speaking-glow { 50% {opacity:1} }`) | `index.css:1258-1288` |
| Video tile (`.voice-video-tile.is-speaking`) | `::after` inset ring `inset 0 0 0 3px speaking` opacity 1; `::before` `inset 0 0 22px 6px speaking@50%` opacity 0.45 animated with the same keyframes; both fade in/out 0.15s ease-out; drawn ABOVE the video, below labels (z-index 1) | `room.css:1713-1748` |
| Person card/chip (`.voice-room-participant.is-speaking`) | `border-color: speaking; box-shadow: 0 0 0 1px speaking` (so 2px total green edge); status text colour → online-text | `room.css:623-629` |

"speaking" for a user = speaking detector true AND not muted AND not deafened (`VoiceRoom.tsx:163-167`; own user uses local muted/deafened).

---

## 2. Page placement and geometry (channel page of a voice channel)

`pages/ChannelPage.tsx:68-104`: `[content column: header(52) + scroll box(flex 1, overflow-y auto) containing VoiceRoom] [MemberPanel 248]`. The call bar is `position:fixed` and the workspace content reserves `padding-bottom: --lr-media-space` = measured dock height (`index.css:390-395`, `VoiceSessionControls.tsx:74-89`); on desktop that is 56 px.

Member panel: while the room has pictures (`staged`), the panel defaults to HIDDEN (`components/memberPanelState.ts:45-46`, `withVideo ?? true`) unless the user toggled it in that state; without pictures it uses the general remembered value (default visible).

Room box (`.voice-room`, the `<section>` inside the scroll box):

| Viewport | Member panel | Room box (w x h) | Notes |
|---|---|---|---|
| 1280x800, not staged | visible (default) | 744 x 692 | 1280-288-248; 800-52-56 |
| 1280x800, staged | hidden (default) | 992 x 692 | |
| 1280x800, Kino | n/a (hidden) | 1280 x 744 | rail, sidebar, header, member list `display:none`; dock spans from x=0 (`room.css:255-263`) |
| 1600x900, not staged | visible | 1064 x 792 | |
| 1600x900, staged | hidden | 1312 x 792 | |
| 1600x900, Kino | — | 1600 x 844 | |

Breakpoints that matter (all `max-width` unless noted):

| Query | Effect | Source |
|---|---|---|
| ≤640px | `narrow`: view forced to Raster, no view chips, stage becomes a 1-column scrolling grid (16:9 items), people grid 3 columns, avatar 56 | `VoiceRoom.tsx:133,317`; `room.css:799-886` |
| ≤800px | no Kino (`roomy` false); room inline padding 24 instead of 28; dock spans full width (no sidebar offset) | `VoiceRoom.tsx:262`; `room.css:793-797, 980-996` |
| ≤1023px | dock hides `.voice-control-text` ("Kein Mikrofon", "Live", "Verlassen"), leave padding 0 8; failed call wraps | `room.css:1039-1047, 1243-1251` |
| ≥801px | dock `left: 288px`, min-height 56, padding 8 16 | `room.css:980-996, 1010-1024` |
| ≤800px | workspace navigation (rail+sidebar) becomes a closed fixed drawer, so the room gains 288 px | `components/WorkspaceNavigation.tsx:7` (`NAVIGATION_DRAWER_MAX_WIDTH = 800`), `index.css:1718-1730` |
| ≤1130px | member panel is no longer an inline 248 px column: it becomes a fixed right-side modal drawer (closed by default, slides in 240 ms, `translate 100%→0`), opened by a fixed `Mitglieder {count}` trigger (32 high, padding 0 10, border 1, radius 5, bg panel, muted, 500 14px) at the header's right (top (52-32)/2 = 10, right 12); the header toggle button is hidden. The room therefore never loses 248 px at ≤1130 | `components/MemberPanel.tsx:68-103` (`breakpoint={1130}`), `index.css:1619-1700`, `components/chat.css:1996-2006` |
| ≥1131px | inline panel; when "hidden" it is `display:none` (`.member-panel-drawer.is-collapsed`) | `components/chat.css:2002-2006` |

At 1280 and 1600 none of the ≤ queries apply (1280 > 1130, so the member panel is inline there).

---

## 3. VoiceRoom (`voice/VoiceRoom.tsx`, CSS `voice/room.css`)

### 3.1 State model (`VoiceRoom.tsx:106-413`)

| Value | Definition |
|---|---|
| `here` | voice joined AND same server AND same channel (`:116-119`) |
| `occupants` | roster entries for this channel, excluding me unless `here`; if `here` and I'm missing, append me with local flags (`:137-147`) |
| `nameOf(id)` | me: `"{memberName ?? userName ?? 'Du'} (du)"`; others: member name or `"Mitglied"` (`:157-160`) |
| `shares` (Lives) | only while `here`: for every occupant publishing `s` (own: `voice.sharing`): tile `{id:"{uid}-s", kind:"live", label:"{name} — Live"}` (em dash U+2014) (`:169-185`) |
| `cameras` | only while `here`: occupant publishing `v` (own: `voice.camera`): tile `{id:"{uid}-v", kind:"camera", label:name, mirror: self, speaking}` (`:186-196`) |
| share order | sorted by order of `livePeople` keys (who went live first first); unknown at end (`:198-204`) |
| `pictures` | `[...shares, ...cameras]` (channel page only); `staged = pictures.length > 0` (`:252-253`) |
| `view` | `narrow ? "raster" : stageViewFor(choices.view, shares.some(t => t.userId !== me))` → picked view, else `"fokus"` if someone ELSE is live, else `"raster"` (`:317-322`, `stageView.ts:388-393`) |
| `focus` | when `view==="fokus"` or Kino: `focusOf(shares, cameras, pinned, me)` (`:323-326`) |
| `ordered` | `rasterOrder(shares, cameras, pinned)` (`:327`) |
| `strip` | `ordered` minus focus; `stripPeople` = occupants without camera (only when focus) (`:330-331`) |
| free seats | 0 if here/denied/overview; else 3 if nobody inside, else 1 (`:386-391`) |

Stage choices (`voice/stageView.ts`): per call key `"{serverId}/{channelId}"`, `{view: null|"raster"|"fokus", pinned: id|null}`. `chooseView` sets view. `pinPicture(id)` sets pinned AND forces view `"fokus"`; `pinPicture(null)` unpins and keeps view. Choices reset when the call ends or moves (`stageView.ts:395-404`). They survive leaving and reopening the page.

### 3.2 Layout algorithms — transcribe exactly (`voice/stage.ts`)

Constants: `STAGE_GAP = 12` (`:92`), `TILE_ASPECT = {min:1, max:16/9}` (`:49`), `STRIP_HEIGHT = 96`, `STRIP_TILE = 171`, `STRIP_GAP = 10` (`:143-145`), `KINO_THUMB = {picture:160, person:90, height:90, gap:8, right:16, bottom:24}` (`:189-196`), `FRAME_LOADING_MS = 300`, `FRAME_STALLED_MS = 8000`, `CONTROLS_IDLE_MS = 3000`, `LONG_PRESS_MS = 500`, slop 10 (`:282-289`).

```text
arrangeStage(shares, cameras, pinned)            // stage.ts:12-25 (overview only)
  spotlight = first of [...shares, ...cameras] with id==pinned, else shares[0], else null
  filmstrip = shares without spotlight

rasterOrder(shares, cameras, pinned)             // :29-37
  all = shares ++ cameras; if one has id==pinned: [it, ...rest] else all

fitGrid(count, width, height, gap, aspect = {min:1, max:16/9})   // :56-86
  if count < 1 or !(width>0) or !(height>0): return null
  best = null; bestArea = 0
  for columns in 1..=count:
    rows = ceil(count / columns)
    cellW = (width  - gap*(columns-1)) / columns
    cellH = (height - gap*(rows-1))    / rows
    if cellW <= 0 or cellH <= 0: continue
    tileW = min(cellW, cellH * aspect.max)
    tileH = min(cellH, tileW / aspect.min)
    area = tileW * tileH
    if area > bestArea + 0.5:            // ties keep fewer columns
      bestArea = area
      best = {columns, rows, width: floor(tileW), height: floor(tileH)}
  return best

rasterRects(count, width, height, gap=12)        // :97-119
  fit = fitGrid(count, width, height, gap); if null return []
  span(n, size) = n*size + (n-1)*gap
  top = round((height - span(fit.rows, fit.height)) / 2)
  for index in 0..count:
    row = floor(index / fit.columns); column = index - row*fit.columns
    inRow = min(fit.columns, count - row*fit.columns)
    left = round((width - span(inRow, fit.width)) / 2)   // short last row centred
    rect = {x: left + column*(fit.width+gap), y: top + row*(fit.height+gap), w: fit.width, h: fit.height}

focusOf(shares, cameras, pinned, me)             // :124-140
  pinned in [...shares,...cameras]
  ?? first share with userId != me ?? shares[0]
  ?? first camera with userId != me ?? cameras[0] ?? null

focusRects(stripCount, width, height, gap=12)    // :150-185
  strip = stripCount > 0 ? 96 + gap : 0
  fit = fitGrid(1, width, height - strip, 0, {min:16/9, max:16/9}); if null return null
  tile = max(0, min(171, floor((width - 10*(stripCount-1)) / stripCount)))
  row  = stripCount*tile + (stripCount-1)*10
  left = round((width - row) / 2)
  focus = {x: round((width-fit.width)/2), y: round((height - strip - fit.height)/2), w: fit.width, h: fit.height}
  strip[i] = {x: left + i*(tile+10), y: height - 96, w: tile, h: 96}

kinoRects(thumbs: ["picture"|"person"], width, height)   // :201-233
  fit = fitGrid(1, width, height, 0, {16/9,16/9}); if null return null
  widths = thumbs.map(k => k=="picture" ? 160 : 90)
  row = sum(widths) + 8*(len-1)
  scale = row > 0 ? min(1, (width - 2*16) / row) : 1
  x = width - 16 - row*scale            // float accumulator
  thumbH = floor(90*scale)
  focus = {x: round((width-fit.width)/2), y: round((height-fit.height)/2), w: fit.width, h: fit.height}
  for size in widths:
    rect = {x: round(x), y: height - 24 - thumbH, w: floor(size*scale), h: thumbH}
    x += (size + 8) * scale
```

Which algorithm runs (`VoiceRoom.tsx:332-356`): no measured size → nothing placed (items hidden, `room.css:453-455`); Kino → `kinoRects([...strip as "picture", ...stripPeople as "person"])`; else focus → `focusRects(strip.length + stripPeople.length)`; else `rasterRects(ordered.length)`. Placement (`:367-372`): focus tile → `layout.focus`; others → `layout.strip[index in (focus ? strip : ordered)]`; strip people → `layout.strip[strip.length + i]`. Element order in the list is fixed (`pictures` order, then strip people) — rects only move them, never rebuild a video.

The stage size is the `<ul class="voice-stage">` client box in integer CSS px (`useBoxSize`, `:706-730`).

### 3.3 Stage geometry worked out (stage `ul` size, then rects)

Room with stage (`.voice-room[data-stage]`, `room.css:62-69`): flex column, gap 12, padding 16 top/bottom, 24 left/right, `height:100%`, `min-height:360`. Children: stage bar (36 high), stage `ul` (flex 1), optional people row (44 high, only in Raster when someone has no camera).

| Viewport | Stage `ul` without people row | with people row | Kino stage |
|---|---|---|---|
| 1280x800 | 944 x 612 (692-32-36-12) | 944 x 556 (-44-12) | 1280 x 744 |
| 1600x900 | 1264 x 712 | 1264 x 656 | 1600 x 844 |

If the user opens the member panel while staged, subtract 248 from width.

Results (computed with the real `stage.ts`):

| Case | 1280x800 | 1600x900 |
|---|---|---|
| Raster 1, no people row | 944x612 @(0,0) | 1264x712 @(0,0) |
| Raster 1, people row | 944x556 @(0,0) | 1166x656 @(49,0) |
| Raster 2 | 2x1, 466x466 @y 73 (45 w/ people) | 2x1, 626x626 @y 43 (15) |
| Raster 3 | 2x2, 466x300; 3rd at (239,312) (272 high w/ people) | 2x2, 622x350 @(4,0),(638,0),(321,362); w/ people 572x322 |
| Raster 4 | 2x2, 466x300 | 2x2, 622x350 |
| Raster 5/6 | 3x2, 306x300 @x 1,319,637; row 2 of 5 at x 160,478 | 3x2, 413x350 @x 1,426,851 |
| Fokus, strip 0 | focus 944x531 @(0,41) | 1264x711 @(0,1) |
| Fokus, strip 1–5 | focus 896x504 @(24,0); strip tiles 171x96 @y 516, centred (1: x 387; 2: 296,477; 3: 206,387,568) | focus 1073x604 @(96,0); strip @y 616 (1: x 547; 3: 366,547,728) |
| Fokus, strip 6 / 8 | tiles 149x96 / 109x96 from x 0 / 1 | 171 (6, from x 94) / 149x96 (8) |
| Kino, 1 picture thumb | focus 1280x720 @(0,12); thumb 160x90 @(1104,630) | focus 1500x844 @(50,0); thumb @(1424,730) |
| Kino, picture+person | thumbs @(1006,630) 160x90, (1174,630) 90x90 | (1326,730), (1494,730) |

### 3.4 Room header / stage bar

Channel page, not staged (`VoiceRoom.tsx:489-493`): one line `.voice-room-summary` (`room.css:21-26`): margin-top 2, body type 400 14/1.5, muted, wraps anywhere. Text from `roomSummary(others, {here, live})` (`voice/roomSummary.ts:26-38`):

| Condition | Text |
|---|---|
| here, nobody else | `Nur du bist im Kanal.` |
| here, others | `Du bist hier mit {nameList}.` |
| not here, empty | `Niemand im Kanal.` |
| not here | `{nameList} ist da.` / `{nameList} sind da.` + if others live: ` {nameList(live)} ist live.` / `sind live.` |

`nameList` (`roomSummary.ts:2-7`): 1 `"Mia"`, 2 `"Mia und Lena"`, 3 `"Mia, Lena und Kim"`, 4+ `"Mia, Lena und {n-2} weitere"`. Names: member name, fallback `"Jemand"` (`VoiceRoom.tsx:379-384`).

Staged, not Kino: `.voice-stage-bar` (`VoiceRoom.tsx:438-488`, `room.css:70-86, 199-250`): flex row, align center, gap 12, min-height 36.

| Element | Spec |
|---|---|
| Facts `<p>` | flex 1, 400 13px/1.35, muted, single line, ellipsis. Text `stageFacts` (`roomSummary.ts:44-60`): `"{n} im Kanal"`; + ` · {liveSentence}` if anyone live (self as "Du" first: "Du bist live", "Du und Mia seid live", "Mia ist live", "Mia und Lena sind live"); else + ` · 1 Kamera` / ` · {n} Kameras` if cameras |
| View switch (ChipGroup radiogroup "Ansicht", hidden ≤640) | container: inline flex, nowrap, gap 2, padding 3, border 1 `--lr-border`, radius 7, bg panel → outer 36 high. Chip: height 28, padding 0 10, no border, radius 5, gap 6, 500 13px, muted; icon 15px. Options: `GridIcon` "Raster", `FocusIcon` "Fokus". Hover (unchecked): text colour + bg `--lr-hover` (from `ui.css:487`). Checked: bg surface, inset 1px `--lr-border` shadow, text colour, weight 500. Focus: 2px focus outline offset 2. Arrow keys move+select. Click → `chooseView` |
| Kino button (only `canKino` = staged && width >800) | inline flex, gap 6, height 34, padding 0 12, border 1 `--lr-border`, radius 7, transparent, text colour, 500 13px, `TheaterIcon` 15 + "Kino"; hover bg `--lr-hover` (120ms); tooltip `Kino (K)` |

Overview variant (`overview=true`, community overview page): heading row `SpeakerIcon 18` + `<h2>` 600 16/1.3 + optional pill "Verbunden" (22 high, padding 0 8, radius full, bg online@14%, online-text, 500 12px, 6px dot before) + optional badge "Live" (radius 3, bg `--lr-live`, ink `--lr-on-live`, padding 2 6, 600 11px) (`VoiceRoom.tsx:428-436`, `room.css:27-33`, `pages/overview.css:78-117`). Overview tiles: spotlight max-height min(50vh,520), filmstrip 192 wide, wrap gap 8, max-width 1100 (`room.css:36-56`). Detailed overview layout belongs to the overview spec.

### 3.5 Stage items

#### Live tile (screen share) — `<li class="voice-stage-item [is-focus|is-strip]">` + `VoiceTile screen live` (`VoiceRoom.tsx:506-527`)
Absolute at rect, border 0 (`room.css:95-107`). The `VoiceTile` figure fills it (height 100%). Video drawn `object-fit: contain` (screen) — letterbox on `#0b0d0e`. Details in §4.

#### Camera tile — `Participant tile camera=…` (`VoiceRoom.tsx:529-539`, `Participant.tsx`)
`<li class="voice-room-participant has-video voice-stage-item ...">`: radius 10 (7 in strip, 0 as Kino focus), overflow hidden, bg `#0b0d0e`, no border, no box-shadow (speaking shown by the tile ring, `room.css:667-682`). Contains a `VoiceTile` (caption off, `mirror` for own camera, `speaking`), then the name pill, then "…".

Name pill on a picture (both the Live caption and the camera's text; `room.css:110-140`):

| Property | Normal / Fokus focus | In Fokus/Kino strip (`room.css:382-405`) |
|---|---|---|
| Position | left 10, bottom 10 | left 6, bottom 6 |
| Size | height 28, padding 0 10, max-width 100%-20 (camera: 100%-58) | auto height, padding 1 6, max-width 100%-12 |
| Look | radius 5, bg `rgba(10,12,13,.62)`, white, 600 13px/1.35, nowrap, name ellipsis | radius 3, 600 11px/1.3 (no uppercase) |
| Content | Live: `"{name} — Live"`. Camera: [marks][name][" (du)" weight 400 opacity .75] | same |
| Gap | 6 | 4 |

Marks inside the pill (`room.css:145-173`): only shown if one of: local-volume chip, locally-muted icon, mic-off icon, headset-off icon. On pictures the green speaker icon is hidden; mic-off/headset-off icons are 14px coloured `#f39b98`. Volume chip: padding 1 6, radius full, bg selected, accent-text, 600 12px tabular, text `"{pct} %"` when local volume ≠100 % and not locally muted (`Participant.tsx:279-288`). Locally muted: `SpeakerOffIcon 16` (`components/UserVolumeControl.tsx:98-105`).

"…" on a camera tile (only others with a member record, `Participant.tsx:94, 207-231`): IconButton xs 28x28, radius 5, bg `rgba(10,12,13,.62)`, white, `MoreIcon 16`, right 10 bottom 10 (strip: right 4 bottom 4), opacity 0 → 1 on li hover / focus-within / menu open (120ms). Opens the participant menu (§3.8). Right click / 500 ms touch long press also open it.

Live badge on a Live tile: see §4.2; strip variant top 6 left 6.

#### Strip person (no camera) in Fokus/Kino (`VoiceRoom.tsx:542-550`, `room.css:406-440`)
Absolute at its strip rect (Fokus 171x96 or narrower; Kino 90x90): card bg surface, border 0 (`.voice-stage > .voice-stage-item {border:0}`), radius 7, flex column centred, gap 6, padding 8. Avatar 40px (initials 15px, 600); connection dot hidden; name 600 12px/1.4, self suffix `" (du)"` at weight 400 opacity .75 (`room.css:141-144` matches everything inside `.voice-stage`); status hidden; marks top 6 left 6: green speaker icon and empty slots hidden, mic-off/headset-off icons coloured `#f39b98` (the rule `room.css:172-174` `.voice-stage .voice-room-participant-marks .voice-state-icons svg` matches strip persons too — NOT muted colour), volume chip / locally-muted icon as usual; "…" top 4 right 4. Speaking in Fokus: avatar ring+glow AND the card's `box-shadow: 0 0 0 1px speaking` (`room.css:623-626`; only `.has-video` removes it) — a 1 px green outline outside the card (border-colour change is invisible since border is 0). Kino: bg panel@92%, border-colour transparent, shadow `0 6px 18px rgba(0,0,0,.45)` which overrides the 1 px speaking ring (higher specificity, `room.css:296-300`), fades with idle (§3.7).

Fokus strip interactions: `.is-selectable` tiles get `cursor:pointer`, hover `filter: brightness(1.08)` (`room.css:379-381`); click on picture (not on a button) → `pinPicture(id)` → becomes focus (`VoiceRoom.tsx:376-377`).

#### People row (Raster, ≥641 px) — `<ul class="voice-room-participants voice-stage-people" aria-label="Ohne Kamera">` (`VoiceRoom.tsx:620-631`, `room.css:456-530`)
Shown when no focus and at least one occupant has no camera. Flex wrap, gap 8. Chip:

| Part | Spec |
|---|---|
| Chip | flex row, align center, gap 10, height 44, padding 0 14 0 5, radius 999, border 1 `--lr-border`, bg surface; speaking: border speaking + 1px ring |
| Avatar | 32px circle, initials 13px 600 on tint (or panel), image cover; ring when speaking |
| Name | 600 13px/1.35 single line (`strong > button`), `" (du)"` suffix for self in the SAME style (600, opacity 1): `.voice-room-self` is only restyled inside `.voice-stage` (`room.css:141`), and the people row is outside it; status line hidden (sr-only) |
| Marks | inline, only volume chip / local mute / mic-off / headset-off (muted colour, 14px) |
| "…" | 28x28 xs IconButton, collapsed to width 0 (margin-left -10, margin-right -8, overflow hidden) and expands to 28 (margin-left 0) on chip hover/focus-within/menu open; 120ms width+margin transition |

### 3.6 Not-staged room content (channel page)

Room padding 20 / 28 / 28 / 28 (`room.css:4-12`).

People grid `<ul class="voice-room-participants" aria-label="Teilnehmer im Sprachkanal">` (`room.css:532-540`): grid `repeat(auto-fill, minmax(min(100%,200px), 1fr))`, gap 12, max-width 1100, margin-top 16. Column math: `cols = floor((W+12)/212)`, tile width `(W-12*(cols-1))/cols`, height = width*10/16.

| Viewport | Inner width W | Columns | Card |
|---|---|---|---|
| 1280x800 (member panel visible) | 688 | 3 | 221.33 x 138.33 |
| 1280x800 (panel hidden) | 936 | 4 | 225 x 140.6 |
| 1600x900 (panel visible) | 1008 | 4 | 243 x 151.9 |
| 1600x900 (panel hidden) | 1256 → capped 1100 | 5 | 210.4 x 131.5 |

Person card (`Participant.tsx`, `room.css:541-664`):

| Part | Spec |
|---|---|
| Card | flex column, centre/centre, gap 2, aspect 16:10, padding 14 12 12, border 1 `--lr-border`, radius 10, bg surface; border-colour/box-shadow transition .15s ease-out |
| Avatar button | 64px circle (initials 20px 600, bg tint else panel, text colour); hover `brightness(1.1)`, pointer; click → profile dialog (disabled when no member) |
| Connection dot | 15x15 circle, bottom 2% right 2% of the avatar, 2px border surface, fill online |
| Text block | margin-top 6, column, centred; name 600 13px/1.35 (button, wraps anywhere; hover `brightness(1.1)`, pointer, disabled without member — `room.css:584-590`) + `" (du)"` in the same 600 style; status line 400 12px/1.4 muted: `Taub` / `Stumm` / `Spricht` / `Im Raum` + optional ` · Live` / ` · Kamera an`; speaking → online-text |
| Marks | absolute top 8 left 8, gap 4, muted: volume chip, locally muted icon, `VoiceStateIcons` (slot 1: `MicOffIcon 14` if muted/deafened else `SpeakerIcon 14` in `--lr-online`; slot 2: `HeadsetOffIcon 14` if deafened; slots 14x14, gap 2) (`components/VoiceStateIcons.tsx`) |
| "…" | xs 28x28 ghost IconButton, top 6 right 6, opacity 0 → 1 on hover/focus/open; muted, hover bg hover + text |

Free seat (`VoiceRoom.tsx:648-653`, `room.css:730-737`): same box as a card but no border/background; circle 64px with `1.5px dashed --lr-border-strong`, margin-bottom 6; text `Platz frei` 400 12px subtle.

Switch note (`VoiceRoom.tsx:656-660`): when in another call: `Du verlässt dabei {voice.channelName ?? "deinen Sprachkanal"}.` 400 12px muted, margin-top 16.

Actions row (`VoiceRoom.tsx:661-686`, `room.css:738-751`): flex wrap, gap 8, margin-top 20; only when not here.

| Control | Spec |
|---|---|
| Join | Button primary lg: min-height 44, padding 0 20, radius 5, bg accent, ink `#111111`, 600 14px, gap 8, `SpeakerIcon 20`, min-width 180; text `Beitreten` or (in another call) `Hierher wechseln`; hover `brightness(1.07)`, active scale .97 (`components/ui/ui.css:12-56`) |
| Denied (no `join_voice`) | `Keine Berechtigung für Sprachkanäle.` 400 13px muted instead of the button |
| Mic test | IconButton secondary md: 36x36, radius 5, border 1 border-strong, bg surface, text colour, `MicIcon 18`, tooltip `Mikrofon testen`; hover bg hover → opens MicrophoneTest modal |
| Quick settings | `QuickCallSettingsButton variant=secondary` (same 36x36 secondary look, `GearIcon 17`, label `Anrufeinstellungen`) → popover §6.2 |

Join behaviour (`session.ts:1206-1279`): state flips to joined immediately (connection "connecting"), sends `{op:"sig",t:"j",s,c}` (+ `t:"m"`/`"d"` with `on:true` if carried over), then media starts in background. Switching calls sends `t:"l"` for the old one.

### 3.7 Kino (`VoiceRoom.tsx:260-316, 553-576`; `room.css:251-373`)

| Aspect | Spec |
|---|---|
| Enter | button, or key `K` (no Ctrl/Alt/Meta/Shift, not repeat, not the PTT key, not while typing in a text field or with a modal open) (`stage.ts:239-266`) |
| Leave | `Esc`, `K` again, or the exit chip |
| Availability | only staged and window >800 wide; leaves automatically otherwise |
| Shell | hides rail+sidebar (`.workspace-navigation`), channel header, member drawer and its trigger; call bar from x=0 |
| Room | padding 0, bg `#0b0d0e`, position relative; cursor hidden while not awake |
| Focus tile | radius 0; its own caption, Live badge, name pill and "…" hidden; actions top 12 right 16, buttons 34x34, radius 7, bg `rgba(10,12,13,.72)` — constant: the Kino rule (`room.css:283-289`) has the same specificity as the stage hover/pressed rule (`room.css:187-190`) and comes later, so hover and pressed do NOT darken to .82 (pressed icon still accent); no "Anheften" on the focus; while awake the actions are opacity 1 even without hover (`room.css:290-292`) |
| Thumbs | strip items z-index 1, shadow `0 6px 18px rgba(0,0,0,.45)`, opacity transition 180ms |
| Idle | `awake` true for 3000 ms after pointer move/down, key, focus (`useWake.ts`); when not awake: thumbs not hovered/focused → opacity 0 & no pointer events; focus actions (unless focused) opacity 0 |
| Kino bar | absolute top 24 left 16, z 3, flex gap 10, max-width 100%-160 |
| Exit chip | inline flex, gap 8, height 34, padding 0 12, radius 7, bg `rgba(10,12,13,.72)` (hover .88), white 500 13px: `TheaterExitIcon 15` + `Kino verlassen` + `<kbd>Esc</kbd>` (padding 1 5, border 1 `rgba(255,255,255,.25)`, radius 3, colour `#c9cdd0`, 400 11px/1.4) |
| Label chip | same chip box, weight 600, ellipsis: optional mini badge `Live` (padding 1 5, radius 3, `--lr-live`, ink `--lr-on-live`, 600 10px/1.3) + `<strong>{nameOf(focus.userId)}</strong>` (no own font rule, so Tailwind preflight `font-weight: bolder` turns the inherited 600 into **900**) + `· {channelName}` (`#c9cdd0`, 400). Kino exit chip text and label chip text are 13px/1.35 (`--lr-type-ui`). `nameOf` for self is `"{name} (du)"` |
| Focus moves | after entering by button, focus goes to the exit chip; after leaving, back to the Kino button |

### 3.8 Participant menu (`Participant.tsx:236-276`, `room.css:712-728`)
Popover (role dialog) anchored bottom-end of "…", 6 px gap, kept 8 px inside the window (`components/ui/popoverPosition.ts:9-10`): width 288, padding 12, radius 10, border 1 border, bg elevated, shadow-1; opens scale .96→1 + opacity over 180ms.
Content: name 600 13px, margin-bottom 10 → `UserVolumeControl` → ghost sm button `Profil ansehen` (full width +24, margin 8 -12 0, left aligned).

`UserVolumeControl` (`components/UserVolumeControl.tsx:28-90`, `index.css:2173-2217`): grid gap 10; row `Lautstärke` / `{pct} %` (14px, value muted tabular); range slider 0–200 positions (§6.4 mapping); row: checkbox `Lokal stummschalten` + text button `Zurücksetzen` (accent-text 13px; disabled muted when 100 % and not muted); hint `Nur für dich hörbar. Live-Ton bleibt unverändert.` 12px muted.

---

## 4. VoiceTile (`voice/VoiceTile.tsx`, `voice/viewer.css`)

### 4.1 Box
`<figure class="voice-video-tile">` (`viewer.css:3-14`): relative, overflow hidden, width 100%, aspect 16:9 (on the stage: fills the item), radius 10, bg `#0b0d0e`, colour white, `isolation`. `is-selectable` → pointer cursor. Video fit: `contain` if screen OR spotlight/focus OR fullscreen; else `cover` (`VoiceTile.tsx:391`). Own camera mirrored horizontally (`-scale-x-100`).

Native path (desktop app): frames come from `media_view_*` (§12); a tile draws them where the browser has `<video>`; until the first frame is painted the surface is transparent (`opacity-0`).

### 4.2 Overlays (z-order bottom→top)

| Overlay | Condition | Spec |
|---|---|---|
| Blocked overlay | browser refused autoplay | full cover, bg `rgba(0,0,0,.65)`, column centred, gap 8, padding 12, 500 13px: `Die Videowiedergabe ist blockiert.` + primary sm button `Wiedergabe starten` (`VoiceTile.tsx:502-509`, `viewer.css:44-62`) |
| Watch overlay | Live with source-watch support and not watching | centred button `Zuschauen`: padding 9 14, radius 5, bg accent, ink, 600 13px (`viewer.css:331-347`) |
| Own-Live sound tag | own Live | `.voice-video-sound` beside the Live badge in `.voice-video-tags` (top 8 left 8, flex gap 4, z 1): inline-flex gap 4, padding 2 6, radius 3, bg `--lr-bg` 82%, colour `--lr-text`, caps 11px, nowrap. `SpeakerIcon 12` + `Mit Ton` while sharing; else `SpeakerOffIcon 12` in `--lr-warning` + `Ohne Ton` (`.voice-video-sound--off`). The full sentence is its tooltip and screen-reader text (role status). Hidden in the strip and on Kino's focus, like the Live badge (`room.css`). Sentences (`VoiceRoom.tsx:213-231`): desktop `Live-Ton nicht aufgenommen · Video läuft weiter` / browser `Kein Live-Ton freigegeben · Video läuft weiter` (unavailable); `Live-Ton beendet · Video läuft weiter` (ended); `Kein Ton von „{app}“` (silent); `Ton wird mitgesendet` (sharing) |
| Placeholder | no picture yet | column centred, gap 8, padding 12, 500 13px, `rgba(255,255,255,.72)`. idle (stream null): the label; after 300 ms: spinner (16px ring, 2px `rgba(255,255,255,.25)`, top `#fff`, 0.9s linear) + `Video wird geladen …`; after 8 s: `Kein Bild empfangen` + sm button `Neu laden` (re-watch or reattach). Native window: `Läuft im eigenen Fenster.` (12px) + sm `Fenster schließen` |
| Caption (non-stage) | `caption && !immersive` | bottom strip, padding 4 8, bg `rgba(0,0,0,.5)`, 400 12px/1.4, ellipsis. On the stage it becomes the pill of §3.5 |
| Live badge | `live` | in `.voice-video-tags` (top 8 left 8), padding 2 6, radius 3, bg `--lr-live`, ink `--lr-on-live`, 600 11px/1.3, text `Live` |
| Low quality | native frames limited | `Geringe Bildqualität`, left 8, top 8 (32 when Live), padding 2 6, radius 3, bg `rgba(0,0,0,.6)`, 600 11px; tooltip per reason (`VoiceTile.tsx:818-822`) |
| Speaking ring | §1.5 | |
| Scrim (non-stage) | hover | top 56px gradient `rgba(0,0,0,.55)`→transparent; hidden on the stage (`room.css:196-198`) |
| Actions | hover / focus-within / awake | §4.3 |

### 4.3 Hover actions (`VoiceTile.tsx:614-658`)
Container: absolute top 6 right 6, gap 2 (stage: top 10 right 10, gap 6; strip: top 4 right 4, gap 2), opacity 0 → 1 on tile hover/focus-within/awake, 120ms `--lr-ease`. Buttons: IconButton sm 32x32, radius 5 (stage), icon 16, white; non-stage hover/pressed bg `rgba(255,255,255,.18)`; stage bg `rgba(10,12,13,.62)`, hover/pressed `.82`; pressed icon colour accent.

| Order | Label (tooltip) | Icon | Shown when | Action |
|---|---|---|---|---|
| 1 | `Nicht mehr zuschauen` | EyeOff 16 | watching a Live (source watch) | toggle watch off |
| 2 | `Live-Ton stummschalten` (pressed = muted) | Speaker/SpeakerOff 16 | spotlight/focus, someone else's Live, watched | `sourceAudioMuted` toggle |
| 3 | `Anheften` (pressed = pinned) | Pin 16 | `onTogglePin` (not on Kino focus) | `pinPicture(pinned==id ? null : id)` |
| 4 | `Vollbild` (title `Vollbild (auch per Doppelklick)`) | Expand 16 | always | fullscreen (§4.4). Double-click on the picture toggles too |
| 5 | `Weitere Aktionen` | More 16 | native consumer track | menu: `Eigenes Fenster` / `Fenster schließen` (`media_viewer_open/close`); failure text `Das Videofenster konnte nicht geöffnet werden.` (bottom 28, inset 12, padding 10, radius 5, bg `#16191bf2`) |

### 4.4 Fullscreen / large view + WatchStrip (`VoiceTile.tsx:105-139, 594-611, 702-814`; `viewer.css:125-321`)
Route: element fullscreen where allowed, else a modal "large view" dialog: fixed inset 12 px, border 1 border, radius 12, bg `#0b0d0e`, backdrop `#000000bb`. For egui: open the tile in a fullscreen viewport (bg `#000`, video contain), with the WatchStrip; no caption, no hover actions. Keys while immersive: `M` or `Space` (not on a button/field) → `toggleMute()` when joined; Esc leaves.

WatchStrip: absolute, centred horizontally, bottom 16; flex gap 8; max-width 100%-24; padding 6 8 6 14; border 1 `rgba(255,255,255,.14)`; radius 10; bg `rgba(18,20,21,.92)`; shadow `0 18px 48px rgba(0,0,0,.5)`; white. Hides (opacity 0, 180ms) after 3 s idle unless hovered/focused; cursor hidden then.

| Part | Spec |
|---|---|
| Label | `<strong>` 600 13px, ellipsis, margin-right 4 |
| Group (joined only) | left border 1 `rgba(255,255,255,.14)`, padding-left 8, gap 4: mute + deafen toggles (36x36, padding 0 9, transparent, white; hover `rgba(255,255,255,.14)`; off: bg danger-bg@45% alpha, white) |
| Stream volume (someone else's Live) | group: mute button (Speaker/SpeakerOff 17) + range 112 px (disabled when muted/deafened) |
| `Nicht mehr zuschauen` | button min-height 36, padding 0 12, radius 5, bg `rgba(255,255,255,.12)` (hover .2), 500 13px, gap 6, EyeOff 16 |
| `Vollbild beenden` | same button, Collapse 16 |

---

## 5. Call bar (`components/VoiceSessionControls.tsx`, `components/VoiceControls.tsx`, `room.css:924-1302`)

### 5.1 Dock box
Visible while joined, while playback is blocked, or after a lost call. `<section class="voice-session-dock" aria-label="Aktive Medien">` (`room.css:936-996`): fixed bottom, z 40, `left: 288` (≥801), right 0; flex column gap 6; min-height 56; max-height min(220, 50vh) (scrolls); padding 8 16 8 16; border-top 1 `--lr-border`; bg panel; text colour; base font 12px. Enter: translateY 100%→0 over 240ms `--lr-ease`; exit 170ms exit ease (kept mounted with last content).

Width: 1280 → 992 (inner 960); 1600 → 1312 (inner 1280); Kino → full width.

### 5.2 Main row `.voice-session-call` — flex, align center, gap 6. Left→right:

| # | Part | Spec | Source |
|---|---|---|---|
| 1 | Status head | flex `0 1 340px`, gap 8, min-width 96 | `room.css:1003-1024` |
| 1a | Quality bars (connected) | button 36x36, radius 7, hover bg hover; 4 bars 3px wide, radius 1, gap 2, bottom-aligned, heights 4/7/10/13; unlit opacity .28; colour by level: 4,3 online; 2 warning; 1 danger; 0 muted. Lit = first `level` bars. Tooltip `callQualitySummary` e.g. `Verbindung: gut · 38 ms · 0 % Verlust`; unmeasured `Verbindung: wird gemessen …` | `room.css:1109-1157`; `VoiceSessionControls.tsx:281-355` |
| 1a' | Signal icon (connecting/reconnecting) | `SignalIcon 16`, margin-left 2, muted; connecting pulses opacity 1↔.35 1.2s; reconnecting warning colour | `room.css:1053-1071` |
| 1a'' | Warning icon (failed) | `WarningIcon 16`, danger | |
| 1b | Link (→ channel page) | column, flex 1, line-height 1.3; line 1 `<strong>` 600 12px: `Verbunden` (online-text) / `Verbinde …` (muted) / `Verbindung wird wiederhergestellt …` (warning-text mix) / `Verbindung verloren` (danger); line 2 400 12px muted `{channel} · {server}` (just `{channel}` while the server name is unknown; channel fallback `Sprachkanal`); both nowrap + ellipsis; hover: line 2 text colour + underline; aria-label `{state}: {where}` | `room.css:1072-1107`; `VoiceSessionControls.tsx:225-277` |
| 1c | Retry (failed) | Button secondary md `Erneut versuchen`, radius 7 in dock | |
| 2 | People | button, margin-inline auto (centred), min-height 36, padding 2 6, radius 7, hover bg; up to 3 avatars 24px (initials 10px, bg tint else surface, ring `0 0 0 2px panel`, overlap -5px), speakers first then alphabetical (de), then sorted alphabetically; `+N` 600 12px/1.4 muted margin-left 4. Tooltip (`title`) = `{names}` joined with `", "` (all people, alphabetical, fallback name `Jemand`); aria-label `Im Anruf: {names}`. Hidden entirely when nobody is in the roster yet. Speaking avatar: glow `::after` of §1.5 (the 2px ring stays panel-coloured because room.css wins the cascade) | `room.css:1253-1279`; `VoiceSessionControls.tsx:357-466` |
| 3 | Controls | flex nowrap, gap 4 (§5.3) | |
| 4 | Leave | §5.3 | |

People popover (`Im Anruf`, placement top-start): list gap 14, min-width 260, padding 4; row: flex, gap 8, margin-bottom 6 (0 if only one person): avatar 32 (initials 12px 600, speaking ring+glow), name 600 13px/1.35 flex 1 (+` (du)`), `spricht` 400 12px/1.4 online-text; under each other person a `UserVolumeControl` (`room.css:1280-1306`).

Quality popover (`Verbindung`, top-start, width 340): compact `ConnectionStatus` (4 signal bars 4px wide, gap 2, radius 1, heights 6/10/15/20 px, box 20 high, same level colours/opacity .28 as the dock bars (`voice/connection.css:47-90, 172-197`); head gap 12; h3 600 13px `Verbindung {gut|ordentlich|schwach|schlecht}` / `Verbindung wird gemessen …` / `Keine laufende Messung`; meta `direkt verbunden`/`über Relay`; figures `Ping` ms / `Paketverlust` % at 600 20px; ping trace 30 bars height 32) + rows `↑ Du sendest` / `↓ Du empfängst` (bitrate `formatBps`, `–` if unknown) + buttons ghost sm `Einstellungen` (opens dialog audio tab) and sm `Alle Details` (diagnostics tab) (`VoiceSessionControls.tsx:314-352`, `voice/ConnectionStatus.tsx`, `voice/connection.css:1-260`). Levels: rtt limits [150,300,500] ms, loss [2,5,10] % → level = 4 - max(steps) (`diagnostics.ts:1246-1311`).

### 5.3 Buttons (`.voice-control.voice-control-compact` in the dock)

Base (`room.css:752-777, 1161-1179`): inline flex centred, gap 4, min-width 36, height 36, padding 0 8, radius 7, border 1 transparent, transparent bg, muted colour, 600 13px; hover bg hover + text colour; active scale .97. Icons 17 px; new icon fades/scales in (opacity 0→1, scale .6→1, 120ms).

| Order | Button | Normal | Pressed / state | Label + tooltip | Source |
|---|---|---|---|---|---|
| 1 | Mic | `MicIcon 17` | muted or deafened: `MicOffIcon 17`, class is-off → danger colour, bg danger-bg@16%. PTT active & not muted: badge `PTT` (600 11px, letter-spacing .04em); PTT key held: colour speaking; PTT idle: icon opacity .55 | aria `Stummschalten`; title `Stummschalten (Strg+Umschalt+M)` / `Stummschaltung aufheben (Strg+Umschalt+M)` + ` · Push-to-Talk` | `VoiceControls.tsx:66-115` |
| 1' | Mic problem (mic unavailable/denied) | `MicOffIcon 17` + 8px warning dot (top 6, left 20, 1.5px panel border) + text `Kein Mikrofon` (500 13px); colour text | — | title/aria `Kein Mikrofon`; popover `Kein Mikrofon` (placement top-start, max-width 320): `<p>` 400 14px/1.5, margin-bottom 12: unavailable `Mikrofon nicht gefunden oder belegt.`; denied: desktop `Kein Zugriff auf das Mikrofon. Erlaube ihn in den Systemeinstellungen.` / browser `Der Browser blockiert das Mikrofon. Erlaube den Zugriff in den Website-Einstellungen.`; actions flex wrap gap 8: sm (secondary) `Erneut versuchen` → `retryMicrophone()`, ghost sm `Mikrofon wählen …` → settings dialog | `VoiceControls.tsx:119-184`; `room.css:1488-1499` |
| 2 | Deafen | `HeadsetIcon 17` | deafened: `HeadsetOffIcon 17`, is-off | `Taubschalten`; title `Taubschalten (Strg+Umschalt+D)` / `Taubschaltung aufheben (…)` | |
| 3 | Camera (if capable or on) | `CameraOffIcon 17` | on: `CameraIcon 17`, is-on → bg selected, accent-text | `Kamera`; title `Kamera einschalten` / `Kamera ausschalten`. Desktop without camera: opacity .5, title+toast `Diese Desktop-App kann keine Kamera nutzen.` | `VoiceControls.tsx:231-250` |
| 4 | Live | `LiveIcon 17` + text `Live` (+ `ChevronIcon 12` rotated 180°, opacity .75, when it is a menu) | live: bg `--lr-live`, ink `--lr-on-live` | `Live gehen`; title `Du bist live` / `Live gehen` (menu) or `Live beenden` / `Live gehen – sehen alle im Sprachkanal` (plain toggle). Unavailable: opacity .5, reason `Diese Desktop-App kann nicht live gehen.` / `In diesem Server darfst du nicht live gehen.` | `VoiceControls.tsx:258-339` |
| 5 | Stream sound (while watching someone's Live) | `SpeakerIcon 17` / `SpeakerOffIcon 17` (is-off) | | `Lautstärke` → popover §6.3 | `QuickCallSettings.tsx:78-105` |
| 6 | Settings | `GearIcon 17` | | `Anrufeinstellungen` → popover §6.2 | |
| 7 | Leave | `HangUpIcon 18` + `Verlassen` (500 13px) | margin-left 4, padding 0 12, bg danger-bg, white; hover mix §1.1 | aria/title `Verlassen` | `VoiceSessionControls.tsx:468-481`; `room.css:1231-1240` |

Live menu (`Live gehen`, role menu, placement top-end, width 320) (`VoiceControls.tsx:399-484`, `room.css:1308-1500`): entries 100% wide, flex, gap 10, min-height 36, padding 8 10, radius 5; icon 17 muted, margin-top 1; title 500 13px; hint 400 12px muted, margin-top 2; hover bg hover; disabled title+icon subtle; danger (Live beenden) title+icon danger. Keys ↑ ↓ Home End.
1. `Live gehen` (LiveIcon) hint `Sehen alle im Sprachkanal` or the unavailable reason; or `Live beenden` (danger) while live.
2. separator 1px border, margin 4 0 (only where app audio possible).
3. Switch item `Ton mitsenden` (role menuitemcheckbox, align center, space-between), label padded 27 px; hint = `sourceAudioScope` (`Ton von „{app}“` / `Ton aller Anwendungen außer Gelabber` / `Ton aller Anwendungen`; browser fallback `Wähle den Ton im Freigabedialog des Browsers aus.`) + optional `Ab dem nächsten Live.` (browser only) + optional carries-call warning (text in §5.3 notices #5), joined with spaces (`VoiceControls.tsx:420-429`). Track 32x18, radius full, border 1 border-strong, bg surface; knob 12x12 muted at 2,2; on: border+bg accent, knob accent-ink moved 14 px.
4. Desktop: `Anwendung für den Ton wählen …` (GearIcon) → settings dialog.

Below the main row (`VoiceSessionControls.tsx:144-196`):
- Lost call row: `WarningIcon 16` + `Verbindung verloren` / channel name, button `Erneut versuchen`, IconButton `Schließen` (CloseIcon 16).
- Playback blocked: row padding 6 8, radius 7, bg accent14%+panel, `Der Browser blockiert den Ton.` + primary compact button `Ton starten` (not in desktop app normally).
- One notice: flex gap 8, padding-left 4, 400 12px/1.4 muted, `WarningIcon 15` warning + text (flex 1) + xs IconButton `Hinweis schließen` (CloseIcon 14). Only the first not-dismissed notice shows; a dismissed one comes back when its text changes. Texts from `dockNotices` in priority order (`voice/callBar.ts:20-96`):
  1. `unavailable`: browser `Der Browser hat keinen Live-Ton freigegeben. Das Video läuft weiter.`; desktop without app audio `Diese Desktop-App kann keinen Ton von Anwendungen teilen. Das Video läuft weiter.`; desktop `Der Live-Ton konnte nicht aufgenommen werden ({failed}). Das Video läuft weiter.` or, without a reason, `Der Live-Ton konnte nicht aufgenommen werden. Das Video läuft weiter.`
  2. `Dieser Medienserver unterstützt Live-Ton noch nicht. Das Video läuft weiter.`
  3. `Der Live-Ton wurde beendet. Das Video läuft weiter.`
  4. `Kein Ton von „{silent}“.`
  5. while sharing with no chosen app: `Dein Stream sendet den {scope}.` (scope = `Ton aller Anwendungen außer Gelabber` / `Ton aller Anwendungen`), plus, if the app version carries the call audio, ` Diese App-Version sendet mit dem Live-Ton auch den Ton des Anrufs; ein Update der Desktop-App behebt das.` (`SOURCE_AUDIO_CARRIES_CALL`, `voice/settings.ts:673-674`)
  6. browser only: `Ton ab dem nächsten Live.`

Screen-reader announcements (not visual): `Verbindung verloren`, `Getrennt`, `Verbindung wiederhergestellt`, `Mit {channel ?? "dem Sprachkanal"} verbunden`, `Verbindung wird wiederhergestellt` (`voice/callBar.ts:104-124`).

Focus ring for every button in the room, the dock and the call controls: `outline: 2px solid --lr-focus; outline-offset: 3px` (`room.css:1625-1631`); exceptions: view chips and Kino button/exit offset 2 (`room.css:225-250`), tile/watch-strip buttons offset 2 (`viewer.css:118-123`), sidebar participant rows offset -2 (inset), settings tabs offset -2.

Shortcuts while joined (`callShortcuts.ts`, `pushToTalk.ts:273-314`): Ctrl+Shift+M mute, Ctrl+Shift+D deafen (exactly one of Ctrl/Meta, no Alt, not repeat, not the PTT key).

Toggle semantics (`session.ts:1312-1356`): mute toggles `muted = !(muted||deafened)`; unmuting while deafened also undeafens. Deafen on also mutes; off restores the pre-deafen mute. Each sends `{op:"sig",t:"m"|"d",s,c,on}` on change.

---

## 6. Popovers and dialogs

### 6.1 Popover base (`components/ui/ui.css:636-666`)
Fixed, z 60, min-width 180, max-width min(360, 100vw-16), max-height 100vh-16 (784 @800, 884 @900) with `overflow:auto`, padding 8, border 1 border, radius 10, bg elevated, text colour, shadow-1, 6 px from anchor, 8 px from window edges (`popoverPosition.ts:9-10`); flips above/below to fit. Enter 180ms `--lr-ease` (opacity 0→1, scale .96→1, origin at the anchor); exit 130ms exit ease.

### 6.2 Quick settings `Anrufeinstellungen` (width 312, placement top-end) (`QuickCallSettings.tsx:149-284`, `room.css:1317-1413`)
Column gap 14, padding 6 6 2. Fields (label 600 12px/1.4 muted, gap 6): `Mikrofon` select; (browser only, while joined and mic live) send-level meter 6px high, radius full, track `--lr-border`, fill `--lr-online-text`; `Lautsprecher` select (desktop always, `capabilities.ts:44-50`); `Kamera` select (if camera capable); volume `Gespräche`; volume `Live-Ton` (disabled when stream muted); ChipGroup (aria `Sprachübertragung`) `Sprachaktivierung` / `Push-to-Talk` in the default chip look (`ui.css:463-498`: flex wrap gap 8; chip min-height 32, padding 0 12, border 1 border-strong, radius full, transparent, text colour, 500 13px/1.35; hover bg hover; checked border accent + bg selected + 600; active scale .97); note `Keine Taste festgelegt. Das Mikrofon bleibt stumm.` if PTT without key (400 12px/1.4 muted, margin-top -6); ghost sm full-width button `Alle Einstellungen …` (left aligned, 32 high, padding 0 12, 500 13px, border-top 1 border, radius 0) → closes popover, opens dialog (`room.css:1322-1412`, `QuickCallSettings.tsx:149-284`).
Volume row (`room.css:1373-1395`): grid `1fr auto`, align center, gap 6 (row) 8 (column): label, output `{pct} %` 400 12px/1.4 tabular (text colour), slider (§6.4) spans both columns. With a mute action (`Lautstärke` popover): grid `1fr auto auto`, the button 30x30, padding 0, margin-block -6, `voice-control-compact` look (transparent, muted, radius 7; is-off danger + 16% danger-bg), icon 16.

### 6.3 Stream sound popover `Lautstärke` (width 264): volumes `Gespräche` (disabled when deafened) and `Live-Ton` with a 30x30 mute toggle after the percentage; note `Taubgeschaltet.` when deafened.

### 6.4 Playback slider mapping (`voice/settings.ts:347-387`)
`MAX_PLAYBACK_VOLUME = 4`, slider 0..200, half = 100:
`position(v) = round(v<=1 ? v*100 : 100 + (v-1)/3*100)`; `volume(p) = round100(p<=100 ? p/100 : 1 + (p-100)/100*3)`; fill % = position/2. Range look (`index.css:2071-2121`): height 20, track 4px radius 2, filled accent, rest text@18%; thumb 14px accent with 3px accent@22% ring; disabled opacity .45.

### 6.5 VoiceSettingsDialog `Audio & Video` (`components/VoiceSettingsDialog.tsx`, `components/Modal.tsx`, `ui.css:740-858`, `voice/quality.css`)

| Part | Spec |
|---|---|
| Modal | width calc(100%-32), max-width 640 (`large`), max-height 100vh-32 (768 @800, 868 @900), radius 14, border 1, bg elevated, shadow-2; backdrop `--lr-backdrop`; enter 240ms opacity+scale .97 |
| Header | padding 20 64 12 24; h2 600 16/1.3 `Audio & Video`; close IconButton md 36 top 12 right 12 `CloseIcon 18` (`Schließen`) |
| Body | column, gap 20, padding 8 24 24, 400 14/1.5, scrolls |
| Footer | border-top 1, padding 12 24 16, right aligned: ghost button `Fertig` |
| Tabs | sticky top, flex gap 4, margin -8 0 20, padding-top 8, border-bottom 1, bg elevated. Tab: min-height 36, padding 0 12, 2px transparent bottom border (overlapping by 1), 500 13px muted; hover text; selected: accent bottom border, text, 600. Tabs `Audio`, `Video & Live`, `Diagnose`; ←/→ move |
| Group head | flex baseline, space-between, padding-top 6; h3 600 12px subtle; optional badge padding 2 8, radius 5, bg selected, accent-text 12px |
| Row | grid `136px | 1fr`, column gap 16, row gap 6, padding 12 0, border-bottom 1 (none on last); label padding-top 8, 500 13px text; stacks when the form is ≤480 wide (not at 592) |
| Hints | 400 12px/1.4 muted |
| Slider row | grid `1fr auto`, gap 4/12, min-height 36; output min-width 5ch 500 13px muted tabular right; scale labels 12px subtle |
| Disclosure | border 1, radius 7; summary min-height 44, padding 0 14, 600 13px, 6px chevron (2px borders) rotating -45°→45°, small text right 12px subtle |

Audio tab (`VoiceSettings.tsx:99-338`), desktop wording:
- Group `Eingabe`: `Mikrofon` (select, first entry system default); `Mikrofonverstärkung` slider 0–200 %, scale `0 % / 100 % / 200 %`, hint `100 % fügt keine Verstärkung hinzu.`; `Senden` ChipGroup `Sprachaktivierung` / `Push-to-Talk` + hint `Das Mikrofon sendet, sobald du sprichst.` / `Das Mikrofon sendet nur, solange du die Taste hältst.`; (PTT) `Taste` button `Taste festlegen` / key label / `Taste oder Maustaste drücken … (Esc bricht ab)` + status; (PTT) `Nachlaufzeit` slider step 50 ms, hint `So lange sendet das Mikrofon nach dem Loslassen weiter.` (`PushToTalkSettings.tsx`); `Filter` radio cards: `Verbesserte Sprache` (`Lokale Rauschunterdrückung`), `WebRTC-Filter` (`Rauschunterdrückung und Auto-Gain von WebRTC`), `Original / Musik` (`Stereo sofern unterstützt, ohne Rauschfilter, Auto-Gain oder Sprechpausen-Erkennung`) + status (e.g. `Desktop-App: RNNoise im nativen Kern`) + sm button `Mikrofon testen`.
- Group `Ausgabe`: `Lautsprecher` select; `Gespräche` slider (playback mapping, scale `0 % / 100 % / 400 %`, hint `Über 100 % wird der Ton digital verstärkt.`); `Live-Ton` slider (hint `Der Ton von Lives hat eine eigene Lautstärke. Taub schaltet auch diesen Ton aus.`) + switch `Live-Ton stummschalten`; `Signaltöne` switch line `Töne für Beitreten, Verlassen, Stumm, Taub, Kamera und Live.` + switch `Töne, wenn andere sich stumm schalten` + field `Lautstärke der Signaltöne` (0–1 step .05, hint `Nutzt die Wiedergabelautstärke. Signaltöne spielen auf dem Standard-Ausgabegerät des Systems.`) + sm button `Testton abspielen` (plays `joinSelf`; disabled if sounds off or a volume is 0).
- Disclosure `Echo und WebRTC-Filter` / small `Für Musik oder Lautsprecher`: `Echo` (`Echo-Unterdrückung (AEC). Bei Lautsprechern empfohlen. Für Musik mit Kopfhörern kannst du sie ausschalten.`), `Rauschfilter` (only mode browser: `Rauschunterdrückung von WebRTC.`), `Auto-Gain` (not in original: `Auto-Gain von WebRTC. Gleicht leise und laute Mikrofone auf einen hörbaren Pegel an.`).

Video tab (`VoiceSettings.tsx:340-510`): group `Kamera` (badge `Automatisch` or `1080p · 60 FPS`, 2160 → `4K`): `Gerät`, `Qualität` (selects `Auflösung` [`Auto (bis 1080p)`, 480p…1440p, `4K · 2160p`] and `Bildrate` [`Auto (bis 30 FPS)` only when auto, 15/24/30/45/60 FPS], 2 columns gap 12; apply note `Auf den laufenden Stream angewendet.` / `Bitrate und FPS-Limit gelten sofort. Die Auflösung gilt ab dem nächsten Stream.`); group `Live` (same + hint `Höhere Werte brauchen mehr Rechenleistung und Upload. Die erreichbare Qualität hängt von Quelle, Gerät und Verbindung ab.`; `Ton mitsenden` switch line (desktop text with feature `app-audio-excludes-self`: `Gilt auch während du live bist. Mitgesendet wird der Ton anderer Anwendungen, nie dein Mikrofon oder der Ton des Anrufs. Der Live-Ton wird als Stereo-Musik übertragen, ohne Mikrofonfilter.`), `Ton von` select (default `Alle Anwendungen außer Gelabber`; quiet saved app `{name} (gerade still)`; list from `media_audio_apps`, refreshed on focus)); group `Upload` (badge bitrate or `ohne Limit`): `Sparmodus` switch, `Video-Limit` radio pills (`Ohne Limit`/`Sparmodus (2,5 Mbit/s)`, `Eigenes Limit` + number field `Mbit/s für alle Videos zusammen`), hint; footer note `Änderungen gelten beim nächsten Start und werden, soweit unterstützt, auch auf laufende Streams angewendet.`

Diagnose tab: `VoiceDiagnostics.tsx` (580 lines) — sections `Übertragung` (table), `Mikrofon`, disclosure with `Video auf diesem Gerät`, `Verbindungen`, `Spuren einzeln`, `Verlauf`, `Ereignisse`. NOT specified in detail here (gap).

### 6.6 MicrophoneTest modal `Mikrofon testen` (`voice/MicrophoneTest.tsx`, `room.css:1577-1614`)
Default modal width 448. Column gap 14: `MicIcon 36` accent-text; muted text `Der Test läuft lokal, ohne Aufnahme oder Übertragung.` (desktop); `Eingangspegel` meter, `Nach Verarbeitung` meter (24 high, accent); fieldset `Verarbeitung` (legend 600 13px; radios 500→400 13px, min-height 32, gap 8 16) with the three modes; status line (`Prüfe dein Mikrofon und vergleiche die Verarbeitung.`, `Mikrofon wird gestartet.` …); clipping `Übersteuerung erkannt. Verringere die Mikrofonverstärkung oder den Gerätepegel.`; buttons primary `Test starten` / `Test beenden`, ghost `Schließen`. Desktop: uses `media_audio_monitor` + polls `media_audio_levels` (§12).

---

## 7. Voice channel occupants in the sidebar (`components/ChannelSidebar.tsx:98-147, 723-777, 862-1000`; `index.css:819-937, 2124-2171`)

Voice channel row (36 px virtual row; row box 34 high, radius 7, 14px, muted; hover bg hover + text; active bg selected + text, icon accent-text): `SpeakerIcon` drawn 17x17, gap 10, padding 0 10, name ellipsis. In own call (`is-in-voice`): text colour, icon online-text, tooltip `{name} · verbunden`. Live: badge right-aligned `LIVE` (text "Live" uppercase), padding 1 5, radius 5, `--lr-live`, ink `--lr-on-live`, 700 10px, letter-spacing .4px; tooltip `liveSentence` (e.g. `Mia ist live`).

Participant rows (directly under the channel, sorted by name `localeCompare(…,"de")`, name fallback `Jemand`):

| Part | Spec |
|---|---|
| Row (button) | flex, align center, gap 8, width 100%, height 28 (virtual row 28), padding 0 8 0 36, 13px muted, radius 7; hover bg hover; focus 2px focus outline inset; tooltip `Dein Profil` / `Profil und Lautstärke`; click/right-click → profile dialog |
| Avatar | 20px, initials 9px 600; speaking ring+glow (§1.5) |
| Name | flex 1, ellipsis; speaking → text colour |
| Speaking | `.speaking-bars` only while speaking, between name and Live: 3 bars 3 wide, gap 2, 12 high, radius 1.5, `--lr-speaking`; each scales 35 % → 100 % over 0.9 s ease-in-out, alternate, offsets 0 / -0.3 / -0.6 s; reduced motion rests at 50 / 100 / 70 %. Hidden text `spricht` (title too) |
| Live | `LiveIcon 13`, `--lr-live-text` |
| Locally muted | `SpeakerOffIcon 16` muted (others only) |
| Mute state | `HeadsetOffIcon 13` if deafened else `MicOffIcon 13` if muted, danger; title `taub` / `stumm` |

Speaking here = detector && not muted/deafened.

Accessible name (the web's hidden texts in order; native accesskit label): `{name}`, then `spricht`, `Live`, `taub` or `stumm` as they apply, joined by `, ` (e.g. `Bob, spricht, Live`).

---

## 8. Go Live flow (desktop app)

1. Click Live (menu → `Live gehen`) → `toggleShare()` (`session.ts:1380-1391`): `sharing = true` at once (button turns red), `startLocalVideo("s")`.
2. `captureVideo("s")` (`localVideo.ts:136-195`): await native features; `nativeGetDisplayMedia` (`native/capture.ts:206-264`): `media_source_screen {options:{type:"any", fps, cursor:true, contentHint:"detail"}}` → source id; poll `media_source_state {source}` every 250 ms while `"pending"` (desktop picker open).
3. State `live` → track (width/height from state); `cancelled` (or `failed` after pending without feature `screen-cancel-distinct`) → silently revert `sharing=false`; other failure → toast `Die Bildschirmaufnahme konnte nicht gestartet werden ({reason}).`
4. If sound wanted (`sharesSourceAudio`): `media_source_app_audio {options:{app}}` ("" = every app but Gelabber) → audio track; failures set notice `unavailable` with reason.
5. Profile ladder: constraints applied; fallback → toast (`STREAM_PROFILE_FALLBACK`).
6. On air: play `liveStart`, `localScreen` set (own Live tile appears), `{op:"sig",t:"p",s:serverId,c:channelId,k:"s"}` (and `k:"sa"` for sound) sent (`sendPub`, `voice/sessionState.ts:415-431`), then produce over the send transport.
7. While live, `media_source_state` polled every 1000 ms; any non-`live` state ends the track → `stopLocalVideo("s")`: plays `liveEnd`, sends `t:"u"` for `s` and `sa`, closes producers (`closeSource`), stops source (`media_source_close`).

Camera: `toggleCamera` → `media_source_camera {options:{width,height,fps,device}}` (falls back to `device:""` unless exact); errors → toast `Die Kamera ist nicht verfügbar.` / `Die Kamera unterstützt das Streamprofil nicht.`; on air `cameraOn`, off `cameraOff`.

## 9. Watch (someone else's Live inside the call)
- Lives are only watched in the call's own room; outside, the room/sidebar/LiveHint only say who is live. `LiveHint` pill in channel headers: `Live` badge + names + `in {channel}` + neutral action `Zuschauen` (joins and watches the Lives there; in that call it watches them) / `Zum Kanal` (in another call: only opens the channel). The live toast has the same button and rule (`voice/LiveHint.tsx`, `voice/watchLive.ts`).
- Once the media connection is up, `sourceWatchSupported = true` (`session.ts:1590`). A Live tile then shows the `Zuschauen` overlay until the user watches; `toggleSourceWatch(uid,"s")` (`session.ts:1426-1447`) flips local state and sends seat request `"w" {u, k:"s", on}`; turning off drops the remote stream and its sound. Subscriptions are re-sent after reconnects (`session.ts:1591-1595`).
- While any watched Live plays, the call bar shows the stream-sound button (§5.3 #5) and the focused tile its mute action.

## 10. Call sounds (`voice/callSounds.ts`, `voice/callSoundSynth.ts`)

Cues are synthesised (no files): port `callSoundSynth.ts` (`renderCue(name, rate)`: seeded voices, 1.6 s canvas, room reverb mix .12, normalise to `CUE_LEVELS`, trim tail <0.002, 20 ms fade). Rate 48 kHz for camera cues, 24 kHz otherwise. Playback volume = `callSoundVolume * min(1, outputVolume)`; one cue at a time (a new one cuts the previous). Plays only if `callSounds && callSoundVolume>0 && outputVolume>0` (and `otherMuteSounds` for mute/unmuteOther). While deafened, "remote" cues are suppressed.

| Cue | Peak | Trigger | Remote? | Source |
|---|---|---|---|---|
| `joinSelf` | .34 | own join echo `j` for a new channel; also "Testton abspielen" | yes | `session.ts:410-418`, `VoiceSettings.tsx:284` |
| `joinOther` | .26 | `j` from someone else not already present, not replay | yes | `session.ts:403-406` |
| `leaveOther` | .24 | `l` from a present participant | yes | `session.ts:433` |
| `leaveSelf` | .32 | `leaveVoice()` after a confirmed join, not when deafened | (plays) | `session.ts:1285` |
| `muteSelf` / `unmuteSelf` | .30 | `toggleMute` | no | `session.ts:1326` |
| `muteOther` / `unmuteOther` | .14 | `m` flag change of a peer settled ≥2000 ms | yes | `session.ts:441-451` |
| `deafen` / `undeafen` | .30 | `toggleDeafen` | no | `session.ts:1354` |
| `liveStart` / `liveEnd` | .32/.26 | own Live on air/off (`localVideo.ts:475,512`); others' `p`/`u` of `s` (start only if settled) | own no / others yes | `session.ts:458-464` |
| `cameraOn` / `cameraOff` | .24/.22 | own camera on air/off | no | `localVideo.ts:475,512` |
| `message` | .18 | message notification (own switch `messageSound`) | — | `useMessageToasts.ts:297` |
| `liveStart` via message player | | someone went live elsewhere (`useLiveNotices.ts:116`) | | |
| `viewerJoin` / `viewerLeave` | .18/.15 | defined, no trigger in the client | | |

Desktop: cues play on the system default output (no sink selection).

## 11. Voice signalling used by this UI (gateway WebSocket, `op:"sig"`)
Sent: `{op:"sig",t:"j",s,c}` join; `t:"l"` leave; `t:"m"|"d", on` mute/deafen; `t:"p"|"u", k:"v"|"s"|"sa"` publish/unpublish (`sessionState.ts:415-431`). Received in room: `j`, `l`, `m`(on), `p`/`u`(k). Media seat requests: `"j" {tk, v}` → router caps; `"w" {u,k,on}`; mediasoup requests `capabilities`, `closeProducer`, `closeTransport`, `restartIce`, `consumerReady`, `consumerFailed` (`mediasoupConnection.ts`). Full protocol belongs to the protocol spec.

---

## 12. Native media core: commands the web client calls (the egui app calls `desktop/core` directly)

The page invokes Tauri commands (`voice/native/bridge.ts:53-58`); each was a thin wrapper in `desktop/app/src/media.rs` (the Tauri app, removed in 0.9; see git history) over `gelabber_media_core` (`desktop/core`). Handles are `u64` registry keys in the app; the egui app can hold the core objects directly. "Channel" = an ordered event callback.

| Command | Args (JSON names) | Returns | Core call (`desktop/app/src/media.rs`) | Web caller |
|---|---|---|---|---|
| `media_info` | — | `{abi, version, platform, features[], videoCodecs}`; features on Linux/Windows: `screen, camera, app-audio, app-audio-excludes-self, video-frames, vp9-svc` + `h264-hardware`, `av1-hardware`, `av1-hardware-decode` from codecs | `video_codecs()` `:545-570` | `native/features.ts:146` |
| `media_audio_devices` | — | `{inputs:[{id,name}], outputs:[…], input, output}` | `engine.audio_devices()` `:572` | `native/capture.ts:21` |
| `media_video_devices` | — | `[{id,name}]` | `engine.video_devices()` `:733` | `capture.ts:22` |
| `media_audio_configure` | `options: {input?: id}` / `{output?: id}` / `{inputGain?: n}` | — | `engine.configure_audio` `:577` | `capture.ts:41,45,347`; `tracks.ts:190` |
| `media_audio_levels` | — | `{input, processed, clipping, blocks}` (0..100 meters) | `engine.audio_levels()` `:582` | `microphoneTest.ts:79`; `speaking.ts:263` (own speaking: `processed / NATIVE_METER_SCALE`, only if `blocks` moved) |
| `media_audio_monitor` | `options: {processingMode, inputGain}` to start, `null` to stop | — | `engine.monitor_audio` `:589` | `microphoneTest.ts:50,59` |
| `media_audio_apps` | — | `[{id,name,streams}]` | `engine.audio_apps()` `:717` | `capture.ts:151` |
| `media_source_microphone` | `options: {processingMode, echoCancellation, noiseSuppression, autoGainControl, inputGain}` | source id | `Source::microphone` `:698` | `capture.ts:306` |
| `media_source_camera` | `options: {width, height, fps, device}` | source id | `Source::camera` `:741` | `capture.ts:100,110` |
| `media_source_screen` | `options: {type:"any", fps, cursor:true, contentHint:"detail"}` | source id (opens desktop picker) | `Source::screen` `:708` | `capture.ts:210` |
| `media_source_app_audio` | `options: {app}` | source id | `Source::app_audio` `:724` | `capture.ts:164` |
| `media_source_state` | `source` | `{state:"pending"|"live"|"cancelled"|"ended"|"failed", width?, height?, streams?}` | `source.state()` `:749` | `capture.ts:119,175,217,245` |
| `media_source_set_enabled` | `source, enabled` | — | `source.set_enabled` `:754` | `tracks.ts:59` (track.enabled) |
| `media_source_close` | `source` | — | `close_source` `:767` | `tracks.ts:112`, `capture.ts:223,227` |
| `media_device_load` | `capabilities` (router RTP caps) | `{device, rtpCapabilities, canProduce:{audio,video}}` | `Device::new(engine).load` `:618` | `native/device.ts:500` |
| `media_device_close` | `device` | — | `:634` | `device.ts:539` |
| `media_transport_create` | `device, direction:"send"|"recv", options:{id, iceParameters, iceCandidates, dtlsParameters, iceServers?, iceTransportPolicy?}, events: Channel` | `{transport, id}`; events `{type:"connect",request,dtlsParameters}`, `{type:"produce",request,kind,rtpParameters,appData}`, `{type:"connectionstatechange",state}` | `open_transport` `:644` | `device.ts:260` |
| `media_transport_respond` | `transport, request, result? | error?` (exactly one; produce result `{id}`) | — | `transport.answer` `:659` | `device.ts:284` |
| `media_transport_restart_ice` | `transport, iceParameters` | — | `:676` | `device.ts:465` |
| `media_transport_stats` | `transport` | RTC stats array | `:686` | `device.ts:473` |
| `media_transport_close` | `transport` | — | `:692` | `device.ts:270,481` |
| `media_produce` | `transport, source, options:{codec? (mimeType), encodings?:[{scaleResolutionDownBy?, scalabilityMode?, maxBitrate?, maxFramerate?, active?}], codecOptions?}` | `{producer, id, rtpParameters}` | `transport.produce` `:772` | `device.ts:375` |
| `media_producer_pause` | `producer, paused` | — | `:797` | `device.ts:113` |
| `media_producer_replace_source` | `producer, source` | — | `:808` | `device.ts:140` |
| `media_producer_parameters` | `producer` | `{encodings[], transactionId?}` | `:826` | `device.ts:106` |
| `media_producer_set_parameters` | `producer, parameters:{encodings}` | — | `:833` | `device.ts:93` |
| `media_producer_stats` | `producer` | stats array | `:850` | `device.ts:148` |
| `media_producer_close` | `producer` | — | `:856` | `device.ts:155` |
| `media_consume` | `transport, params:{id, producerId, kind, rtpParameters}` | `{consumer, id}`; audio starts at volume 0 | `transport.consume` + `set_volume(0)` `:872` | `device.ts:435` |
| `media_consumer_pause` | `consumer, paused` | — | `:900` | `device.ts:184` |
| `media_consumer_set_volume` | `consumer, volume` (0..2) | — | `:912` | `tracks.ts:279` (element volume = output × per-user × mute) |
| `media_consumer_stats` | `consumer` | `{rtc?:[], audioLevel?, samplesPlayed?}` | `:923` | `device.ts:198,207` (remote speaking via `audioLevel`) |
| `media_consumer_close` | `consumer` | — | `:1052` | `device.ts:221` |
| `media_view_open` | `consumer? | source?` (exactly one), `maxWidth?, maxHeight?, maxFps?` (physical px; default 1280x720) | `{view}` | `frames.open` `:1110` | `native/videoFeed.ts:323` |
| `media_view_configure` | `view, maxWidth, maxHeight, maxFps?` | — | `:1130` | `videoFeed.ts:525` |
| `media_view_frame` | `view, after?` (last seq) | binary packet "GFR1": 32-byte LE header (magic u32 = `0x31524647`, header len u16 @4 (planes start there; may be >32), max side 16384, chroma planes `((w+1)>>1) x ((h+1)>>1)`, format u8 @6 = 0 I420, flags u8 @7: bit0 BT.709, bits1-2 rotation/90, width u32 @8, height u32 @12, seq u32 @16, timestamp µs i64 @24) + Y, U, V planes (limited range) | `view.pull` `:1148` | `videoFeed.ts:359`; parse `native/frames.ts:35-75` |
| `media_view_close` | `view` | — | `:1170` | `videoFeed.ts:549,553` |
| `media_viewer_open` | `consumer, title:"{label} – Gelabber", events: Channel` (`{type:"height",height}`, `{type:"closed"}`) | — | `Viewer::open` `:1021` | `native/viewer.ts:88` |
| `media_viewer_close` | `consumer` | — | `:1047` | `viewer.ts:101` |
| `ptt_bind` / `ptt_unbind` | `key` (KeyboardEvent.code or `Mouse4/5`), `events` (`{type:"bound",trigger}`, …) | — | global PTT hook (not media) | `voice/pushToTalk.ts:169,198` |

Notes for egui: limited-quality fallback (320x180 @15 fps, `videoFeed.ts:61`) is a webview transport issue and does not apply when drawing core frames directly; frame conversion matrices BT.601 `[1.402, .3441, .7141, 1.772]`, BT.709 `[1.5748, .1873, .4681, 1.8556]`, luma scale 255/219, chroma 255/224 (`frames.ts:78-81`) — or upload I420 to a wgpu texture and convert in a shader. Request view size = drawn tile size in physical pixels (HiDPI aware).

## 13. Strings (verbatim, not listed above)
`Bühne` (stage aria), `Ohne Kamera`, `Teilnehmer im Sprachkanal`, `Übertragungen`, `Ansicht`, `Profil von {name}`, `Optionen für {name}`, `Profil ansehen`, `Lokale Lautstärke` (volume chip title), `Lokal stummgeschaltet`, `Vollbild: {label}`, `Steuerung: {label}`, `Live-Ton-Lautstärke`, `Live-Ton wieder einschalten`, `Keine Vorschau in der Desktop-App.`, `Im Fenster ansehen`, `Video wird geladen …`, `Hinweis schließen`, `Aktive Medien`, `Im Anruf`, `Verbindung`, `Du sendest`, `Du empfängst`, `Pegel, den die anderen hören`, `Zum Sprechen gedrückt halten` / `Du sprichst` (touch PTT), VoiceStateIcons titles `Taub in {channel}`, `Stumm in {channel}`, `In {channel}`.

## 14. Icon geometry (`components/Icons.tsx`, 24-unit viewBox, stroke 2 round)

| Icon (line) | SVG |
|---|---|
| SpeakerIcon (29) | `M11 5 6 9H3v6h3l5 4V5Z` · `M15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13` |
| SpeakerOffIcon (273) | `M11 5 6 9H3v6h3l5 4V5Z` · `m22 9-6 6M16 9l6 6` |
| MicIcon (97) | `M12 3a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3Z` · `M19 10v1a7 7 0 0 1-14 0v-1M12 18v3` |
| MicOffIcon (106) | `M9 9v0a3 3 0 0 0 3 3m3-3V6a3 3 0 0 0-4.4-2.6` · `M19 10v1a7 7 0 0 1-9.9 6.4M5 10v1a7 7 0 0 0 4 6.3M12 18v3M4 4l16 16` |
| HeadsetIcon (115) | `M4 14v-2a8 8 0 0 1 16 0v2` · `M4 14a2 2 0 0 0-2 2v1a2 2 0 0 0 2 2h1v-5H4Z` · `M20 14a2 2 0 0 1 2 2v1a2 2 0 0 1-2 2h-1v-5h1Z` |
| HeadsetOffIcon (125) | `M4 14v-2a8 8 0 0 1 12.5-6.6M20 11v3` · `M4 14a2 2 0 0 0-2 2v1a2 2 0 0 0 2 2h1v-5H4Z` · `M20 14h-1v2.5M4 4l16 16` |
| CameraIcon (164) | `M15 9 22 6v12l-7-3` · rect x2 y7 w13 h10 rx2 |
| CameraOffIcon (173) | `M15 9 22 6v12l-7-3M4 4l16 16` · rect x2 y7 w13 h10 rx2 |
| LiveIcon (155) | filled circle cx12 cy12 r3 (no stroke) · `M5 12a7 7 0 0 1 2-4.9M19 12a7 7 0 0 0-2-4.9M8.5 7.5a9 9 0 0 1 7 0M8.5 16.5a9 9 0 0 0 7 0` |
| ChevronIcon (182) | `m6 9 6 6 6-6` |
| ExpandIcon (198) | `M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7` |
| CollapseIcon (206) | `M21 9h-6V3M3 15h6v6M15 9l6-6M9 15l-6 6` |
| CloseIcon (240) | `M6 6l12 12M18 6 6 18` |
| HangUpIcon (248) | `M3 14.5c5.5-5 12.5-5 18 0l-2.5 2.5-3.5-1.5v-2.5c-2-.7-4-.7-6 0v2.5L5.5 17Z` |
| SignalIcon (256) | `M5 19v-3M10 19v-7M15 19V8M20 19V5` |
| GridIcon (304) | 4 rects: (3,3,8,8,rx1.5) (13,3) (3,13) (13,13) |
| FocusIcon (316) | rect 3,3,18x12 rx1.5; rects 3,17 / 9.5,17 / 16,17 each 5x4 rx1 |
| TheaterIcon (328) | `M8 3H5a2 2 0 0 0-2 2v3M21 8V5a2 2 0 0 0-2-2h-3M3 16v3a2 2 0 0 0 2 2h3M16 21h3a2 2 0 0 0 2-2v-3` |
| TheaterExitIcon (337) | `M8 3v3a2 2 0 0 1-2 2H3M21 8h-3a2 2 0 0 1-2-2V3M3 16h3a2 2 0 0 1 2 2v3M16 21v-3a2 2 0 0 1 2-2h3` |
| MoreIcon (390) | circles r1 at (5,12) (12,12) (19,12) |
| PinIcon (409) | `M12 17v5M9 3h6l-1 6 4 4v2H6v-2l4-4Z` |
| WarningIcon (425) | `M10.3 3.9 2.4 17.6A2 2 0 0 0 4.1 20.6h15.8a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z` · `M12 9v4M12 17h.01` |
| EyeOffIcon (452) | `M9.9 5.2A9.7 9.7 0 0 1 12 5c6.4 0 10 7 10 7a17 17 0 0 1-2.6 3.5M6.6 6.6C3.9 8.3 2 12 2 12s3.6 7 10 7a9.6 9.6 0 0 0 5.4-1.6` · `M9.9 9.9a3 3 0 0 0 4.2 4.2M3 3l18 18` |
| GearIcon (46) | circle cx12 cy12 r3 · `M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z` |

## 15. Known gaps / judgement calls
- Diagnose tab (`VoiceDiagnostics.tsx`) only outlined (§6.5).
- Text widths depend on Inter shaping; no per-glyph metrics given.
- Call-bar speaking avatar ring: cascade order makes the 2px ring panel-coloured with the green glow on top; verify against a screenshot.
- Select, Switch, RadioCard, Checkbox, Button primitives are referenced by variant only (`components/ui/ui.css`); their full spec belongs to the UI-primitives spec.
- Mobile/touch variants (≤640, `pointer:coarse`) are noted but not specified for the desktop app.
- Review pass (checked against source): all §1.1 token hexes for both presets were recomputed with the real `fixContrast` + `themeVariables`; every §3.3 rect was recomputed with the real `stage.ts`; derived composites in §1.1 were recomputed by hand. All matched. The corrections from this pass are in §2 (≤1130 member drawer), §3.5 (strip person colours and ring, "(du)" weight), §3.6, §3.7 (Kino label weight, focus actions hover), §5.2–5.3 (people tooltip, compact signal bars, mic problem popover, notices, focus rings), §6.1–6.2, §8, §12 (frame magic) and §14 (gear path).
