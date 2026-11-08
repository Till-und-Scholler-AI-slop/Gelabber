# UI spec: chat (ui-chat)

Target: egui 0.37 native client that matches the React web client pixel-for-pixel (SSIM >= 0.90).
Every value below was read from the source; file:line refs are relative to `web/src/`.
Units are CSS px at the default UI scale "Kompakt" (`--lr-ui-scale: 1`, `interface/preferences.ts:21`, `index.css:108-109,193-195`).
egui points == CSS px at scale 1 (set `pixels_per_point` to the OS scale factor only).

Conventions:
- `mix(A p%, B)` = CSS `color-mix(in srgb, A p%, B)` = per channel `A*p + B*(1-p)`. Already resolved below for both default themes.
- "alpha X" = `color-mix(..., X p%, transparent)` = colour X at opacity p. The table also gives the result composited on `--lr-bg`.
- Line height: Tailwind preflight sets `html { line-height: 1.5 }`, so every element without its own `line-height` uses 1.5 x font-size (`index.css:1` imports tailwindcss).
- `box-sizing: border-box` everywhere (preflight): widths and heights include padding and border.
- Fonts: `"Gelabber Inter"` = Inter 4.1 Variable (`fonts/InterVariable-latin.woff2`, `-latin-ext`, `-Italic-latin`; `index.css:46-78`). Weights used in chat: 400, 500, 600, 650, 700, 800. egui does not interpolate variable axes: ship static instances (or instance the variable TTF) at 400/500/600/650/700/800 plus Italic 400 (and Italic 700 for `***x***`). Monospace in code: CSS stack `ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace` (`components/chat.css:526-530`); on Linux that resolves to the system `monospace` (normally DejaVu Sans Mono). Colour emoji: the system emoji font (Noto Color Emoji on Linux).
- Text rendering: `-webkit-font-smoothing: antialiased; text-rendering: optimizeLegibility; font-synthesis: none` (`index.css:88-90`). Never synthesize bold/italic.

---

## 1. Theme tokens (default presets)

Source: `theme/defaults.json`, `theme/presets.ts:5-21,227` (presets pass through `fixContrast`, which changes nothing for these two), `theme/model.ts:187-238` (`derivedColors`), `theme/model.ts:295-391` (`themeVariables`). Both presets use style `clear`.

### 1.1 Base + derived variables

| CSS variable | Dark ("Gelabber Dunkel") | Light ("Gelabber Hell") | Derivation |
|---|---|---|---|
| `--lr-bg` | `#17191a` | `#f8f5f0` | background |
| `--lr-panel` | `#1d2021` | `#f4f0e9` | panel |
| `--lr-surface` | `#272b2d` | `#ece7df` | surface |
| `--lr-rail` | `#131617` | `#eee9e1` | rail |
| `--lr-elevated` | `#252829` | `#fcfbf9` | dark: mix(panel, text 4%); light: mix(bg, #fff 60%) |
| `--lr-text` | `#eef0ee` | `#252829` | text |
| `--lr-muted` | `#a4abb1` | `#5c646b` | ensureContrast(muted) |
| `--lr-subtle` | `#8c9295` | `#65696c` | |
| `--lr-border` | `#2c3032` | `#dcd6cc` | border |
| `--lr-border-strong` | `#717475` | `#86837e` | |
| `--lr-accent` | `#f3b752` | `#e8a638` | accent (fill) |
| `--lr-accent-ink` | `#111111` | `#111111` | text on accent |
| `--lr-accent-text` | `#f3b752` | `#836026` | accent used as text/icon colour |
| `--lr-focus` | `#f3b752` | `#836026` | = accent-text |
| `--lr-hover` | `#2e3131` | `#e3e0da` | mix(panel, text 8%) |
| `--lr-selected` | `#443b2a` | `#f2e3c9` | mix(panel, accent 18%) |
| `--lr-online` / `--lr-online-text` | `#85d69b` / `#85d69b` | `#287a45` / `#277643` | |
| `--lr-warning` | `#d8a657` | `#a27e44` | |
| `--lr-danger` | `#f39b98` | `#a12b29` | danger text |
| `--lr-danger-bg` | `#8f302d` | `#b33b38` | danger fill |
| `--lr-on-danger` | `#ffffff` | `#ffffff` | |
| `--lr-backdrop` | `rgba(9,12,13,0.72)` | `rgba(37,40,41,0.45)` | modal backdrop |
| `--lr-shadow-1` | `0 8px 24px rgba(0,0,0,0.32)` | `0 8px 24px rgba(60,45,20,0.12)` | popovers |
| `--lr-shadow-2` | `0 18px 48px rgba(0,0,0,0.5)` | `0 18px 48px rgba(60,45,20,0.18)` | modals |

Radii (style `clear`, `theme/model.ts:290-294,327,374-381`): `--lr-radius-xs 3px`, `--lr-radius-sm`/`--lr-radius-small 5px`, `--lr-radius`/`--lr-radius-md 7px`, `--lr-radius-lg 10px`, `--lr-radius-xl 14px`, `--lr-radius-full 999px` (`index.css:127`). Tailwind bare `rounded` = 4px (no `--radius` override).

Tailwind neutral scale (used only by EmojiPicker/GifPicker utilities; `theme/model.ts:299-334`):
dark: 50-300 = text `#eef0ee`, 400-600 = `#a4abb1`, 700 = `#2c3032`, 800 = `#272b2d`, 900 = `#1d2021`, 950 = `#17191a`.
light: 50 `#f8f5f0`, 100 `#f4f0e9`, 200 `#ece7df`, 300 `#dcd6cc`, 400-600 `#5c646b`, 700-950 `#252829`.
The `dark:` variant is active when `<html class="dark">` (`theme/theme.ts:96`, `index.css:3`).

### 1.2 Avatar / tile tints (`--lr-tile-1..8`)

| # | Dark | Light |
|---|---|---|
| 1 | `#634336` | `#ead2c8` |
| 2 | `#635636` | `#eae0c8` |
| 3 | `#4e6336` | `#daeac8` |
| 4 | `#366349` | `#c8ead6` |
| 5 | `#365d63` | `#c8e5ea` |
| 6 | `#364963` | `#c8d6ea` |
| 7 | `#433663` | `#d2c8ea` |
| 8 | `#633653` | `#eac8dd` |

Person tint (`components/initials.ts:98-108`): FNV-1a over UTF-16 code points of the name:
```
h = 0x811c9dc5
for cp in name.code_points(): h ^= cp; h = imul(h, 0x01000193)   // u32 wrapping
h ^= h >> 16; h = imul(h, 0x45d9f3b); h ^= h >> 16
tile = (h % 8) + 1
```
(JS `for…of` iterates code points; `codePointAt(0)` gives the full scalar value.)
Initials (`components/initials.ts:2-9`): split trimmed name on whitespace, first code point of the first two words, uppercased; `?` if blank.

### 1.3 Resolved chat mixes (computed for both presets)

| Use | CSS | Dark | Light |
|---|---|---|---|
| Row hover / focus-within bg | mix(bg, text 3.5%) `chat.css:307-310` | `#1f2121` | `#f1eee9` |
| Message body text | mix(text 90%, muted) `chat.css:394` | `#e7e9e8` | `#2b2e30` |
| Reply author | mix(text 85%, muted) `chat.css:367` | `#e3e6e5` | `#2d3133` |
| Preview description | mix(text 85%, muted) `chat.css:650` | `#e3e6e5` | `#2d3133` |
| Highlighted row (jump target) | alpha accent 14% `chat.css:384-387` | `#362f22` | `#f6ead6` |
| Failed row bg | mix(danger-bg 8%, bg) `chat.css:445` | `#211b1c` | `#f2e6e1` |
| Failed row hover | mix(danger-bg 12%, bg) `chat.css:450` | `#251c1c` | `#f0dfda` |
| Mentioned row bg | mix(warning 9%, bg) `chat.css:2073` | `#28261f` | `#f0eae1` |
| Mentioned row hover | mix(warning 13%, bg) `chat.css:2078` | `#302b22` | `#ede6da` |
| Link underline | alpha accent 45% `chat.css:517` | `rgba(243,183,82,.45)` ≈ `#7a6033` on bg | `rgba(232,166,56,.45)` ≈ `#f1d19d` |
| Quote bar | alpha text 25% `chat.css:541` | ≈ `#4d4f4f` | ≈ `#c3c2be` |
| Spoiler hidden bg | mix(text 22%, bg) `chat.css:587` | `#464849` | `#cac8c4` |
| Spoiler revealed bg | alpha text 8% `chat.css:596` | ≈ `#282a2b` | ≈ `#e7e5e0` |
| Preview left border | mix(accent 70%, border) `chat.css:616-617` | `#b78f48` | `#e4b464` |
| Composer/search placeholder | mix(subtle 80%, panel) `chat.css:79,1440` | `#74797c` | `#868889` |
| Header search hover border | mix(border 82%, text 18%) `chat.css:85` | `#4f5354` | `#bbb7af` |
| Focused field border | mix(accent 70%, border) `chat.css:92,1399` | `#b78f48` | `#e4b464` |
| Drop-target outline | alpha accent 70% `chat.css:1404` | ≈ `#b18841` | ≈ `#edbe6f` |
| Upload badge bg | alpha panel 85% `chat.css:1276` | `rgba(29,32,33,.85)` | `rgba(244,240,233,.85)` |
| Send-summary bg | mix(danger-bg 10%, panel) `chat.css:1630` | `#282222` | `#eeded7` |
| Reaction hover border | mix(border 78%, text 22%) `chat.css:1785` | `#575a5b` | `#b4b0a8` |
| Reaction mine border | mix(accent 60%, border) `chat.css:1788` | `#a38145` | `#e3b973` |
| "Neu" divider line | alpha accent 60% `chat.css:1970` | ≈ `#9b783c` | ≈ `#eec682` |
| Unread bar bottom border | mix(accent 35%, border) `chat.css:248-249` | `#725f3d` | `#e0c598` |
| Mention chip bg | alpha accent 18% `chat.css:2064` | ≈ `#3f3524` (on hover row `#453c2a`) | ≈ `#f5e7cf` (hover `#efe1c9`) |
| Own mention chip bg | alpha warning 26% `chat.css:2069` | ≈ `#493e2a` (on mentioned row `#56472e`) | ≈ `#e2d6c3` (on mentioned row `#dcceb8`) |
| Jump pill shadow | `0 6px 20px #0000003d` (= rgba(0,0,0,.24)) `chat.css:209` | same | same |
| Hover action bar shadow | `0 4px 14px #00000029` (= rgba(0,0,0,.16)) `chat.css:776` | same | same |

Draw alpha colours as real alpha over whatever is beneath (row hover changes the backdrop).
Rounding: channels are rounded half-up (x.5 → up), as the browser does; e.g. focused border dark G = 183*0.7 + 48*0.3 = 142.5 → 143 (`#b78f48`), light message text R = 42.5 → 43 (`#2b2e30`). A banker's-rounding implementation gives `#b78e48` / `#2a2e30` — use half-up.
Also derived (live pill, §3.1): mix(danger-bg 45%, border) `#593030` / `#ca9089`; mix(danger-bg 10%, bg) `#231b1c` / `#f1e2de`. Context target (§24.1): alpha accent 12% ≈ `#312c21` / `#f6ecda` on bg.

---

## 2. Type scale and motion

`index.css:98-107`:

| Token | Weight / size / line-height |
|---|---|
| `--lr-type-caps` | 600 11px / 1.3 |
| `--lr-type-meta` | 400 12px / 1.4 |
| `--lr-type-ui` | 500 13px / 1.35 |
| `--lr-type-body` | 400 14px / 1.5 |
| `--lr-type-title-sm` | 600 16px / 1.3 |
| `--lr-type-title` | 600 20px / 1.25 |

Motion (`index.css:133-144`): micro 100ms, fast 120ms, standard 180ms, large 240ms, exit 130ms; `--lr-ease` = cubic-bezier(0.2,0.8,0.2,1) (enter), `--lr-ease-standard` = cubic-bezier(0.2,0,0,1), press scale 0.97. Reduced motion (system or app switch) sets all durations to 0 (`index.css:173-192`).
Hover colour transitions: rows, message actions, menu items, reactions, composer icons, jump pill animate `background-color, border-color, color` over 100ms standard ease (`index.css:247-279`).

---

## 3. MessagePane layout

Source: `components/MessagePane.tsx:391-640`, `components/chat.css:95-100`, pages `pages/ChannelPage.tsx:86-150`, `pages/DmChannelPage.tsx:94-127`.

Vertical stack (flex column, `min-h-0 flex-1`, bg `--lr-bg`):

| # | Part | Height | Ref |
|---|---|---|---|
| 1 | Header `.lr-channel-header` | 52 (incl. 1px bottom border) | `chat.css:4-15`, `index.css:150` |
| 2 | `notice` (live-hint strip; hidden ≥801px) | 0 on desktop | `chat.css:1651-1660,2046-2050` |
| 3 | Inline notice "Lesestatus nicht gespeichert." (only on read-state error) | 8 margin + 6+18+6 | `MessagePane.tsx:428-435`, `chat.css:101-117` |
| 4 | List area (relative wrapper; holds unread bar, list, search overlay, jump pill) | flex 1 | `MessagePane.tsx:439-609` |
| 5 | `footer` = TypingBar | 28 fixed | `ws/live.ts:23`, `components/TypingBar.tsx` |
| 6 | `.lr-toast-space` | 0 without toasts; else toastH + min(toastH, 12) | `chat.css:1369-1375` |
| 7 | Composer `.lr-composer` | 60 idle (46 field + 14 bottom pad) | `chat.css:1361-1364` |

Pane widths (row/composer layout depends on them; shell columns from `index.css:146-149`: rail 64 + sidebar 224 = 288 left, member panel 248 right, shown in server channels at ≥1131px; DMs have no member panel):

| Viewport | Server channel pane | DM pane | Pane height (no call) | List area height (server/DM, idle composer, no notices) |
|---|---|---|---|---|
| 1280x800 | 744 | 992 | 800 | 800-52-28-60 = 660 |
| 1600x900 | 1064 | 1312 | 900 | 900-52-28-60 = 760 |

With an active call (`.has-active-media`) the workspace gets `padding-bottom: 64px` (`index.css:392-395`), so subtract 64 from the pane height.
Message text column width = pane width - 20 - 20 (row padding) - 36 (avatar) - 12 (gap) = pane - 88 → 656 / 976 (server) and 904 / 1224 (DM).
No breakpoint in this spec changes between 1280 and 1600; all `max-width: 800/640/520px` and `pointer: coarse` rules (Section 25) are inactive at both sizes.

### 3.1 Header (part owned by the pane)

`chat.css:4-28,45-93,1931-1939`, `MessagePane.tsx:394-426`, `index.css:344-362`.
- `.lr-channel-header`: flex row, align center, gap 10, height 52, padding `0 12 0 20`, border-bottom 1px `--lr-border`, bg `--lr-bg`, colour `--lr-text`.
- Server title: `.lr-channel-title-name` flex gap 10: `HashIcon` 18px (`SpeakerIcon` for voice) colour `--lr-muted`, then `h1` 15px / 650 / letter-spacing -0.1px, single line, ellipsis. (Server name line is hidden ≥801px.)
- DM title: `.dm-peer-button` inline-flex gap 10, margin-left -6, padding 4 6, radius 7, hover bg `--lr-hover` (button height 40); tooltip "Profil von {peer}"; PresenceAvatar + `h1` (`pages/DmChannelPage.tsx:97-110`, `chat.css:29-42`).
  - PresenceAvatar (`components/PresenceAvatar.tsx:12-41`, `index.css:1308-1331`): Avatar `sm` 32px circle (initials 12px/600 on person tint) + dot absolute right 0 bottom 0, **10x10** (23% of 32 = 7.4, clamped by min 10), border 2px `--lr-bg` (border-box, so 6px fill), circle. Fill: online `o` → `--lr-online` (`#85d69b`/`#287a45`), away `i` → `--lr-warning` (`#d8a657`/`#a27e44`), offline `x` → `--lr-muted`. Tooltip "Online" / "Abwesend" / "Offline".
- Live pill (server text channels only, when someone in the server is live; part of the title fragment, `pages/ChannelPage.tsx:55-65`, `voice/LiveHint.tsx:72-89`, `chat.css:2004-2045,1689-1703`): `.lr-live-pill` inline-flex, align center, gap 8, height 30, **margin-left auto** (the search after it then gets margin-left 0), padding 0 4 0 6, border 1px mix(danger-bg 45%, border) = `#593030` / `#ca9089`, radius 15, bg mix(danger-bg 10%, bg) = `#231b1c` / `#f1e2de`, 12px, nowrap. Children:
  1. Badge `.lr-live-badge` "LIVE" (text "Live", uppercase): inline-grid centred, height 18, padding 0 6, radius 9, bg `--lr-danger-bg` (`#8f302d`/`#b33b38`), `#fff`, 10px/700, letter-spacing 0.4px (margin-right 0 inside the pill).
  2. Link (ellipsis, colour text, underline on hover): `nameList(who)` + (" in {voice channel}" if known) + (" +{n}" for further live channels). `who` lists "Du" first if I am live, unknown members "Jemand"; nameList: "A", "A und B", "A, B und C", "A, B und {n} weitere" (`voice/roomSummary.ts:2-20`). Tooltip `liveSentence` + where: "Mia ist live in Lounge", "Mia und Lena sind live…", "Du bist live", "Du und Mia seid live".
  3. Button (only if I am not in that call, may join voice and someone other than me is live): "Beitreten" (not in any call) / "Zum Kanal" (in another call); padding 3 10, radius 12, bg `--lr-danger-bg`, `#fff`, 12px/650 (height 24).
  Below 801px the pill is hidden and a `.lr-live-hint` strip under the header is used instead (not on desktop).
- Search field `.lr-header-search`: margin-left auto, width 200 (260 focused or non-empty, width animates 100ms), height 30, padding 0 10, gap 8, border 1px `--lr-border`, radius 7, bg `--lr-panel`, colour `--lr-muted`, 13px. `SearchIcon` 16. Placeholder "Suchen" in placeholder colour (§1.3). Right: `kbd` "Strg F" (mac "⌘ F") — margin-left auto, padding 0 5, border 1px `--lr-border`, radius 5, 11px, muted; hidden while text present. Hover: colour text, border `#4f5354`/`#bbb7af`. Focus: border `#b78f48`/`#e4b464`. Tooltip "Wörter oder eine Phrase in Anführungszeichen". Strg/⌘+F focuses it; Esc clears, then blurs.
- Then notification bell and member toggle (`.lr-header-icon` 32x32, radius 7, muted, hover/on bg `--lr-hover` + text; `chat.css:1982-1995`). Header gap 10 between all children.
  - Bell (server channels and DMs, `components/NotificationSettingsDialog.tsx:318-327`): `BellIcon` **16** (`M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9`; `M10.3 21a1.94 1.94 0 0 0 3.4 0`) or, when muted, `BellOffIcon` 16 (`M8.7 3A6 6 0 0 1 18 8a21.3 21.3 0 0 0 .6 5M17 17H3s3-2 3-9a4.67 4.67 0 0 1 .3-1.7`; `M10.3 21a1.94 1.94 0 0 0 3.4 0M2 2l20 20`). Tooltip "Benachrichtigungen" / "Stummgeschaltet".
  - Member toggle (server channels only, `components/MemberPanel.tsx:23-40`): `UsersIcon` **18** (circle (9,7.5) r3.5; `M2.5 20a6.5 6.5 0 0 1 13 0M16 4.5a3.5 3.5 0 0 1 0 6.5M18 14a6 6 0 0 1 3.5 6`). `.is-on` (bg `--lr-hover`, colour text) while the panel is shown — i.e. ON by default. Tooltip "Mitgliederliste ausblenden" / "Mitgliederliste einblenden". Hidden ≤1130px.
  - Header x-layout at server pane 744 (no live pill): 20 pad | title | auto | search 200 | 10 | bell 32 | 10 | toggle 32 | 12 pad → search left edge = 744-12-32-10-32-10-200 = 448.

---

## 4. Message list container

`MessagePane.tsx:643-879`, `components/useMessageScroll.ts`.

- Scroll container: flex column, `overflow-y: auto`, `overscroll-behavior: contain`, no scroll anchoring. Content box has `margin-top: auto` so a short history sits at the bottom (`MessagePane.tsx:809-815`).
- Virtualised (overscan 12, `useMessageScroll.ts:96-102`). Each virtual item = [history start if first & nothing older] + [day divider] + ["Neu" divider] + MessageRow, stacked with no extra gaps (`MessagePane.tsx:816-865`).
- Stick to bottom: at the end it stays pinned as content grows; "at latest" = distance to end ≤ 16px (`useMessageScroll.ts:110-138`). Older page loads when the first rendered index ≤ 4 and the reader has scrolled up (`useMessageScroll.ts:347-356`).
- Keys (not while typing in a field): PageUp/PageDown scroll by a page, Home to the oldest loaded row, End to the newest; ArrowUp, PageUp and Home release the stick to the bottom, End sets it again (`useMessageScroll.ts:397-413`).
- Top loading row `.lr-list-loading` — always present (when there are items or older pages) so the list never shifts: min-height 32, padding 8 16, 12px / 16px line height, muted, centred; text "Ältere Nachrichten…" while loading, else empty (`MessagePane.tsx:802-808`, `chat.css:168-175`).
- Load error block `.lr-list-state` (padding 12 16, centred, 13px, colour text): "Nachrichten konnten nicht geladen werden." or "Ältere Nachrichten konnten nicht geladen werden." + button "Erneut laden" `.lr-button-secondary` (margin-top 8, padding 5 12, border 1px `--lr-border`, radius 7, bg `--lr-panel`, 13px; hover bg `--lr-hover`; disabled opacity .5) (`MessagePane.tsx:785-801`, `chat.css:119-129,176-189`).
- First load skeleton: `SkeletonRows count=6 avatars`, container padding 8 20 16; rows column gap 16; each row: flex, align start, **gap 12** between the 32px circle and a column (flex 1, gap 8) of two 10px-high bars (row height 32; block = 6*32 + 5*16 = 272 + padding 24 = 296, sitting at the bottom of the list area), widths 22% and LINE_WIDTHS[(i+1)%6] with LINE_WIDTHS = 38%, 86%, 62%, 74%, 48%, 92%; bar radius 5, fill `--lr-surface` with a shimmer gradient of alpha text 6% moving over 1.4s linear; whole region fades in after 150ms over 180ms (`components/ui/Skeleton.tsx:27-50`, `ui.css:538-577`, `chat.css:138-140`). When loaded, the content fades in (opacity 0→1, 120ms ease-out, `ui.css:621-628`).
- Jump-to-original highlight: row bg alpha accent 14% for 2000ms, fading out over 0.6s ease-out (`useMessageScroll.ts:330-344`, `chat.css:384-387`); scrolls the row to the centre.

### 4.1 History start (above the oldest message)

`MessagePane.tsx:892-921`, `chat.css:142-167`.
- Container: flex column, align start, gap 4, padding 24 20 8.
- Server channel: `h2` flex gap 6, font 600 20px / 1.25, colour text: `HashIcon` 20px muted + channel name. `p` "Anfang des Kanals." font 13px / 1.35 weight 400, muted.
- DM: Avatar size `lg` = 64px circle, initials 20px/600 (`index.css:1293-1296`), margin-bottom 8 (+ the 4 gap); `h2` peer name (no icon); `p` "Anfang der Unterhaltung."
- Height (server): 24 + 25 + 4 + 17.55 + 8 ≈ 78.5.

### 4.2 Empty channel

`MessagePane.tsx:871-876`, `chat.css:119-136`. When loaded with zero messages: `.lr-list-empty` (padding-bottom 16, 13px muted) containing History start, then `p` "Noch keine Nachrichten. Schreib die erste." left-aligned, padding 0 20, 13px muted (line height 19.5). Sits at the bottom of the list area (above typing bar).

---

## 5. Dividers

`MessagePane.tsx:833-846`, `chat.css:1941-1977`, `components/messageTime.ts:5-33`.

| | Day divider `.lr-day-divider` | Unread divider `.lr-new-divider` |
|---|---|---|
| When | first item, or local calendar day (Y-M-D in local TZ) differs from previous message | before the first unread row (`messages/pages.ts:244-270`) |
| Layout | flex row, align center, gap 12 | same |
| Padding | 14 20 6 | 6 20 2 |
| Text | 12px / 600, muted, line h 18 | 11px / 700, UPPERCASE, letter-spacing 0.4px, `--lr-accent-text`, line h 16.5 |
| Lines | 1px `--lr-border` on BOTH sides, each flex 1 | ONE line on the LEFT only (flex 1), alpha accent 60%; the label "NEU" sits at the right edge |
| Total height | 38 | 24.5 |
| Label | `dayLabel()` | "Neu" (rendered uppercase "NEU"); accessible label "Neue Nachrichten" |

`dayLabel` (de-DE): today → "Heute", yesterday → "Gestern", same year → `{weekday long}, {day numeric}. {month long}` e.g. "Mittwoch, 21. Oktober"; other year adds year: "Dienstag, 4. März 2025". Redraws at local midnight (`useLocalDay`).
A row after either divider is never "continued" (`MessagePane.tsx:822-823`).

---

## 6. Unread bar (opened with unread messages)

`MessagePane.tsx:443-481`, `chat.css:239-296`. Shown only if the list opened at the first unread row and no search/context overlay is open.
- Absolute at the top of the list area (overlays list, z 6), full width; flex row, align center, gap 8 (`--lr-space-2`), min-height 36, padding 2 12 2 20, border-bottom 1px `#725f3d` / `#e0c598`, bg `--lr-selected` (`#443b2a`/`#f2e3c9`), font 13px / 1.35 / weight 400, colour text. Rendered height **37** (2 + 32 button + 2 + 1 border exceeds the 36 min-height). Region label "Ungelesene Nachrichten".
- Text `p` (flex 1, single line, ellipsis): bold (600) "1 neue Nachricht" | "{n} neue Nachrichten" (n>99 → "99+") + normal " seit {formatWhen(first unread)}".
- Button 1: `Button size=sm variant=secondary` "Springen": min-height 32, padding-inline 12, gap 8, border 1px `--lr-border-strong`, radius 5, bg `--lr-surface`, font 500 13px/1.35, text colour; hover bg `--lr-hover`; press scale 0.97 (`ui.css:12-64`).
- Button 2: `variant=ghost` "Als gelesen markieren" + `kbd` "Esc" (margin-left 6, padding 0 4, border 1px `--lr-border-strong`, radius 3, 400 11px/1.4, muted). Ghost: transparent, hover `--lr-hover`.
- Esc anywhere (no dialog open, not in a field that consumed it) marks the conversation read; Shift+Esc the whole server (`MessagePane.tsx:201-229`).

---

## 7. "Jump to latest" / new messages pill

`MessagePane.tsx:595-608`, `chat.css:191-236`.
- Shown when not at the end and the list has items (no overlay).
- Absolute, horizontally centred in the list area, bottom 10, z 5; inline-flex, align center, gap 7, height 32, padding 0 14, border 1px `--lr-border`, radius 16, bg `--lr-surface`, colour text, 13px / 600, nowrap, shadow `0 6px 20px rgba(0,0,0,.24)`. Icon `ArrowDownIcon` 15px.
- Labels: none new → "Zu den neuesten Nachrichten"; 1 → "1 neue Nachricht"; n → "{n} neue Nachrichten" (counts messages by others that arrived since leaving the end; `messages/pages.ts arrivedSince`).
- Hover: bg `--lr-hover`. With news (`.has-news`): border transparent, bg `--lr-accent`, colour `--lr-accent-ink` (#111111); hover brightness 1.08.
- Enter: fades from opacity 0 and translateY +6px over 180ms enter ease.
- Click scrolls to the end.

---

## 8. MessageRow

`components/MessageRow.tsx:336-537`, `chat.css:298-400,418-510,2072-2079`.

### 8.1 Grouping (`messages/pages.ts:272-286`)

A message is **continued** (compact, no avatar/name) iff all hold: there is a previous message; message has no `reply`; neither message nor previous is a `system` message; same `author.id`; `0 ≤ created_at - prev.created_at < 5 min`; AND no day divider and not the first-unread row before it (`MessagePane.tsx:822-823`). A message being edited always renders as a group head (`MessageRow.tsx:380`).

### 8.2 Group head row geometry

Row = flex row, `align-items: flex-start`, gap 12 (`gap-3`), `position: relative`, padding **6 20 4** (`chat.css:299-302`), full pane width.

| Element | Spec |
|---|---|
| Avatar | 36x36 circle (overrides the `sm` 32px), margin-top 2. Image (cover, circular) if `avatar_url` loads; else initials 12px / 600, colour `--lr-text`, bg = person tint (§1.2). `chat.css:311-315`, `index.css:1234-1253,1302-1307` |
| Content column | flex 1, min-width 0 |
| (optional) Reply header | §9 |
| Author line | flex, wrap, `align-items: baseline`, gap 8: author name + time |
| Author name | 14px / 650, colour `--lr-text`, line h 21. Not truncated (wraps). |
| Time | 11px / 400, `--lr-muted`, `formatWhen()`; tooltip `formatFull()` |
| Body `.lr-message-text` | margin-top 1, 14.5px / 1.5 (21.75 line), weight 400, colour message-text mix (§1.3), `white-space: pre-wrap`, `overflow-wrap: anywhere`. Rendered markdown (§11) + edited marker |
| Edited marker | span with text "(bearbeitet)" (NO leading space; the 4px gap is `margin-left: 4`), inline right after the last content glyph, 11px, muted; tooltip "Bearbeitet am {formatFull(edited_at)}" (`MessageRow.tsx:328-335`, `chat.css:396-400`). Shown in group heads and continued rows alike. |
| Then | GIF (§14) → attachments (§13) → link previews (§15) → reactions (§16) → send failure (§18) |

Typical heights: single-line head = 6 + 21 + 1 + 21.75 + 4 = **53.75**; with reply header +22 (20 + 2 margin) = 75.75. Text x-offset = 20 + 36 + 12 = 68 from the pane's left edge. Avatar top = row top + 8.
If content is empty and there is no GIF, an empty `.lr-message-text` is still rendered (attachment-only message keeps a 1px margin, 0-height text) (`MessageRow.tsx:504-512`).

### 8.3 Continued row

`MessageRow.tsx:380-429`, `chat.css:303-306,316-332`.
- Padding **1 20 1**. Left column = `.lr-message-gutter`: width 36, padding-top 4, right-aligned, 10.5px, line height 18, muted; content `formatClock()` ("14:02"); `visibility: hidden` until the row is hovered or has focus. Hidden entirely (empty) while sending.
- Content column: body (margin-top 1) + same attachments/previews/reactions; send status shown as its own line under the body (`.lr-message-status-line`, margin-top 2).
- Single-line height = 1 + 1 + 21.75 + 1 = **23.75**.

### 8.4 Row states

| State | Visual | Ref |
|---|---|---|
| Hover or focus-within | bg row-hover (`#1f2121` / `#f1eee9`); gutter time visible; action bar visible | `chat.css:307-310,328-332,783-787` |
| Mentions me (not mine; `mentions` contains my id or `mention_everyone`) | bg mentioned (`#28261f`/`#f0eae1`), left inset bar 2px `--lr-warning` (full row height, at x 0); hover bg `#302b22`/`#ede6da` | `chat.css:2072-2079`, `MessageRow.tsx:91` |
| Jump highlight | alpha accent 14% bg, 2s | §4 |
| Sending (`tmp:` id) | body, attachments and GIF at opacity 0.7; time replaced by status "Wird gesendet…" (§18) | `chat.css:418-423` |
| Failed / uncertain | bg `#211b1c`/`#f2e6e1`, inset left bar 3px `--lr-danger-bg`; hover `#251c1c`/`#f0dfda`; upload badges hidden | `chat.css:444-451,502-504` |
| Live arrival | row animates from opacity 0, translateY +3px over 180ms enter ease | `chat.css:426-438` |
| Own message confirmed by server | body/attachments/GIF animate opacity 0.7 → 1 over 180ms | `chat.css:429-443` |

### 8.5 Timestamps (`components/messageTime.ts`, locale de-DE, local time zone)

| Function | Format | Example |
|---|---|---|
| `formatWhen` (next to name, unread bar) | today: `HH:mm`; other day: `dd.MM., HH:mm` | "14:02", "21.10., 14:02" |
| `formatClock` (continued gutter) | `HH:mm` (2-digit) | "09:05" |
| `formatFull` (tooltips) | dateStyle full + timeStyle short | "Mittwoch, 21. Oktober 2026 um 14:02" |
| `dayLabel` | §5 | "Heute", "Gestern", "Mittwoch, 21. Oktober", "Dienstag, 4. März 2025" |
| Search hit time | `toLocaleString("de-DE")` | "21.10.2026, 14:02:00" |

"today" in `formatWhen` = same local Y/M/D as now. Invalid dates render "".

---

## 9. Reply header (inside the group head, above the author line)

`MessageRow.tsx:600-632`, `chat.css:342-383`.
- Button (or `p` if deleted): flex, align center, gap 6, max-width 100%, margin-bottom 2, no padding/border/bg, colour muted, 12.5px, line height 20, left-aligned.
- Children: `ReplyIcon` 13px; author name (600, colour reply-author mix, no shrink); excerpt (single line, ellipsis, shrinks). If the excerpt exists and the original had an attachment or GIF, a `PaperclipIcon` 12px follows inline (margin-left 4, vertical-align -1px).
- Excerpt = system sentence if original is a system message; else `reply.excerpt` with `<@id>` tokens shown as `@Name`; else "GIF" or "Anhang".
- Hover/focus: excerpt colour → `--lr-text`. Click jumps to the original (scroll + highlight, or opens the context view).
- Deleted original: italic, default cursor: icon + "Die ursprüngliche Nachricht wurde gelöscht."
- Avatar aligns with the reply header line (row `align-items: flex-start`).

---

## 10. Hover action bar and actions menu

### 10.1 Bar (`MessageRow.tsx:287-312`, `ReactionBar.tsx:123-143`, `chat.css:764-806`)

- Absolute: top **-14** (continued rows **-16**), right **16**, z 2. Flex row, gap 2, padding 2, border 1px `--lr-border`, radius 7, bg `--lr-panel`, shadow `0 4px 14px rgba(0,0,0,.16)`.
- Opacity 0 + no hit-testing until row hover/focus-within; fades 120ms.
- Buttons `.lr-message-action`: 30x28, radius 5, centred icon, colour muted; hover or menu-open: bg `--lr-hover`, colour text. `aria-disabled` → opacity 0.5.
- Order (each only if allowed): 1) "Reaktion hinzufügen" `SmileIcon` **16** (if can send) 2) "Antworten" `ReplyIcon` 14 3) "Nachricht bearbeiten" `PencilIcon` 14 (own, not GIF/system) 4) "Weitere Aktionen" `MoreIcon` 14 (always). Tooltips = labels.
- Size: 4 buttons → 132x34; 3 → 100x34; 2 → 68x34. Overlaps the previous row by 14 (16) px.
- Bar exists only when the row is not pending/editing and something is possible.

### 10.2 Menu (Popover anchored to "Weitere Aktionen", also right-click on the row)

`components/MessageActionSheet.tsx`, `components/ui/Popover.tsx`, `ui/popoverPosition.ts`, `ui.css:636-661`, `chat.css:831-908`.
- Placement bottom-end: top = anchor.bottom + 6; right edges aligned; flips above (top = anchor.top - 6 - height) if it does not fit below within 8px of the window bottom and does fit above; clamped 8px inside the window (`ui/popoverPosition.ts:9-58`). transform-origin = point on the edge facing the anchor, under the anchor's horizontal centre.
- Anchor: the "Weitere Aktionen" button, OR — for a right-click on the row — the **whole row element** (not the cursor): the menu then opens under the row's bottom edge, right-aligned with the row's right edge (= pane right edge), flipping above the row if needed (`MessageRow.tsx:257-283,313-321`). Right-click on a link, image, video, audio, input/textarea, or with a text selection keeps the system menu.
- Surface: fixed, min-width 180, max-width min(360, vw-16), padding 8, border 1px `--lr-border`, radius 10, bg `--lr-elevated`, shadow-1. Opens with opacity 0→1 and scale 0.96→1 (180ms), origin at anchor.
- Content `.lr-message-menu` column, min-width 220:
  - Quick reactions row (if can react): flex wrap, gap 4, padding 2 2 8, margin-bottom 4, border-bottom 1px `--lr-border`. Buttons 36x36 circles, bg `--lr-surface`, muted, emoji 18px: 👍 ❤️ 😂 😮 😢 🎉, then "+" `PlusIcon` 18 ("Andere Reaktion auswählen"). Hover bg `--lr-hover` + colour text. Already given: bg `--lr-selected` + inset 1px ring `--lr-accent`; label "Reaktion {e} entfernen" (else "Mit {e} reagieren"). Row width 7*36 + 6*4 + 4 = 280 → menu outer width 298 (280 + 2*8 padding + 2*1 border). Without the reactions row the content is min-width 220 → outer 238.
  - Menu height with all parts: 8 + (2+36+8+1) + 4 + 3*32 + (4+1+4) + 32 + 8 + 2 = 206.
  - Items `.lr-message-menu-item`: flex, gap 10, min-height 32, padding 0 10, radius 5, 500 13px/1.35, colour text, icon 18 muted. Hover bg `--lr-hover`. Labels: "Antworten" (ReplyIcon), "Bearbeiten" (PencilIcon), "Text kopieren" (CopyIcon).
  - Divider (margin 4 0, 1px top `--lr-border`), then "Löschen" `TrashIcon` 18 in `--lr-danger` (tooltip "Löschen (Umschalt: ohne Nachfrage)").
- Copy toast: "Text kopiert." / error "Der Text konnte nicht kopiert werden."
- Delete confirm dialog: title "Nachricht löschen?" (own) or "Nachricht von {name} löschen?", message "Das lässt sich nicht rückgängig machen.", confirm "Löschen" (`MessagePane.tsx:881-890`).
- Below 640px or touch: bottom sheet titled "Nachricht von {author}" (not relevant on desktop).

---

## 11. Message markdown

Parser `messages/markdown.ts` (port 1:1), renderer `messages/MessageContent.tsx`, styles `chat.css:512-599`.

### 11.1 Parser rules

- Blocks (`markdown.ts:57-132`): fenced code ```` ```lang\n…``` ```` (lang `[A-Za-z0-9_+#.-]{1,32}` followed by newline, lower-cased; empty/whitespace body → not a code block); a newline right before the opening fence and right after the closing fence is dropped. Lines starting `"> "` are quote lines (consecutive ones join); a line starting `">>> "` quotes the rest of the message. Everything else: paragraph(s) — consecutive plain lines join with "\n".
- Inline (`markdown.ts:135-229`): `\x` escapes (x not alphanumeric/space); `` `code` `` (n-backtick runs, one surrounding space stripped when length>2); `<@uuid>` mention; `<http(s)://…>` / `<mailto:…>` link without preview; `@everyone` / `@here` (not after a letter/digit/`_@/-`, not followed by letter/digit/`_@-`); bare `http(s)://` URLs (not after an alphanumeric; trailing `.,:;!?'"*_~|` and unbalanced `)`/`]` trimmed); marks, longest first: `**` bold, `__` underline, `~~` strike, `||` spoiler, `*` / `_` italic (with the open/close rules in `opens`/`findCloser`, `markdown.ts:231-294`); nesting depth max 8. No headings, lists, tables, images.
- Only http/https (with host) and mailto (with path) become links (`markdown.ts:364-375`).

### 11.2 Rendering (all inside `.lr-message-text`: 14.5px/1.5, pre-wrap)

| Node | Rendering |
|---|---|
| Paragraph | inline run (newlines are hard breaks) |
| Bold `**` | weight 700 (`font-weight: bolder` from 400) |
| Italic `*`/`_` | Inter Italic |
| Underline `__` | underline (text colour) |
| Strike `~~` | line-through |
| Inline code | mono, **0.88em = 12.76px**, padding 1 4, radius 5, bg `--lr-panel`, border 1px `--lr-border` (`chat.css:526-536`) |
| Link | colour `--lr-accent-text`, underline in alpha accent 45%, underline offset 2px; hover underline = currentColor. Opens in the system browser. Text = the URL as written. |
| Quote | block, margin 2 0, padding-left 10, left border 3px alpha text 25% (`chat.css:537-542`) |
| Code block | block, max-width 100%, margin 4 0, border 1px `--lr-border`, radius 5, bg `--lr-panel` (`chat.css:543-557`). Top bar: flex space-between, gap 8, padding 2 6 0 10, 11px muted: left = lang (or empty), right = button (inline-flex gap 4, padding 2 6, radius 5, muted; hover bg `--lr-hover` + text) with `CopyIcon` 13 + "Kopieren"; after copy: `CheckIcon` 13 in `--lr-online-text` + "Kopiert" for 1500ms; on failure "Kopieren nicht möglich" for 1500ms (`MessageContent.tsx:167-217`, `ui.css:146-172`). Body `pre`: padding 2 10 8, glyphs mono 12.76px, `white-space: pre`, horizontal scroll. **Line pitch ≈ 21.03px, not 18.5**: preflight gives `pre` itself `font-size: 1em` (14.5px mono) and `line-height: 1.45` is set on `pre`, so its strut is 14.5×1.45 = 21.03 and dominates the 12.76px `code` boxes (`chat.css:526-530,551-557`). Bar height ≈ 2 + 20.5 (button: 2+16.5+2) = 22.5; block height ≈ 2 border + 22.5 + 2 + n×21.03 + 8. |
| Spoiler | inline, radius 5, bg mix(text 22%, bg), all text inside transparent (links not clickable). Click/Enter/Space reveals: bg alpha text 8%, normal colour. a11y label "Spoiler anzeigen" (`MessageContent.tsx:126-165`) |
| Mention / @everyone / @here | §12 |
| Emoji | ordinary text at 14.5px in the colour emoji font (see §20 gaps — there is no jumbo emoji) |

---

## 12. Mentions

`components/mentionRenderer.tsx:27-67`, `chat.css:2060-2103`, `messages/mentions.ts:192-247`.
- `<@id>` renders `@{member name}` ("@Unbekannt" if not a member) as chip `.lr-mention`: radius 4, padding 0 2, bg alpha accent 18%, colour `--lr-accent-text`, weight 600 (same 14.5px).
- Own mention (token is me and the server kept it, or `@everyone`/reached `@here` and I was pinged): `.is-me` bg alpha warning 26%, colour `--lr-text`.
- `@everyone` renders as chip only if `mention_everyone`; `@here` only if it reached someone beyond named users; otherwise plain text "@everyone"/"@here". `@here` tooltip "Alle, die gerade online waren".

---

## 13. Attachments

`components/Attachments.tsx`, `components/imageBox.ts`, `chat.css:964-1001,1157-1358`.
Container `.lr-attachments`: column, align start, gap 6, margin-top 6. Images first (one block), then other files in order.

### 13.1 Single image

| Case | Box |
|---|---|
| Size unknown (before load) | height **240**, max-width min(100%, 360); image top-left aligned inside, `max-height:100%; max-width:100%`, contain, border 1px `--lr-border`, radius 7 |
| Size known | `imageFit`: scale = min(1, 358/w, 238/h); drawn size = (w*s, h*s) **plus** 1px border all round (content-box) → box ≤ 360x240, min 24x24; never upscaled; if column narrower, scale down keeping aspect |
| Cursor | zoom-in; click opens lightbox |

Sizes are cached per attachment id (2000 entries) so re-mounted rows start at the final height (`imageBox.ts:20-50`).
Example: 1920x1080 → s = min(1, 0.1865, 0.2204) = 0.18646 → 358 x 201.4 (+2 border = 360 x 203.4).

### 13.2 Several images (grid)

`.lr-attachment-grid`: 2 equal columns, gap 4, width min(100%, 400) → cells 198 x **160**; image fills cell (cover), border 1px, radius 7 (`chat.css:1196-1210`). 3 images → 2 rows (last cell empty). Row height for n images = ceil(n/2)*160 + (rows-1)*4.

### 13.3 Failed image

`Attachments.tsx:178-205`, `chat.css:1218-1269`. Box width 240 (in grid: cell width), height 240 (grid 160). Inner: column, start-aligned, vertically centred, gap 6, padding 12, dashed 1px `--lr-border`, radius 7, bg `--lr-panel`, 12px muted: `PaperclipIcon` 15 (accent-text) / filename (13px, text, single line ellipsis) / "Bild konnte nicht geladen werden." / actions row gap 12: "Erneut laden" and "Herunterladen" (accent-text, underlined, no bg).

### 13.4 Upload state badge

"Wird hochgeladen…" / "Wartet" — 11px muted. On images: absolute left 6 bottom 6, padding 2 6, radius 5, bg alpha panel 85% (`chat.css:1214-1217,1270-1277`).

### 13.5 Generic file `.lr-attachment-file`

inline-flex, max-width 100%, align center, gap 8, padding 7 12 7 10, border 1px `--lr-border`, radius 7, bg `--lr-panel`, 13px text; `PaperclipIcon` 15 accent-text; filename 600 single-line ellipsis; size 11px muted, no shrink. Hover bg `--lr-hover`. Height 35.5. Clicking downloads; then icon → `CheckIcon` 15 and size text → "Download gestartet" (`components/download.ts:12`).
Size format (`messages/rules.ts:263-267`): ≥1 MiB → "{x,y} MB" (one decimal, comma), else "{max(1, round(kB))} KB".

### 13.6 Video / audio

`chat.css:1278-1310`. Figure column gap 4, max-width min(100%, 420). Video: width 100%, max-height 240, border 1px, radius 7, bg #000, native controls. Audio: width min(100%, 360), native controls. Caption: flex gap 8, 12px muted: filename link (600, text colour, ellipsis) + size. Unplayable types fall back to the file chip.

### 13.7 Text file preview (≤100 KiB text types)

`<details>` `.lr-attachment-text`: width min(100%, 560), border 1px, radius 7, bg panel, 13px. Summary row: flex gap 8, padding 7 12 7 10, Paperclip 15 accent-text, filename 600 ellipsis, size 11px muted. Open: "Vorschau wird geladen…" → `pre` (max-height 320, padding 10 12, border-top 1px, mono 12.5px / 1.45, scroll) + "Herunterladen" link (padding 6 12 8, 12px muted); error "Vorschau nicht verfügbar. Herunterladen".

### 13.8 Lightbox

`components/ImageLightbox.tsx:53-95`, `chat.css:1081-1156`. Full-window, backdrop rgba(0,0,0,.85), padding 56 16 16, image centred contain, radius 7. Top bar fixed: padding 10 16, gap 16, 14px white: filename (ellipsis, flex 1), "Herunterladen" link (underlined), close 40x40 round bg rgba(255,255,255,.12) `CloseIcon` 18. Fade in 240ms, image scale 0.97→1.

---

## 14. GIF message

`components/GifPicker.tsx:61-100`, `gifs/api.ts:40-54`, `chat.css:1002-1043`.
- Box = `gifBox`: scale = min(1, 320/w, 240/h), rounded, min 1 → width x height (border-box incl. 1px border). Margin-top 4, border 1px `--lr-border`, radius 7, bg `--lr-panel`, clip.
- Plays automatically unless reduced motion (then still frame, animates on hover). Click toggles play/pause.
- Paused: badge "GIF" bottom-left 6/6, padding 1 6, radius 5, bg rgba(0,0,0,.65), white 11px/700, letter-spacing 0.04em.
- No image URL: placeholder centred "GIF" 13px/700, letter-spacing 0.04em, muted (`chat.css:1036-1043`).
- Keyboard focus: outline 2px `--lr-focus`, offset 2 (`chat.css:1018-1022`). Accessible label "GIF: {title}, Animation" (or "GIF, Animation"); tooltip = title.
- A GIF message has no body text block when content is empty.

---

## 15. Link previews

`components/LinkPreviews.tsx`, `chat.css:601-762`.
- Container column gap 6, margin-top 6. Up to 3 cards (server decides).
- Card `.lr-preview`: grid `[minmax(0,1fr) auto]`, gap 10, max-width 432, padding 10 12, border 1px `--lr-border` with left border **3px** `#b78f48`/`#e4b464`, radius 5, bg `--lr-panel`. The card is a stretched item of a flex column, so its width is **always min(432, text column)** = 432 at both viewports — not shrink-to-fit. Height = 20 + 2 + max(text column, 80 thumbnail).
- Text line heights: site 12px → 18; title 14px → 21 per line; description 13px × 1.4 = 18.2 per line (max 4 lines = 72.8).
- Text column: flex column gap 2: site name (`site_name` or host without `www.`) 12px muted; title (if any) 14px / 600, `--lr-accent-text`, wraps anywhere, hover underline, opens URL; description 13px / 1.4, colour text85 mix, clamp 4 lines.
- Image (small, width < 400): 80x80 button, radius 5, cover, zoom-in, opens lightbox ("Bild vergrößern").
- Large (image width ≥ 400 or YouTube): single column; image full width (404px max inner), height auto, img max-height 160 (cover).
- YouTube: 16:9 full width, radius 5, bg #000, thumbnail cover; centred play badge 56x40 radius 10 bg rgba(0,0,0,.7) white "▶" 18px, hover bg `#cc0000`. Click replaces with embedded player (native: open in browser).
- Remove button (author/moderator): absolute top 4 right 4, 20x20 circle, muted, `CloseIcon` 12, opacity 0 until card hover; hover bg `--lr-hover` + text. Tooltip "Vorschauen entfernen".
- Inner widths: text column = 432 - 24 - 4 - 80 - 10 = 314 (with thumbnail).

---

## 16. Reactions bar

`components/ReactionBar.tsx:50-161`, `messages/reactions.ts:20-60`, `chat.css:1758-1808`.
- Container `.lr-reactions`: flex wrap, align center, gap 6, margin-top 6. With no reactions it takes no space (the add button lives in the hover bar).
- Chip `.lr-reaction`: inline-flex, align center, gap 4, min-height 26, padding 0 9, border 1px `--lr-border`, radius 13, bg `--lr-panel`, colour text, 13px. Content text "{emoji} {count}" (one space). Height 26.
- Hover: border `#575a5b`/`#b4b0a8`. Mine: border `#a38145`/`#e3b973`, bg `--lr-selected`. Disabled (cannot send & not mine)/busy: opacity 0.5.
- Newly added: pop scale 1 → 1.14 (at 40%) → 1 over 260ms.
- Tooltip: reactor names, first letter upper-cased: up to 3 names ("du" for me), "A, B und C", "A, B, C und 2 weitere".
- Click toggles mine. Emoji picker modal title "Reaktion auswählen".
- Fallback inline add chip (no hover bar slot): same chip with `SmileIcon` 15, muted.

---

## 17. System messages

`MessageRow.tsx:344-378`, `messages/system.ts:33-40`, `chat.css:401-416`.
- Same row container (padding 6 20 4, gap 12, hover bg, action bar) but no header.
- Icon column: width 36, padding-top 3, centred `LiveIcon` 14 muted.
- Text `p`: 13.5px / 1.5, muted, wraps anywhere: "{author} ist live in {voice_name}." (unknown kind: "Systemnachricht von {author}"), then a space and the time (`formatWhen`, 11px muted).
- Reactions below. Height single line: 6 + 20.25 + 4 = 30.25.
- Never grouped with neighbours.

---

## 18. Own-send status and failure

`MessageRow.tsx:540-598`, `chat.css:452-510`.
- Sending: in place of the time: inline-flex gap 5, font 400 12px/1.4 muted: spinner (10x10 circle, 2px border currentColor with transparent right side, rotating 0.8s linear) + "Wird gesendet…". Appears only after 400ms.
- Failed: `WarningIcon` 13 + "Nicht gesendet" (or "Status unklar – Verlauf prüfen"), colour `--lr-danger`, 600.
- Failure block (margin-top 6, 500→400 13px/1.35 muted): error text (+ " Die Nachricht kann bereits gespeichert sein. Prüfe den Verlauf, bevor du sie erneut sendest." when uncertain); buttons row (wrap, gap 6, margin-top 6): `sm secondary` "Erneut senden" (failed only), `sm ghost` "Bearbeiten" (not GIF), `sm ghost` "Verwerfen".
- Above the composer field, `.lr-send-summary` (alert): flex gap 8, margin-bottom 8, padding 6 10, radius 5, bg send-summary mix, inset left bar 3px `--lr-danger-bg`, font 500 13px/1.35 text: `WarningIcon` 14 (danger) + "1 Nachricht nicht gesendet" / "{n} Nachrichten nicht gesendet" + button "Anzeigen" (600, underlined offset 2) (`Composer.tsx:341-353`, `chat.css:1623-1648`).

---

## 19. Edit mode

`MessageRow.tsx:454-503`, `chat.css:910-962`.
- Replaces the body in the group head: form column gap 6, margin-top 4.
- Textarea: width 100%, `field-sizing: content` (NOT only under `pointer: coarse` — this one sizes on desktop too, `chat.css:921-937`), min-height 3.5em = **50.75px**, max-height 50% of the window height (**400 @800**, **450 @900**), then scrolls; no resize, padding 8 10, border 1px `--lr-accent`, radius 7, bg `--lr-panel`, text colour, 14.5px/1.5 (21.75 per line). Height = max(50.75, lines*21.75 + 16 + 2): 1 line → 50.75, 2 lines → 61.5, 3 lines → 83.25. (`rows={2}` only matters in engines without field-sizing, e.g. Firefox/WebKitGTK: fixed 61.5. Match Chromium.) Content shown with `@Name` instead of `<@id>`.
- Mention suggestions list above it (§21.6).
- Error line `.lr-field-error` 12px danger.
- Actions row: flex wrap, align center, gap 4 6, 12px muted: "Esc bricht ab · Enter speichert" (margin-right auto), "Abbrechen" (padding 4 10, radius 5, 600, text colour, hover `--lr-hover`), "Speichern" (accent bg, accent-ink; disabled opacity .5).
- Enter saves, Shift+Enter newline, Esc cancels.

---

## 20. Emoji sizing — and a gap

There is no jumbo/emoji-only enlargement anywhere in `web/src` (searched for jumbo/emoji-only; none). Emoji inside messages render at the surrounding font size (14.5px body, 13px in reaction chips, 18px in menu quick reactions, 24px in the emoji picker grid). Do NOT add jumbo emoji, or screenshots will diverge.

---

## 21. Composer

`components/Composer.tsx:326-553`, `chat.css:1360-1648,2080-2103`.

### 21.1 Structure (top to bottom inside `form.lr-composer`, padding **0 16 14**)

1. Send summary (§18) if unsent attempts.
2. Field `.lr-composer-field`: border 1px `--lr-border-strong`, radius **11** (7+4), bg `--lr-panel`; focus-within border `#b78f48`/`#e4b464` (120ms). File dragged over pane: border `--lr-accent`, pane outline 2px dashed alpha accent 70% inset 6px.
   1. Reply bar (§21.4)
   2. Drop hint "Loslassen zum Anhängen" (padding 10 14 0, 13px/600, accent-text) while dragging files
   3. Pending attachments (§21.5)
   4. Mention suggestions (§21.6)
   5. Row `.lr-composer-row`
3. Status line (§21.7)
4. File error `p` (padding 4 6 0, 12px danger)

### 21.2 Row

Flex, `align-items: flex-end`, gap **2**, padding **5**. Children:

| # | Element | Spec |
|---|---|---|
| 1 | Attach (only if can send files) | `.lr-composer-icon` 34x34, radius 7, muted; hover bg `--lr-hover` + text. `PaperclipIcon` 18. Tooltip/label "Datei anhängen" |
| 2 | Textarea | flex 1, min-width 0, min-height 34, max-height 160, rows 1, padding 6 6, no border/outline, transparent, text colour, 14.5px, line height **22**. Placeholder `Nachricht an #{channel}` (server) / `Nachricht an @{peer name}` (DM) in placeholder colour `#74797c`/`#868889` |
| 3 | Emoji | 34x34 icon button, `SmileIcon` 18, "Emoji einfügen" |
| 4 | GIF (only if server GIFs enabled) | 34x34 icon button with text "GIF" 11px / 800 / letter-spacing 0.04em, "GIF senden" |
| 5 | Send | 34x34, radius 7, bg `--lr-accent`, colour `--lr-accent-ink`, `SendIcon` 17, "Senden". Hover brightness 1.08; press scale 0.97. Disabled (empty draft and no files, over limit, cannot send): bg transparent, colour muted, opacity 0.55 |

Desktop height: the textarea is ONE line (34px) and scrolls internally for longer drafts — auto-grow (`field-sizing: content`, up to 160) applies only under `pointer: coarse` (`chat.css:1447-1459`, `composerHeight.ts`). So idle field = 1 + 5 + 34 + 5 + 1 = **46**; composer = **60**.
Horizontal: field inner width = pane - 32 - 2; textarea width = inner - 10 - 2*gaps - buttons (e.g. server 744: 710 - 10 - 4*34 - 4*2 = 556 with attach+emoji+GIF+send).
Keys: Enter sends, Shift+Enter newline, Esc cancels a reply (`Composer.tsx:309-324`). Paste/drop of files attaches.

### 21.3 Read-only

`.lr-composer-readonly` instead of the form: margin 0 16 14, padding 12 14, radius 7, bg `--lr-panel`, muted 13px: "Du kannst in diesem Kanal nicht schreiben." (height 43.5 + 14).

### 21.4 Reply bar

`Composer.tsx:359-378`, `chat.css:1521-1562`. Margin 8 10 0, padding 6 8 6 10, flex gap 8, radius 7, bg `--lr-surface`, muted 13px: `ReplyIcon` 14; `p` (flex 1, single line ellipsis) "Antwort an **{name}** · Esc bricht ab" (name 600 text colour); close button 28x28 radius 5 `CloseIcon` 14, hover `--lr-hover` + text, label "Antworten abbrechen". Adds 48px to the field.

### 21.5 Pending attachments

`Composer.tsx:384-410`, `chat.css:1509-1520,1563-1606`. List: flex wrap gap 8, margin 10 10 0. Item: inline-flex align center gap 10, max-width min(100%, 260), padding 8 12 8 8, radius 7, bg `--lr-surface`; image thumbnail 40x40 radius 5 cover (images only); text: name 13px/600 ellipsis, size 11px muted; remove 26x26 circle (margin-left 4) muted `CloseIcon` 14, hover `--lr-hover` + text, tooltip "Anhang entfernen".

### 21.6 Mention suggestions (inline list, not a floating popup)

`components/useMentionSuggestions.tsx`, `messages/mentions.ts:192-231`, `chat.css:2080-2103`.
- Trigger: caret right after `@` + up to 32 non-space, non-@ chars, at start or after whitespace/`(`.
- Candidates: members (not me) whose name starts with the query (case-insensitive), then those containing it; then `@everyone`, `@here` (server channels with permission) whose keyword starts with the query; max 8.
- Rendered inside the composer field above the row (pushes the field taller; the list area shrinks): `ul` padding 4, max-height 240 (scroll), border-bottom 1px `--lr-border`. `li`: flex, baseline, gap 8, padding 6 10, radius 5, 14px (21px line → 33px rows). Active (keyboard or hover): bg `--lr-hover`. User: "@{name}". Keyword: "@everyone" + small "Alle Mitglieder" / "@here" + "Alle, die gerade online sind" (12px muted).
- Keys: ↑/↓ cycle, Enter/Tab pick, Esc dismiss. Picking inserts "@{name} ". Sent as `<@id>`.

### 21.7 Status line

`Composer.tsx:528-545`, `chat.css:1607-1618`. Under the field: flex wrap, justify end, gap 4 12, padding 4 6 0, 12px muted; zero height when empty. Shows "{n} Zeichen übrig" when fewer than 200 of 2000 remain; "{n} Zeichen zu viel" in danger when over; plus validation error.

---

## 22. Pickers (modals opened from the composer / reactions)

Modal (`components/Modal.tsx`, `ui.css:737-812`), `wide`: width min(100%-32, 512), max-height window-32, centred, border 1px `--lr-border`, radius 14, bg `--lr-elevated`, shadow-2, backdrop `--lr-backdrop`. Header padding 20 64 12 24, title 600 16px/1.3. Close: `.ui-icon-button` absolute top 12 right 12, **36x36**, radius 5, transparent, colour muted, `CloseIcon` 18, label/tooltip "Schließen" (`components/Modal.tsx:157-162`, `ui.css:101-117`). Content width inside the body = 512 - 2 - 48 = **462** at both viewports.
Tailwind notes for both pickers: bare `rounded` = `--radius` = **4px** (Tailwind theme default; not overridden by `themeVariables`); a bare `border` class has **no colour utility, so it is `currentColor`** (= `--lr-text`, Tailwind 4 preflight `border: 0 solid`). `text-sm` = 14px/20px, `text-xs` = 12px/16px, `text-2xl` = 24px/32px, `font-medium` = 500, `p-2` = 8, `px-3 py-2` = 12/8, `gap-1` = 4, `gap-3` = 12. Body: column gap 20, padding 8 24 24, 400 14px/1.5, scrolls. Open: fade + scale 0.97→1 over 240ms.

Emoji picker (`components/EmojiPicker.tsx`, titles "Emoji einfügen" / "Reaktion auswählen"):
- Column gap 12 (inside body). Label "Emoji suchen" 14px/500; search input radius 4, border 1px (light `#dcd6cc`, dark `#a4abb1`), padding 8, autofocus. Label "Kategorie"; select same style, first option "Alle Kategorien" then the Unicode groups from `messages/emoji-data.json`.
- Status "{n} Emojis gefunden" 12px muted.
- Grid (`EmojiPicker.tsx:72-90`): max-height 40% window height (320 @800, 360 @900), scroll; columns 5 / 6 (≥380px window) / **8 (≥640px window: active at both sizes)**, gap 4 → cells (462 - 7*4)/8 = **54.25 wide x 44 high**; button min 44x44, radius 4, transparent, emoji 24px (line 32) centred; hover bg light `#f4f0e9`, dark `#272b2d`. 80 emoji = 10 rows = 10*44 + 9*4 = 476 > max-height, so the grid scrolls. Shows 80, then "Weitere Emojis anzeigen" (+80; border 1px **currentColor**, radius 4, padding 8 12, 14px/20px → height 38).
- Load failure (`ReactionBar.tsx:30-47`): "Die Emoji-Auswahl konnte nicht geladen werden." + button "Seite neu laden" (rounded 4, border currentColor, padding 8 12).
- Footer link "Unicode {version} / CLDR {cldr} – Lizenz" 12px muted underlined.
- Emoji not supported by the local emoji font are hidden. Loading text "Emoji-Auswahl wird geladen…".

GIF picker (`components/GifPicker.tsx:106-189`, title "GIF senden"): label "GIF suchen"; input placeholder "Search KLIPY" (max 100 chars, 550ms debounce); status 12px muted ("GIFs werden geladen …", "{n} angesagte GIFs", "{n} GIFs zu „{q}“", "Keine GIFs gefunden.", errors per `gifs/api.ts:63-91`); input like the emoji search (radius 4, border 1px light `#dcd6cc` / dark `#a4abb1`, padding 8); on an error with no GIFs a button "Erneut versuchen" (radius 4, border currentColor, padding 8 12, 14px); grid 3 columns (≥480px window), gap 6, max-height 50% window (400 @800, 450 @900), tiles (462-12)/3 = **150 x 112**, radius 7, bg panel, image cover, hover brightness 1.08, focus outline 2px `--lr-focus` offset 2; animates unless reduced motion (then only the hovered/focused tile); "Weitere GIFs anzeigen"/"Lädt …" (same button style); footer "Powered by KLIPY" 11px muted right-aligned. Column gap of the whole picker 12 (`GifPicker.tsx:122-189`, `chat.css:1044-1072`).

---

## 23. TypingBar

`components/TypingBar.tsx`, `ws/live.ts:20-23,145-150`, `chat.css:1810-1852`.
- Always 28px tall (reserved, no relayout), flex, align center, gap 7, padding 0 **22**, 12px muted.
- When someone types: three dots (4x4 circles, gap 3, currentColor) animating 1.2s ease-in-out infinite: opacity .35 → 1 and translateY -2px at 30%, delays 0 / 0.15s / 0.3s. Then text (single line, ellipsis).
- Text: "{A} tippt…", "{A} und {B} tippen…", "{A}, {B} und {n} weitere tippen…"; unknown member "Jemand". Entries expire 6s after the last typing event; own typing excluded.

---

## 24. Search overlay (header search)

`messages/MessageSearch.tsx:73-152`, `chat.css:1854-1929`. After 250ms of typing, the list area is replaced (same box): summary line (padding 10 20 0, 12px muted, gap 8) "Treffer in dieser Unterhaltung" + right button "Zurück zum Chat" (padding 4 8, radius 7, 12px/600 muted, hover `--lr-hover`). Results (padding 10 12 16, 14px): hit button full width, padding 10 12, radius 7, hover `--lr-hover`; line 1 (13px, baseline, gap 8): bold author + time (11px muted, `toLocaleString`); line 2 message text (margin-top 2, pre-wrap, text colour; non-interactive markdown); line 3 "Nachricht im Kontext öffnen" 12px muted (accent-text on hover). States "Suche läuft…", "Keine Nachrichten gefunden.", "Die Suche konnte nicht geladen werden." + "Erneut laden" (state paragraphs padding 6 8, muted), "Ältere Treffer"/"Lädt…" (`.lr-button-secondary`). Non-interactive code block preview: padding 6 8, max-height 8em, clipped. Hits are listed newest first. The search overlay (and the context view below) covers the list area only; the unread bar and jump pill are hidden while it is open. Hit accessible label "Zur Nachricht von {author}: {text or "Datei"}".

### 24.1 Context view (a search hit opened, or a reply whose original is not loaded)

`messages/MessageContext.tsx:52-134`, `MessagePane.tsx:551-582`, `messages/MessageSearch.tsx:63-72`. Replaces the list area (absolute inset 0, flex column). All Tailwind: bare `border` = **currentColor** (`--lr-text`), bare `rounded` = 4px.
- Top bar: flex wrap, gap 8, padding 12, border-bottom 1px currentColor (= `--lr-text`). Buttons (radius 4, border 1px currentColor, padding 8 12, inherit 16px/24px → height 42): "Zurück zu den Treffern" (only when opened from search) then "Zurück zum Chat".
- Body: flex 1, scroll, padding 12 0. Loading: "Nachrichtenkontext wird geladen…" (padding 0 16). Error (padding 0 16): "Diese Nachricht ist nicht mehr verfügbar oder du hast keinen Zugriff mehr." (API code `not_found`/`forbidden`) else "Der Nachrichtenkontext konnte nicht geladen werden."; button "Erneut laden" (margin-top 8, radius 4, border currentColor, padding 8 12).
- Messages: each rendered as a full group-head MessageRow (`continued=false` always, no dividers). The target message is wrapped: margin 0 4, radius 4, border **2px `--lr-accent`**, bg alpha accent 12% (≈ `#312c21` / `#f6ecda` on bg), scrolled to the vertical centre and focused. Label "Gefundene Nachricht von {author}". Section label "Nachrichtenkontext".

### 24.2 Keyboard focus ring (all controls)

`index.css:235-240`: any button/link/input/textarea/select/summary/[tabindex] with `:focus-visible` gets outline 2px `--lr-focus` (`#f3b752` / `#836026`), outline-offset 2, plus box-shadow `0 0 0 2px --lr-bg` (a bg-coloured gap ring). Exceptions: the composer textarea and header search input (the field border shows focus instead). Only for keyboard focus, never after a mouse click.

---

## 25. Breakpoints (for completeness; none active at 1280x800 or 1600x900)

| Query | Change | Ref |
|---|---|---|
| ≤1130px | member panel becomes drawer; header padding-right 128 | `chat.css:1705-1710,1996-2001` |
| ≤800px | row padding-inline 16; composer padding 0 12 12; textarea 16px; dividers padding-inline 16; typing bar padding-inline 18; header padding-left 16; search icon-only 34px; unread bar padding-left 16; live pill → strip | `chat.css:279-283,1711-1739,1848-1852,1972-1977,2051-2058` |
| ≤640px | link preview images 72x72 thumbnails, title clamp 2, description clamp 2; menus/modals become bottom sheets | `chat.css:676-703`, `ui/popoverPosition.ts:5` |
| ≤520px | unread bar wraps | `chat.css:284-296` |
| ≤500px height | composer/typing hidden during search | `chat.css:1740-1748` |
| pointer: coarse | 44px controls, composer auto-grow, no hover bar | `chat.css:807-829,1447-1508`, `index.css:161-169` |

---

## 26. Icons (`components/Icons.tsx`)

All: viewBox 24x24, `fill none`, `stroke currentColor`, stroke-width 2 (in viewBox units → scales with size, e.g. 1.17px at 14px), round caps and joins (`Icons.tsx:5-19`).

| Icon | Used at (size) | Path data |
|---|---|---|
| HashIcon | header 18, history start 20 | `M4 9h16M4 15h16M10 3 8 21M16 3l-2 18` |
| SearchIcon | header search 16 | circle (11,11) r7; `m20 20-4-4` |
| ReplyIcon | bar 14, reply header 13, composer reply 14, menu 18 | `M9 14 4 9l5-5`; `M4 9h10.5a5.5 5.5 0 0 1 0 11H11` |
| PencilIcon | bar 14, menu 18 | `M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z` |
| MoreIcon | bar 14 | circles r1 at (5,12), (12,12), (19,12) |
| SmileIcon | bar 16, inline chip 15, composer 18 | circle (12,12) r9; `M8.5 14.5a4.5 4.5 0 0 0 7 0M9 9.5h.01M15 9.5h.01` |
| PlusIcon | menu 18 | `M12 5v14M5 12h14` |
| CopyIcon | menu 18, code block 13 | rect x9 y9 w12 h12 rx2; `M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1` |
| TrashIcon | menu 18 | `M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6M10 11v6M14 11v6` |
| PaperclipIcon | composer 18, files 15, reply 12 | `m21.4 11.6-9.2 9.2a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.1a2 2 0 0 1-2.8-2.8l8.5-8.4` |
| SendIcon | composer 17 | `M4 12 20 4l-6 16-3-7-7-1Z`; `m11 13 9-9` |
| CloseIcon | reply cancel 14, attachment remove 14, preview remove 12, modal/lightbox 18 | `M6 6l12 12M18 6 6 18` |
| ArrowDownIcon | jump pill 15 | `M12 5v14M6 13l6 6 6-6` |
| LiveIcon | system row 14 | filled circle (12,12) r3 (no stroke); `M5 12a7 7 0 0 1 2-4.9M19 12a7 7 0 0 0-2-4.9M8.5 7.5a9 9 0 0 1 7 0M8.5 16.5a9 9 0 0 0 7 0` |
| WarningIcon | send status 13, summary 14 | `M10.3 3.9 2.4 17.6A2 2 0 0 0 4.1 20.6h15.8a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z`; `M12 9v4M12 17h.01` |
| CheckIcon | download started 15, copied 13 | `m5 12.5 4.5 4.5L19 7` |
| BellIcon | header 16 | `M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9`; `M10.3 21a1.94 1.94 0 0 0 3.4 0` (`Icons.tsx:364-371`) |
| BellOffIcon | header 16 (muted) | `M8.7 3A6 6 0 0 1 18 8a21.3 21.3 0 0 0 .6 5M17 17H3s3-2 3-9a4.67 4.67 0 0 1 .3-1.7`; `M10.3 21a1.94 1.94 0 0 0 3.4 0M2 2l20 20` (`Icons.tsx:373-380`) |
| UsersIcon | header member toggle 18 | circle (9,7.5) r3.5; `M2.5 20a6.5 6.5 0 0 1 13 0M16 4.5a3.5 3.5 0 0 1 0 6.5M18 14a6 6 0 0 1 3.5 6` (`Icons.tsx:282-288`) |
| SpeakerIcon | header title of a voice channel 18 | `M11 5 6 9H3v6h3l5 4V5Z`; `M15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13` (`Icons.tsx:29-36`) |

---

## 27. German strings (verbatim)

Pane/list: "Suchen", "In dieser Unterhaltung suchen", "Wörter oder eine Phrase in Anführungszeichen", "Strg F", "Lesestatus nicht gespeichert.", "Erneut versuchen", "Nachrichten", "Ältere Nachrichten…", "Nachrichten konnten nicht geladen werden.", "Ältere Nachrichten konnten nicht geladen werden.", "Erneut laden", "Wird geladen…", "Anfang des Kanals.", "Anfang der Unterhaltung.", "Noch keine Nachrichten. Schreib die erste.", "Heute", "Gestern", "Neu", "Neue Nachrichten", "Ungelesene Nachrichten", "1 neue Nachricht", "{n} neue Nachrichten", " seit {time}", "Springen", "Als gelesen markieren", "Esc", "Zu den neuesten Nachrichten".
Row: "(bearbeitet)", "Bearbeitet am {full}", "Antworten", "Nachricht bearbeiten", "Weitere Aktionen", "Reaktion hinzufügen", "Wird gesendet…", "Nicht gesendet", "Status unklar – Verlauf prüfen", " Die Nachricht kann bereits gespeichert sein. Prüfe den Verlauf, bevor du sie erneut sendest.", "Erneut senden", "Bearbeiten", "Verwerfen", "Die ursprüngliche Nachricht wurde gelöscht.", "GIF", "Anhang", "Esc bricht ab · Enter speichert", "Abbrechen", "Speichern", "{author} ist live in {voice}.", "Systemnachricht von {author}".
Menu: "Reagieren", "Mit {e} reagieren", "Reaktion {e} entfernen", "Andere Reaktion auswählen", "Text kopieren", "Löschen", "Löschen (Umschalt: ohne Nachfrage)", "Nachricht von {author}", "Text kopiert.", "Der Text konnte nicht kopiert werden.", "Nachricht löschen?", "Nachricht von {name} löschen?", "Das lässt sich nicht rückgängig machen.".
Content: "Kopieren", "Kopiert", "Kopieren nicht möglich", "Spoiler anzeigen", "@Unbekannt", "Alle, die gerade online waren".
Attachments/previews: "{file} vergrößern", "Bild konnte nicht geladen werden.", "Herunterladen", "Download gestartet", "Wird hochgeladen…", "Wartet", "Vorschau wird geladen…", "Vorschau nicht verfügbar.", "Vorschau: {name}", "Bild vergrößern", "Vorschauen entfernen", "Video abspielen", "▶", "Schließen".
Reactions: "Reaktionen", "{e}: {n} Reaktion(en), du hast reagiert", "du", "{a} und {b}", "und {n} weitere", "Reaktion auswählen".
Composer: "Nachricht an #{channel}", "Nachricht an @{peer}", "Datei anhängen", "Emoji einfügen", "GIF senden", "Senden", "Antwort an {name}", " · Esc bricht ab", "Antworten abbrechen", "Loslassen zum Anhängen", "Anhänge", "{file} entfernen", "Anhang entfernen", "{n} Zeichen übrig", "{n} Zeichen zu viel", "1 Nachricht nicht gesendet", "{n} Nachrichten nicht gesendet", "Anzeigen", "Du kannst in diesem Kanal nicht schreiben.", "Erwähnen", "Alle Mitglieder", "Alle, die gerade online sind", "Nicht angehängt: {names}. {reason}".
Typing: "{a} tippt…", "{a} und {b} tippen…", "{a}, {b} und {n} weitere tippen…", "Jemand".
Pickers: "Emoji suchen", "Kategorie", "Alle Kategorien", "{n} Emojis gefunden", "Emoji-Auswahl", "Weitere Emojis anzeigen", "Unicode {v} / CLDR {c} – Lizenz", "Emoji-Auswahl wird geladen…", "Die Emoji-Auswahl konnte nicht geladen werden.", "Seite neu laden", "GIF suchen", "Search KLIPY", "GIF-Auswahl", "Weitere GIFs anzeigen", "Lädt …", "Powered by KLIPY", "GIFs werden geladen …", "{n} angesagte GIFs", "{n} GIFs zu „{q}“", "Keine GIFs gefunden.".
Header: "Profil von {peer}", "Online", "Abwesend", "Offline", "Benachrichtigungen", "Stummgeschaltet", "Benachrichtigungen für {name}", " (stumm)", "Mitgliederliste", "Mitgliederliste einblenden", "Mitgliederliste ausblenden", "Live", "Du", "Jemand", "{a} und {b}", "{a}, {b} und {c}", "{a}, {b} und {n} weitere", " in {channel}", " +{n}", "{names} ist live" / "sind live" / "Du bist live" / "seid live", "Beitreten", "Zum Kanal".
Context view: "Nachrichtenkontext", "Zurück zu den Treffern", "Zurück zum Chat", "Nachrichtenkontext wird geladen…", "Diese Nachricht ist nicht mehr verfügbar oder du hast keinen Zugriff mehr.", "Der Nachrichtenkontext konnte nicht geladen werden.", "Erneut laden", "Gefundene Nachricht von {author}", "Zur Nachricht von {author}: {text}", "Datei".
Search: "Nachrichten suchen", "Treffer in dieser Unterhaltung", "Zurück zum Chat", "Suche läuft…", "Die Suche konnte nicht geladen werden.", "Keine Nachrichten gefunden.", "Nachricht im Kontext öffnen", "Ältere Treffer", "Lädt…".

---

## 28. Data the UI reads (`messages/types.ts`)

Message (`messages/types.ts:22-54`): `id` (`tmp:` prefix = pending local), `revision?` (number), `created_order?` (number), `channel_id`, `author {id, name, avatar_url: string|null}`, `content` (wire text with `<@uuid>` tokens; empty for GIF and system messages), `created_at` (ISO), `edited_at` (ISO|null), `attachments[] {id, filename, content_type, size}` (client-only `preview_url`, `upload: "waiting"|"uploading"|"done"`), `reactions?[] {emoji, user_ids[]}`, `gif? {provider (default "klipy"), slug, title, width, height, url, still|null}` (dropped unless slug/url strings and width/height > 0), `mentions?[]` (user ids), `mention_everyone?`, `reply? {id, deleted, author?{id,name}, excerpt?, has_attachment?, has_gif?, system?}` (a deleted original is only `{id, deleted:true}`), `previews?[] {url, site_name?, title?, description?, image?{url,width,height}, youtube_id?}`, `system? {kind, params}` (only kind `live_started` with `params.voice_name` is known), `nonce?` (echo of the sender's attempt uuid).
Normalisation the UI relies on: Redis cjson may send `{}` for an empty array — treat any non-array `attachments`/`reactions`/`mentions`/`previews` as empty. A preview is kept only if `url` is http(s); its `image` only if `image.url` starts with `/api/previews/` and width/height > 0; `youtube_id` only if it matches `^[A-Za-z0-9_-]{11}$` (`types.ts:143-174`). Reply excerpt ≤ 100 chars, server-made (`REPLY_EXCERPT_MAX`).
Page: `{messages (oldest → newest), has_more}`; older page cursor = `"{created_at}|{id}"` of the oldest row; page size 50 (`rules.ts:7`). Content limit 2000 chars (`messages/rules.ts:6`).

---

## 29. Gaps and implementation notes

1. No jumbo emoji in the web client (§20) — match that.
2. Font weight 650 (author name, header title) and 800 (GIF button) need a variable-font instance; egui cannot synthesize them.
3. CSS `line-height: 1.5` places glyphs centred in a 21.75px box for 14.5px text; egui's default row height differs — set explicit row heights per §8 to keep row tops aligned (cumulative drift breaks SSIM fast).
4. Text with `overflow-wrap: anywhere` + `pre-wrap`: wrap at any character when a word does not fit, keep author newlines.
5. Native `<video>`/`<audio>` controls (§13.6) and the YouTube iframe (§15) have no egui equivalent; draw a poster frame/black box of the same size and open externally.
6. Widths at 1280/1600 assume the shell columns from `index.css:146-149` (rail 64, sidebar 224, members 248); the shell spec owns those.
7. Image/preview/avatar loading states: reserve the sizes in §13 before load so rows do not jump.
8. `mix()` values in §1.3 apply only to the two default presets; for other themes compute them at runtime with the formulas given.
