# UI spec: design tokens and desktop app shell (egui 0.37 port of web/src)

Area: `ui-tokens-shell`. Source of truth: `web/src` on branch `claude/native-foundation`.
All paths below are relative to `web/src/` unless they start with `web/`.
"px" means CSS px at UI scale 1 (default, see section 1.7). Desktop target = mouse
(`pointer: fine`), so every `@media (pointer: coarse)` rule is listed only for
completeness and must NOT be applied.

Conventions used in this document

- `box-sizing: border-box` everywhere (Tailwind v4 preflight), so every width/height
  below INCLUDES its border and padding. A 1px border is drawn inside the box.
- Default `line-height` is `1.5` (Tailwind preflight on `html`); body font-size is
  `16px` unless a rule sets it. A text "line box" is `font-size * line-height`, and
  glyphs are vertically centred in it by CSS half-leading (see 2.4).
- Colours are given for the default DARK theme ("Gelabber Dunkel") and the default
  LIGHT theme ("Gelabber Hell"). Default preference with nothing stored (or an invalid
  value) is `dark` (`theme/theme.ts:11-27`: `asThemePreference`/`loadThemePreference`
  return `"dark"`); `theme/startup.ts:21-40` applies `builtinTheme(preference)` (with
  `"system"` resolved via `prefers-color-scheme`) and falls back to
  `builtinTheme("dark")` on any error (`startup.ts:40`).
- `hover` = pointer over the element; `active`/`selected` = the current route item.

---------------------------------------------------------------------------------

## 1. Design tokens

### 1.1 Theme model (how colours are produced)

`theme/model.ts:3-12` – a theme has 8 editable colours: `background, panel, surface,
rail, text, muted, accent, border`, plus `mode` (dark|light) and `style`
(clear|soft|terminal). Built-ins pass through `fixContrast` (`theme/presets.ts:227`,
`model.ts:269-289`) – for both defaults it changes nothing.

Default palettes: `theme/defaults.json:1-22`; presets `theme/presets.ts:6-21`
(id `dark` = "Gelabber Dunkel", id `light` = "Gelabber Hell", both `style: "clear"`).

Derived colours: `model.ts:187-238` (`derivedColors`). Helper `mix(a,b,t)` is a plain
per-channel sRGB lerp rounded to int (`model.ts:124-138`); `ensureContrast` steps a
colour towards #ffffff/#111111 in 1% steps until WCAG ratio met (`model.ts:139-156`).
The values below are the exact outputs (computed by running `themeVariables()` on both
defaults), so the port can hardcode them for the two default themes. For custom/other
presets the port must re-implement `derivedColors` + `themeVariables` 1:1.

### 1.2 `--lr-*` colour variables, resolved (`model.ts:295-391`)

| Variable | Source expression | Dark (default) | Light |
|---|---|---|---|
| `--lr-bg` | `background` | `#17191a` | `#f8f5f0` |
| `--lr-panel` | `panel` | `#1d2021` | `#f4f0e9` |
| `--lr-surface` | `surface` | `#272b2d` | `#ece7df` |
| `--lr-rail` | `rail` | `#131617` | `#eee9e1` |
| `--lr-elevated` | dark: mix(panel,text,.04); light: mix(bg,#fff,.6) | `#252829` | `#fcfbf9` |
| `--lr-text` | `text` | `#eef0ee` | `#252829` |
| `--lr-muted` | ensureContrast(muted,[panel,surface,hover]) | `#a4abb1` | `#5c646b` |
| `--lr-subtle` | ensureContrast(mix(muted,bg,.25),[bg,panel,surface,elevated]) | `#8c9295` | `#65696c` |
| `--lr-border` | `border` | `#2c3032` | `#dcd6cc` |
| `--lr-border-strong` | ensureContrast(mix(border,text,.35), …, 3) | `#717475` | `#86837e` |
| `--lr-accent` | `accent` (fill colour) | `#f3b752` | `#e8a638` |
| `--lr-accent-ink` | readable ink on accent | `#111111` | `#111111` |
| `--lr-accent-text` | accent as text/icon colour | `#f3b752` | `#836026` |
| `--lr-focus` | = accent-text | `#f3b752` | `#836026` |
| `--lr-hover` | mix(panel,text,.08) | `#2e3131` | `#e3e0da` |
| `--lr-selected` | mix(panel,accent,.18) | `#443b2a` | `#f2e3c9` |
| `--lr-online` | ensureContrast(#85d69b / #287a45, …, 3) | `#85d69b` | `#287a45` |
| `--lr-online-text` | ensureContrast(online,[panel,surface]) | `#85d69b` | `#277643` |
| `--lr-warning` | ensureContrast(#d8a657, …, 3) | `#d8a657` | `#a27e44` |
| `--lr-danger` | ensureContrast(#f39b98 / #a12b29) | `#f39b98` | `#a12b29` |
| `--lr-danger-bg` | fixed | `#8f302d` | `#b33b38` |
| `--lr-on-danger` | fixed | `#ffffff` | `#ffffff` |
| `--lr-backdrop` | fixed | `rgba(9,12,13,0.72)` | `rgba(37,40,41,0.45)` |
| `--lr-speaking` | `var(--lr-online)` (`index.css:95`) | `#85d69b` | `#287a45` |

Tile tints (`--lr-tile-1..8`, `model.ts:178-180, 229-236, 335-337`): hues
`[18,42,88,145,188,214,258,322]`; dark `hsl(h,.30,.30)`, light `hsl(h,.45,.85)`, then
mixed toward bg in 5% steps until text contrast ≥ 4.5 (step 0 already passes for both
defaults).

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

Tailwind neutral overrides (`model.ts:299-334`; used by any `*-neutral-*` utility):

| Shade | Dark | Light |
|---|---|---|
| 50/100/200/300 | `#eef0ee` | 50 `#f8f5f0`, 100 `#f4f0e9`, 200 `#ece7df`, 300 `#dcd6cc` |
| 400/500/600 | `#a4abb1` | `#5c646b` |
| 700 | `#2c3032` | `#252829` |
| 800 | `#272b2d` | `#252829` |
| 900 | `#1d2021` | `#252829` |
| 950 | `#17191a` | `#252829` |

Static fallbacks in `index.css:6-18` (`@theme`) are overwritten at runtime by the
values above; ignore them.

### 1.3 Colours computed by `color-mix()` in shell CSS (pre-resolved)

`color-mix(in srgb, A p%, B)` = per-channel `A*p + B*(1-p)`.

| Use (file:line) | Expression | Dark | Light |
|---|---|---|---|
| Header search placeholder (`components/chat.css:78-80`) | subtle 80% + panel | `#74797c` | `#868889` |
| Header search border on hover / with text (`chat.css:81-86`) | border 82% + text 18% | `#4f5354` | `#bbb7af` |
| Header search border focused (`chat.css:91-93`) | accent 70% + border | `#b78f48` | `#e4b464` |
| Header Live pill bg (`chat.css:2020`) | danger-bg 10% + bg | `#231b1c` | `#f1e2de` |
| Header Live pill border (`chat.css:2017-2018`) | danger-bg 45% + border | `#593030` | `#ca9089` |
| Call-bar control "off" bg on panel (`voice/room.css:1189-1191`) | danger-bg 16% over panel | `#2f2323` | `#ead3cd` |
| Call-bar Verlassen hover (`room.css:1238-1240`) | danger-bg + 12% black | `#7e2a28` | `#9e3431` |
| Speaking glow (`index.css:1270-1272`) | speaking @30% / @45% alpha | `rgba(133,214,155,.30)` / `.45` | `rgba(40,122,69,.30)` / `.45` |

### 1.4 Radii (`model.ts:290-294, 327, 374-381`; `index.css:127`)

Style `clear` (both defaults). Soft/terminal given for other presets.

| Token | clear | soft | terminal |
|---|---|---|---|
| `--lr-radius-xs` | 3px | 6px | 1px |
| `--lr-radius-sm` = `--lr-radius-small` | 5px | 8px | 2px |
| `--lr-radius-md` = `--lr-radius` | 7px | 12px | 2px |
| `--lr-radius-lg` | 10px | 16px | 2px |
| `--lr-radius-xl` | 14px | 20px | 3px |
| `--lr-radius-full` | 999px | 999px | 999px |

Tailwind `rounded-*` map: xs→xs, sm/md/lg→sm, xl→md, 2xl/3xl→xl (`model.ts:340-346`).
`--lr-font` for `terminal` style is `ui-monospace, "Cascadia Code", monospace`
(`model.ts:370-373`); both defaults use Inter.

### 1.5 Shadows (`model.ts:382-389`)

| Token | Dark | Light |
|---|---|---|
| `--lr-shadow-1` (menus, popovers, tooltips) | `0 8px 24px rgba(0,0,0,.32)` | `0 8px 24px rgba(60,45,20,.12)` |
| `--lr-shadow-2` (sticky bars, dialogs) | `0 18px 48px rgba(0,0,0,.5)` | `0 18px 48px rgba(60,45,20,.18)` |
| `--lr-shadow-card` | `none` (clear) | `none` (clear); soft style: `0 6px 22px #00000012` |
| User menu (`index.css:1221`) | `0 8px 25px rgba(0,0,0,.2)` (`#0003`) | same |

Format: `x y blur [spread] colour`. egui: draw as a blurred rect (egui `Shadow { offset:[0,8], blur:24, spread:0, color }`).

### 1.6 Type scale (`index.css:98-107`)

Shorthand is `weight size / line-height`. All use `--lr-font` (Inter).

| Token | Weight | Size | Line height | Line box |
|---|---|---|---|---|
| `--lr-type-caps` | 600 | 11px | 1.3 | 14.3px (always with `letter-spacing:.06em` = 0.66px, UPPERCASE) |
| `--lr-type-meta` | 400 | 12px | 1.4 | 16.8px |
| `--lr-type-ui` | 500 | 13px | 1.35 | 17.55px |
| `--lr-type-body` | 400 | 14px | 1.5 | 21px |
| `--lr-type-title-sm` | 600 | 16px | 1.3 | 20.8px |
| `--lr-type-title` | 600 | 20px | 1.25 | 25px |
| `--lr-type-page` | 700 | 28px | 1.15 | 32.2px |
| `--lr-type-hero` | 700 | clamp(28px, 3vw, 36px) | 1.1 | 1280w: 36px (3vw=38.4→clamped); 1600w: 36px |
| `--lr-type-input-touch` | – | 16px | – | touch only |

Other viewport-dependent sizes in `index.css` (not shell, for reference):
`.living-room-title` `clamp(32px,3.23vw,48px)` → 1280w: 41.34px, 1600w: 48px;
`.living-room-subtitle` `clamp(18px,1.85vw,27px)` → 1280w: 23.68px, 1600w: 27px
(`index.css:1560-1570`).

### 1.7 Spacing, controls, layout, motion (`index.css:108-160`)

| Token | Value |
|---|---|
| `--lr-space-0_5 / 1 / 1_5 / 2 / 3 / 4 / 5 / 6 / 8 / 10` | 2 / 4 / 6 / 8 / 12 / 16 / 20 / 24 / 32 / 40 px |
| `--lr-control-xs / sm / md` | 28 / 32 / 36 px (all 44 under coarse pointer) |
| `--lr-rail-width` | 64px |
| `--lr-sidebar-width` | 224px (256 under coarse pointer, `index.css:1976-1980`) |
| `--lr-navigation-width` | 288px (rail + sidebar) |
| `--lr-members-width` | 248px |
| `--lr-header-height` | 52px |
| `--lr-dock-height` | 56px (61 coarse) – bottom strip incl. its 1px top border |
| `--lr-media-space` | 64px default; set at runtime to the measured call-bar height (`components/VoiceSessionControls.tsx:74-89`), = 56px for a one-row bar |
| `--lr-ui-scale` | 1 (Kompakt, default) / 1.125 (Standard) / 1.25 (Groß) – `interface/preferences.ts:6-22`; applied as `html { zoom }` (`index.css:193-201`). Port: multiply egui `pixels_per_point` by it. |
| `--lr-dur-micro / fast / dur / large / exit / large-exit` | 100 / 120 / 180 / 240 / 130 / 170 ms (all 0 with reduced motion) |
| `--lr-ease` = `--lr-ease-enter` | cubic-bezier(0.2, 0.8, 0.2, 1) |
| `--lr-ease-standard` | cubic-bezier(0.2, 0, 0, 1) |
| `--lr-ease-exit` | cubic-bezier(0.4, 0, 1, 1) |
| `--lr-press-scale` | 0.97 (buttons scale while pressed) |

Hover/selected colour changes on rows/buttons transition `background-color, border-color,
color` over 100ms `ease-standard` (`index.css:247-279`).

Focus ring (keyboard only, `index.css:235-240`): 2px solid `--lr-focus` outline at
2px offset, plus a 2px `--lr-bg` box-shadow gap.

### 1.8 Breakpoints that affect the shell

| Query | Effect | 1280×800 | 1600×900 |
|---|---|---|---|
| `max-width: 1130px` | member list becomes a drawer + "Mitglieder" button (`index.css:1618-1717`, `chat.css:1996-2006`) | not active | not active |
| `min-width: 1131px` | toggled-off member panel is `display:none` (`chat.css:2002-2006`) | active | active |
| `max-width: 1023px` | call-bar hides button words (`room.css:1243-1251`) | not active | not active |
| `min-width: 801px` | call bar starts at x=288 and shares the 56px strip with the user panel (`room.css:980-996`) | active | active |
| `max-width: 800px` | navigation (rail+sidebar) becomes a left drawer, mobile top bar (`index.css:1718-1915`) | not active | not active |
| `max-width: 640px / 480px / 370px` | phone tweaks | not active | not active |
| `pointer: coarse` | 44px targets | never (desktop) | never |

So at 1280×800 and 1600×900 the layout is identical except the width of the main column.

### 1.9 Stacking order of shell overlays (paint order, low → high)

| z | Element | Source |
|---|---|---|
| 35 | server menu (`.shell-menu` under the server name), user menu (`.user-panel-menu > nav`) | `index.css:717, 1212` |
| 40 | call bar `.voice-session-dock` | `voice/room.css:938` |
| 60 | `.ui-popover` (row right-click menu, status popover) | `components/ui/ui.css:638` |
| 70 | server-rail tooltip `.server-tile-tip` (position fixed) | `index.css:577` |
| 100 | skip link "Zum Inhalt springen" (only on keyboard focus) | `index.css:305` |

---------------------------------------------------------------------------------

## 2. Fonts

### 2.1 Files and faces (`index.css:42-78`, `fonts/INTER-PROVENANCE.json`)

Family name "Gelabber Inter" = Inter 4.1 variable, served from `web/src/fonts/`:

| File | Style | Axes | Unicode range |
|---|---|---|---|
| `InterVariable-latin.woff2` | normal | wght 100–900 (default 400), opsz 14–32 | U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD |
| `InterVariable-latin-ext.woff2` | normal | same | U+0100-02BA, U+02BD-02C5, U+02C7-02CC, U+02CE-02D7, U+02DD-02FF, U+1E00-1E9F, U+1EF2-1EFF, U+2020, U+20A0-20AB, U+20AD-20C0, U+2113, U+2C60-2C7F, U+A720-A7FF |
| `InterVariable-Italic-latin.woff2` | italic | same | as latin |

Fallback stack: `system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`
(`model.ts:373`). Rendering: `font-synthesis: none; text-rendering: optimizeLegibility;
-webkit-font-smoothing: antialiased` (`index.css:88-90`). No `font-feature-settings`
anywhere; `font-variant-numeric: tabular-nums` only on counters noted below.

### 2.2 Vertical metrics (read from the woff2)

unitsPerEm 2048; hhea/typo ascender 1984 (0.96875 em), descender −494 (0.24121 em),
lineGap 0; xHeight 1118; capHeight 1490. Glyph content height = 1.20996 em.

### 2.3 Weights actually used in the shell

| Weight | Where |
|---|---|
| 400 | channel/DM row names, participant names, member status/voice lines, `--lr-type-meta`, body text |
| 500 | `--lr-type-ui` (menus, tooltip, search field) |
| 600 | server tile letters, server name, category/section caps, member names, user panel name, active "Übersicht", avatar initials, unread channel names, tooltip name |
| 650 | main header channel name `h1`, "Mitglieder" heading, member group headings |
| 700 | unread count pills, "LIVE" chips, sidebar brand "Gelabber" |

### 2.4 Port notes (egui)

- egui cannot set variable axes. Generate static instances from the variable TTF
  (`fonttools varLib.instancer`) for wght 400, 500, 600, 650, 700 at opsz 14 (browsers
  use `font-optical-sizing:auto`, i.e. opsz = font-size clamped to 14–32; every shell
  text is ≤ 16px, so opsz 14 for ≤14px; use opsz = size for 15/16px headings if you
  want the last few %). Register each as its own egui `FontFamily::Name`.
- CSS centres the glyph box (1.20996 em) in the line box: top of glyph box =
  `(line_height − 1.20996·size)/2` from the line-box top; baseline =
  that + 0.96875·size. Use this to place egui galleys so text lands on the same pixel
  rows as Chrome.
- Letter-spacing values below must be applied (egui `TextFormat::extra_letter_spacing`).

---------------------------------------------------------------------------------

## 3. Shell layout at 1280×800 (and 1600×900)

Component tree (route `/s/$serverId/c/$channelId`, `pages/WorkspaceLayout.tsx:97-209`,
`pages/ChannelPage.tsx:68-106`):

```
.app-shell.app-shell-workspace  (100vh, bg --lr-bg)          index.css:280-302
└ .workspace (flex row, h 100%)                               index.css:316-321
  ├ dialog.workspace-navigation (w 288, static on desktop)    index.css:398-428
  │  ├ nav.server-rail      x 0..64                           ServerRail.tsx:147-168
  │  └ aside.channel-sidebar x 64..288                        ChannelSidebar.tsx:206-357
  └ .workspace-main (flex 1, column)                          index.css:322-330
     ├ header.workspace-mobile-topbar  display:none ≥801px    index.css:339-341
     └ main.workspace-content
        └ ChannelPage row: [.lr-channel-content flex 1][dialog.member-panel-drawer w 248]
section.voice-session-dock (fixed, only in a call)            room.css:936-996
```

Absolute boxes (x from left, y from top, width × height):

| Box | 1280×800 | 1600×900 | Background | Separator |
|---|---|---|---|---|
| Server rail | x0 y0 64×800 | 64×900 | `--lr-rail` | 1px `--lr-border` on its right edge (x=63) |
| Channel sidebar | x64 y0 224×800 | 224×900 | `--lr-panel` | 1px `--lr-border` right edge (x=287) |
| Main column | x288 y0 744×800 | x288 1064×900 | `--lr-bg` | – |
| Main header | x288 y0 744×52 | 1064×52 | `--lr-bg` | 1px `--lr-border` bottom (y=51) |
| Member panel | x1032 y0 248×800 | x1352 248×900 | `--lr-panel` | 1px `--lr-border` left edge (x=1032 / 1352) |
| User panel | x64 y744 223×56 | y844 | `--lr-panel` | 1px `--lr-border` top (y=744 / 844) |
| Call bar (in a call) | x288 y744 992×56 | x288 y844 1312×56 | `--lr-panel` | 1px `--lr-border` top |

With the member panel toggled off (`MemberPanelToggle`), the main column is 992 / 1312 wide.
While a call runs, `.workspace-content` gets `padding-bottom: --lr-media-space` (56px)
(`index.css:392-395`), so main column AND member panel end at y=744 (1280×800) and
the call bar sits under both of them. The channel sidebar and the rail are NOT
shortened: the user panel (x64..287) and the call bar (x288..1280) form one
continuous 56px strip with the same top line and colour. The rail column keeps
running to the bottom edge (the rail has no bottom strip).

---------------------------------------------------------------------------------

## 4. ServerRail (`components/ServerRail.tsx`, CSS `index.css:437-631`)

### 4.1 Column

`index.css:437-447`: width 64 (content 63 because of the 1px right border), full
height, flex column, items centred horizontally, bg `--lr-rail`. No separators exist
between items (none in markup or CSS).

Horizontal centre of every 44px item: content width 63 → item x = 9.5..53.5 (Chrome
lays it at a half pixel; drawing at x=9.5 or 10 is indistinguishable for SSIM).

Vertical stack: Home tile (52) → server list (scroll, `flex: 0 1 auto`, so it is only
as tall as its content) → footer with "+" directly after the last server (it is NOT
pinned to the bottom).

### 4.2 Home / DM button (`ServerRail.tsx:184-229`, `index.css:448-468`)

| Property | Value |
|---|---|
| Wrapper `.server-home-tile` | full width × 52 (`--lr-header-height`), centres child; no bottom border |
| Button `.server-home-link` | 44×40, radius `--lr-radius` 7px, colour `--lr-text`, bg transparent |
| Position at 1280×800 | x 9.5..53.5, y 6..46 |
| Icon | `ChatIcon` size 27 (centred) |
| Hover or active (DM route) | bg `--lr-hover`; icon colour `--lr-accent-text` |
| Tooltip / a11y | native `title` "Direktnachrichten" |
| Edge pill | as 4.5 (`is-unread` when DM count>0 or dot) |
| Count badge | as 4.6 (DM mention/message count) |

### 4.3 Server list (`ServerRail.tsx:233-356`, `index.css:499-519`)

- Scroll container: padding 8px top/bottom, `overflow-y:auto`, thin scrollbar.
- Row pitch 60px (`TILE_PX`, `ServerRail.tsx:136`); each row centres a 44×44 tile, i.e.
  8px above and below.
- Tile k (0-based) at 1280×800: y = 68 + 60k .. 112 + 60k; x 9.5..53.5.
- Loading state: 3 placeholder tiles (`ServerRail.tsx:171-182`, `index.css:516-519,
  556-567`): rows 60px, tile bg `--lr-surface`, opacity pulse 1 → 0.45, 1.4s
  ease-in-out infinite alternate.

### 4.4 Server tile (`ServerRail.tsx:358-450`, `index.css:523-555`, `chat.css:2110-2112`)

| Property | Value |
|---|---|
| Size | 44×44 |
| Radius at rest | `calc(--lr-radius-lg * 2.2)` = 22px → a circle |
| Radius hover / active | `--lr-radius-lg` = 10px (animates 120ms `--lr-ease`) |
| Background without icon | tile tint `--lr-tile-N`, N = `tileTone(server.id)` (see 4.8) |
| Uploaded icon | image `object-fit: cover`, fills 44×44 with the same rounding (`index.css:549-555`) |
| Letters | `tileInitials(name)` (4.8), `600 15px / 1` Inter, letter-spacing 0.01em (0.15px), colour `--lr-text`, centred |
| Active (current server) | no ring: the 28px edge pill (4.5) and the hover radius mark it, so the accent ring stays the keyboard focus |
| Muted server | whole tile `opacity: 0.6` |

### 4.5 Edge pill / selected indicator (`index.css:469-498`)

Pseudo-element on tiles and the home button: x = item.left − 10 (≈ x −0.5..3.5 of the
window, i.e. flush with the rail's left edge), width 4px, vertically centred on the
item, radius `0 4px 4px 0` (rounded on the right only).

| State | Height | Colour |
|---|---|---|
| hidden (default) | 0 | `--lr-muted` (so grow/shrink never flashes accent) |
| `is-unread` (any unread/mention, not active) | 8px | `--lr-accent` |
| hover, not active | 16px (keeps unread colour if unread, else muted) | – |
| active (current server / DM route) | 28px | `--lr-text` |

Height and colour animate 120ms `--lr-ease`.

### 4.6 Unread count on rail items (`messages/UnreadBadge.tsx:11-66`, `components/ui/ui.css:421-462`, `index.css:568-573`)

Shown only when the mention/DM count > 0 (plain unread shows only the pill, no dot):

| Property | Value |
|---|---|
| Position | absolute, right −5px, bottom −5px of the 44px tile (or 44×40 home button) |
| Size | min 18×18, padding 0 5px, radius 999px (pill grows with digits) |
| Fill / text | bg `--lr-danger-bg` (#8f302d dark / #b33b38 light), text `--lr-on-danger` #ffffff |
| Font | `700 11px / 1`, letter-spacing 0, no uppercase |
| Ring | `box-shadow: 0 0 0 2px --lr-rail` |
| Text | count, `"99+"` when > 99 |
| Appear animation | scale .5→1 + opacity 0→1, 180ms ease-enter, only when it appears while visible (`index.css:894-902`) |

### 4.7 Tooltip beside a tile (`ServerRail.tsx:258-353`, `index.css:575-609`, `components/tooltipTiming.ts:6-11`)

| Property | Value |
|---|---|
| Anchor | left = tile right edge + 12px, vertical centre = tile centre |
| Box | max-width 260, padding 7px 10px, flex column gap 2px, border 1px `--lr-border`, radius `--lr-radius-sm` 5px, bg `--lr-elevated`, shadow `--lr-shadow-1`, z above everything |
| Line 1 | server name, `--lr-type-ui` with weight 600, colour `--lr-text`, wraps anywhere |
| Line 2 (optional) | `--lr-type-meta`, `--lr-muted`; text: `"<n> Erwähnung"` (n=1) / `"<n> Erwähnungen"` / `"Mehr als 99 Erwähnungen"`; else `"Ungelesene Nachrichten"`; else `"Stummgeschaltet"`; else none (`ServerRail.tsx:390-397`) |
| Timing | first hover waits 200ms; leaving keeps it 80ms; next tile within 400ms shows at once and the tip just moves |
| Enter animation | opacity 0→1, translateX 8→12px, 120ms ease-enter |
| Home button | no custom tooltip, only native title "Direktnachrichten" |

### 4.8 Tint and letter algorithms (`components/initials.ts`)

- `tileTone(id)` (`initials.ts:27-31`): `hash = 0; for each char of id: hash = (hash*31 + charCode) >>> 0` (u32 wrapping); tone = `hash % 8 + 1`. Use `u32::wrapping_mul(31).wrapping_add(c)`; iterate UTF-16 code units for ASCII ids (UUIDs).
- `personTint(name)` for avatar initials (`initials.ts:43-53`): FNV-1a 32-bit (`hash=0x811c9dc5; hash ^= codePoint; hash = hash*0x01000193` wrapping), then `hash ^= hash>>>16; hash *= 0x45d9f3b; hash ^= hash>>>16`; tone = `hash % 8 + 1`.
- `initials(name)` (`initials.ts:1-9`): first code point of the first two whitespace-separated words, uppercased; `"?"` if empty.
- `tileInitials(name)` (`initials.ts:16-21`): one-word name → first letter upper + second letter lower ("Gaming" → "Ga"); otherwise `initials`.

### 4.9 Add server (`ServerRail.tsx:155-165`, `index.css:610-631`)

| Property | Value |
|---|---|
| Footer | full width, padding 5px top, 16px bottom → 65px tall, directly below the list |
| Button | 44×44 circle (radius 50%), border 1px DASHED `--lr-border-strong`, bg transparent |
| Icon | `PlusIcon` size 20, colour `--lr-online-text` (#85d69b dark / #277643 light) |
| Hover | border becomes solid, bg `--lr-hover` |
| Tooltip | title "Server hinzufügen" |
| Example | 3 servers at 1280×800: list y 52..248, footer y 248..313, button y 253..297 |

---------------------------------------------------------------------------------

## 5. ChannelSidebar (`components/ChannelSidebar.tsx`, CSS `index.css:633-973, 1953-1975, 2124-2171`, `components/shell.css:4-50`)

### 5.1 Column (`index.css:633-646`)

Width 224 (content 223, 1px right border `--lr-border`), full height, flex column,
bg `--lr-panel`, `overflow-y:auto` (thin scrollbar). Children top to bottom:
header 52 → "Übersicht" link → channel list (flex 1, min-height 156) → user panel
(pinned to bottom with `margin-top:auto`).

### 5.2 Header (`ChannelSidebar.tsx:208-283`, `index.css:648-748`)

| Element | Spec |
|---|---|
| `header.server-sidebar-heading` | height 52 incl. 1px bottom border `--lr-border`; flex row, items centred, gap 2px, padding 0 6px 0 8px |
| Server-menu button (`summary`) | flex 1; height 38; padding 0 8px; gap 6px; radius `--lr-radius-sm` 5px; hover or menu open → bg `--lr-hover`. Native title = server name |
| Server name `h2` | `600 15px / 1.3` (line box 19.5), letter-spacing −0.01em (−0.15px), `--lr-text`, single line, ellipsis |
| Muted mark | only when the server is muted: `BellOffIcon` 15px, `--lr-muted`, title "Stummgeschaltet" |
| Chevron | `ChevronIcon` 16px, `--lr-muted`, pushed right (`margin-left:auto`); rotates 180° while the menu is open (120ms) |
| Invite button | `.shell-icon-button` 30×34, radius 7, `--lr-muted`; hover bg `--lr-hover` + `--lr-text`; icon `LinkIcon` 16px; title "Einladen" |

Geometry at 1280×800 (header content is 51 tall above the 1px border): menu button x 72..249, y 6.5..44.5; invite button x 251..281, y 8.5..42.5.

Server menu popup (`ChannelSidebar.tsx:220-278`, `index.css:713-748`, `shell.css:4-50`):

| Property | Value |
|---|---|
| Position | absolute, top = button bottom + 6px, left = button left; width max(button width, 208px) |
| Box | flex column gap 2px, padding 8px, border 1px `--lr-border`, radius `--lr-radius-lg` 10px, bg `--lr-elevated`, shadow `--lr-shadow-1`, z 35 |
| Item | min-height 36, padding 0 10px, gap 10px, radius 5px, `--lr-type-ui` (500 13px/1.35), `--lr-text`; icon 16px `--lr-muted`; hover bg `--lr-hover` |
| Items (order) | `LinkIcon` "Einladen"; `BellIcon` (or `BellOffIcon` if muted) "Benachrichtigungen…"; if `manage_channels`: `PlusIcon` "Kanal erstellen", `PlusIcon` "Kategorie erstellen"; last: `GearIcon` "Servereinstellungen" (with `manage_server`) or `UsersIcon` "Mitglieder" |
| Open / close | opacity 0→1 + scale .96→1 from top-left, 180ms ease-enter; close 130ms ease-exit |
| Geometry at 1280×800 | the `details` wrapper is 72..249 × 38 (y 6.5..44.5), so width = max(177, 208) = 208 → popup x 72..280, top y 50.5; height = 2·8 padding + 2 border + n·36 + (n−1)·2 (5 items → 196, so y 50.5..246.5) |

### 5.3 "Übersicht" link (`ChannelSidebar.tsx:285-294`, `index.css:749-775`)

| Property | Value |
|---|---|
| Box | margin 10px 8px 2px → x 72..279 (207 wide), y 62..96 at 1280×800; min-height 34; padding 6px 10px; gap 10px; radius 7 |
| Content | `ChatIcon` 20px + "Übersicht", 14px weight 400, line box 21 |
| Rest | colour `--lr-muted`, no bg |
| Hover | bg `--lr-hover`, colour `--lr-text` |
| Active (route `/s/<id>` exactly) | bg `--lr-selected`, colour `--lr-text`, weight 600, icon `--lr-accent-text`; stays `--lr-selected` on hover |

### 5.4 Channel list container (`ChannelSidebar.tsx:372-607`, `index.css:776-787`)

- Starts at y 98 (1280×800). Padding 0 8px 12px → rows span x 72..279 (207 wide).
- Virtualised flat list; each row's SLOT height (`ChannelSidebar.tsx:153-159`):

| Row kind | Slot | Visible box |
|---|---|---|
| section heading ("Sprachkanäle") | 34 | 34 |
| category | 34 | 34 |
| channel (text or voice) | 36 | 34 (top-aligned, so 2px gap below each channel) |
| voice participant | 28 | 28 |
| empty category ("Leer") | 28 | text only |

- Order (`servers/rows.ts:53-79`): uncategorised text channels; if any uncategorised
  voice channels: section row "Sprachkanäle" then those voice channels; then each
  category followed by its channels (or a "Leer" row). Each voice channel is directly
  followed by its participants sorted by name (`localeCompare` "de")
  (`ChannelSidebar.tsx:109-141`).
- Empty server: `<p>` "Noch keine Kanäle." (`ChannelSidebar.tsx:464-466`), style
  `.sidebar-empty`: padding 24px 22px, 14px, line-height 1.6, `--lr-muted`.

### 5.5 Category row and section heading (`ChannelSidebar.tsx:779-865`, `index.css:788-817`)

| Property | Category | Section "Sprachkanäle" |
|---|---|---|
| Box | 34 tall, flex, items at bottom (flex-end), space-between, gap 5, padding 0 3px 5px 8px | 34 tall, flex-end, padding 0 3px 7px 8px |
| Text | `--lr-type-caps` 600 11px/1.3, letter-spacing .06em, UPPERCASE, `--lr-muted`, ellipsis; margin-bottom 2px | same |
| Text box bottom | row top + 27 (glyph line box 14.3 tall: 12.7..27) | row top + 27 |
| Hover actions (only with `manage_channels`) | right side, opacity 0 → 1 on row hover/focus-within (Tailwind `transition`: 150ms cubic-bezier(0.4,0,0.2,1)): three 26×26 `.shell-icon-button.sidebar-action-small` (radius 7, `--lr-muted`, hover bg `--lr-hover` + `--lr-text`, no gap) with `PlusIcon` 14 "Kanal in dieser Kategorie", `PencilIcon` 14 "Kategorie umbenennen", `TrashIcon` 14 "Kategorie löschen". Unlike channel rows these stay IN FLOW (`ChannelSidebar.tsx:835`, no absolute rule): the invisible 78px block always takes room, so the category name ellipsises at row width − 3 − 78 − 5 gap − 8 = 113px of text (row 207 wide); buttons bottom-aligned (flex-end + 5px padding) → button y = row top + 3 .. + 29 | none |
| Empty category row | "Leer": padding 0 8px, 12px, `--lr-muted` (`index.css:915-919`) | – |

### 5.6 Channel row (`ChannelSidebar.tsx:867-1010`, `index.css:818-936`, `chat.css:2105-2112`)

Structure: `div.channel-row` (34 tall, flex, items centred, gap 2, padding-right 3,
radius 7) › `a.channel-row-link` (flex 1, padding 0 10px, gap 10px, full height) ›
[icon][name][draft?][count?][LIVE?].

| Element | Spec |
|---|---|
| Icon | `HashIcon` (text) / `SpeakerIcon` (voice), drawn at 17×17 (CSS overrides the `size={25}` attribute); stroke 2 in a 24 viewBox → scaled; colour = row colour |
| Name | 14px weight 400, line box 21, flex 1, single line ellipsis |
| Rest colours | text `--lr-muted`, no bg |
| Hover | bg `--lr-hover`, text `--lr-text` |
| Active (open channel) | bg `--lr-selected`, text `--lr-text`, icon `--lr-accent-text`; stays selected on hover (rule order) |
| In voice (`is-in-voice`, your call's channel) | text `--lr-text`, icon `--lr-online-text`; native title `"<name> · verbunden"` |
| Unread text channel (`is-unread`) | name `--lr-text` + weight 600; plus an edge bar: x = row.left − 8 (= sidebar x 0), width 3, height 16, vertically centred, radius `0 3px 3px 0`, `--lr-accent` |
| Muted text channel | name `opacity .5` |
| Pending (optimistic) | whole row `opacity .5`, not clickable |
| Draft mark (text, not active) | `PencilIcon` 13px `--lr-muted`, title "Entwurf" |
| Mention/DM count | the pill of 4.6 without ring/offset: `margin-left:auto` at the right end of the link (right edge = row right − 13px) |
| LIVE chip (voice channel with someone live) | `margin-left:auto`; padding 1px 5px; radius `--lr-radius-small` 5px; bg `--lr-danger-bg`; text `#fff` `700 10px`, letter-spacing .4px, UPPERCASE "LIVE"; line box 15 → chip 17 tall; title = `liveSentence(...)` |
| Hover actions (`manage_channels`) | absolute (`@media (hover:hover)`, `index.css:1965-1975`), right 3px, vertically centred, padding-left 6, radius 7, `background: inherit` = row's current bg (covers the name/count end); total 6 + 26 + 26 = 58 wide → x = row right − 61 .. row right − 3, y = row top + 4 .. + 30; opacity 0 → 1 on hover/focus-within (150ms cubic-bezier(0.4,0,0.2,1)): two 26×26 buttons `PencilIcon` 14 "Kanal bearbeiten", `TrashIcon` 14 "Kanal löschen". Takes no room while hidden |
| Right-click | row menu popover (`ChannelSidebar.tsx:613-714`), items `.shell-menu-item` (min-height 36, padding 0 10, gap 10, radius 5, `--lr-type-ui`, icon `--lr-muted`, hover `--lr-hover`) with 18px icons, list gap 2 (`shell.css:15-19`). Channel: `PencilIcon` "Kanal bearbeiten"; only for TEXT channels `BellIcon` "Benachrichtigungen…"; separator (`hr`, 1px top `--lr-border`, margin 4px 6px); danger `TrashIcon` "Kanal löschen" (text AND icon `--lr-danger`). Category: `PlusIcon` "Kanal in dieser Kategorie", `PencilIcon` "Kategorie umbenennen", separator, `TrashIcon` "Kategorie löschen" (danger). Popover label (a11y): `#name` / `name` / `Kategorie <name>`. For members without manage rights: right-click on a text channel opens notification settings; voice: nothing |
| Popover box (`ui/ui.css:635-660`, `ui/popoverPosition.ts:9-10`) | `.ui-popover`: fixed, z 60, min-width 180, max-width min(360, 100vw−16), padding 8, border 1px `--lr-border`, radius `--lr-radius-lg` 10, bg `--lr-elevated`, shadow `--lr-shadow-1`. Default placement `bottom-start`: top = anchor bottom + 6 (GAP), left = anchor left; flips above (anchor top − 6 − height) when it does not fit; kept 8px (MARGIN) inside the viewport. Anchor = the row link (channel) or the category row. Opens opacity 0→1 + scale .96→1 from the anchor side (180ms ease-enter), closes 130ms ease-exit |

### 5.7 Voice occupants (`ChannelSidebar.tsx:716-777`, `index.css:2124-2171`)

| Element | Spec |
|---|---|
| Row | 28 tall, full row width (207), flex, centred, gap 8, padding 0 8px 0 36px, radius 7, 13px weight 400, `--lr-muted`; hover bg `--lr-hover`; title "Dein Profil" (self) / "Profil und Lautstärke" |
| Avatar | 20×20 circle, initials 600 9px (5.x), on `personTint` tile; image cover |
| Name | flex 1, ellipsis; speaking → `--lr-text` |
| Speaking avatar | ring `0 0 0 2px --lr-speaking` + glow (`0 0 0 4px speaking@30%`, `0 0 8px 4px speaking@45%`) pulsing opacity .45↔1 over 1.2s ease-in-out (`index.css:1258-1288`). Only when not muted/deafened |
| Live | `LiveIcon` 13px, `--lr-danger`, title "Live" |
| Locally muted (not self) | `SpeakerOffIcon` 16px, `--lr-muted`, title "Lokal stummgeschaltet" (`UserVolumeControl.tsx:98-105`, `index.css:2213-2217`) |
| Muted / deafened | `MicOffIcon` 13px or `HeadsetOffIcon` 13px (deafened wins), `--lr-danger`, title "stumm" / "taub" |
| Unknown member name | "Jemand" |

### 5.8 Worked example (1280×800, server with 2 text, 1 loose voice room with 2 people, category "Spiele" with 2 channels)

| Row | Slot y | Visible y |
|---|---|---|
| # allgemein | 98–134 | 98–132 |
| # memes | 134–170 | 134–168 |
| SPRACHKANÄLE | 170–204 | text 182.7–197 |
| Lounge (voice) | 204–240 | 204–238 |
| participant A | 240–268 | 240–268 |
| participant B | 268–296 | 268–296 |
| SPIELE | 296–330 | text 308.7–323 |
| # minecraft | 330–366 | 330–364 |
| # valorant | 366–402 | 366–400 |

Text in a 34px channel row: line box 21 at y+6.5; glyph box (16.94px) at y+8.53.

### 5.9 Other sidebars sharing the column (same width/bg/border)

- Home (`/`): `.sidebar-brand` "Gelabber" 52 tall incl. bottom border, padding 0 16px,
  16px weight 700, letter-spacing −0.3px; empty hint "Keine Server. Erstelle einen über
  + in der Serverleiste oder öffne einen Einladungslink." (`WorkspaceLayout.tsx:73-90`,
  `index.css:656-661`).
- Loading: brand + 4 bars 22px tall, radius 7, `--lr-surface`, 14px apart, inside
  padding 24px 18px → bars x 82..269 (187 wide), first bar y 76..98 at 1280×800
  (`SidebarPlaceholder.tsx:15-49`, `index.css:964-973`). Each bar has Tailwind
  `animate-pulse`: opacity 1 → 0.5 at 50% → 1, 2s cubic-bezier(0.4,0,0.6,1) infinite
  (off with reduced motion). User panel stays at the bottom. Failure:
  message + button "Erneut versuchen" / "Wird geladen…" (36 tall, padding 0 12,
  border 1px `--lr-border`, radius 5, 600).
- DMs (`components/DmSidebar.tsx`, `index.css:974-1022`): header "Nachrichten"
  (600 15px/1.3) with padding 0 8px 0 16px and a 32×32 ghost icon button
  (`PlusIcon` 18, title "Neue Nachricht", radius 5, `--lr-muted`, hover `--lr-hover`).
  List padding-top 8; DM row 44 tall in a 48 slot, padding 0 10, gap 10, radius 7,
  14px, `--lr-muted`; 32px avatar with presence dot; hover bg `--lr-hover` + text;
  active `--lr-selected` + text, BUT hover wins over active for DM rows (rule order
  `index.css:1015-1022`). Unread: name 600 `--lr-text` + 3×16 accent edge bar.
  Muted: name opacity .5.

---------------------------------------------------------------------------------

## 6. Main header bar (`components/MessagePane.tsx:394-426`, `pages/ChannelPage.tsx:52-106`, `components/chat.css:4-93, 1931-2058`, `index.css:344-362`)

There is NO channel topic: the `Channel` type has no topic field and no header shows one.

| Element | Spec |
|---|---|
| `header.lr-channel-header` | height 52 incl. 1px bottom border `--lr-border`; flex row, centred, gap 10; padding 0 12px 0 20px; bg `--lr-bg`; colour `--lr-text` |
| Channel icon | `HashIcon` / `SpeakerIcon` 18px, `--lr-muted`; gap 10 to the name |
| Name `h1` | 15px weight 650, letter-spacing −0.1px, line box 22.5, ellipsis |
| Server name line | hidden on desktop (`index.css:350-352`; shown only ≤800px) |
| Live pill (when someone in the server is live) | after the title, `margin-left:auto`; height 30, padding 0 4 0 6, gap 8, radius 15, border 1px (1.3 table), bg (1.3), 12px; contains "LIVE" badge (18 tall, padding 0 6, radius 9, `--lr-danger-bg`, #fff 700 10px, letter-spacing .4px, `chat.css:1689-1703`) + link `"<names> in <channel> +<more>"` (`--lr-text`, ellipsis, underline on hover) + optional button "Beitreten" (not in a call) / "Zum Kanal" (padding 3px 10px, radius 12, `--lr-danger-bg`, #fff, 650) (`voice/LiveHint.tsx:72-90`). Then the search loses its auto margin (`chat.css:2009-2045`) |
| Search field (text channels and DMs) | `margin-left:auto`; 200×30 (260 wide when focused or holding text, width animates 100ms); padding 0 10; gap 8; border 1px `--lr-border`; radius 7; bg `--lr-panel`; text 13px `--lr-muted`; `SearchIcon` 16px; input text `--lr-text`; placeholder "Suchen" in (1.3); hover/with text: icon/text `--lr-text`, border (1.3); focused: border (1.3); title "Wörter oder eine Phrase in Anführungszeichen" |
| Shortcut hint | `kbd` "Strg F" (macOS "⌘ F"), right end of the field (margin-left auto), padding 0 5, border 1px `--lr-border`, radius 5, 11px weight 400 `--lr-muted` (stays muted on hover; `chat.css:1931-1939`; hidden ≤800px, `chat.css:1724`), line-height inherited 1.5 → 16.5 line box → kbd box 18.5 tall, centred in the 28px inner height; hidden while the field has text |
| Bell (`NotificationSettingsDialog.tsx:311-329`) | `.lr-header-icon` 32×32, radius 7, `--lr-muted`; `BellIcon` 16 or `BellOffIcon` 16 when muted; title "Benachrichtigungen" / "Stummgeschaltet"; hover bg `--lr-hover` + `--lr-text` |
| Member list toggle (`MemberPanel.tsx:24-40`) | `.lr-header-icon` 32×32, `UsersIcon` 18; while the panel is shown it has class `is-on` = bg `--lr-hover`, colour `--lr-text` permanently; title "Mitgliederliste ausblenden" / "Mitgliederliste einblenden" |
| Voice channel header | title, optional Live pill, spacer (flex 1), member toggle only (no search, no bell) (`ChannelPage.tsx:73-77`) |

Geometry, text channel, no Live pill:

| Item | 1280×800 | 1600×900 |
|---|---|---|
| Icon | x 308..326, y 16.5..34.5 | same |
| Name starts | x 336 | x 336 |
| Search (rest) | x 736..936, y 10.5..40.5 | x 1056..1256 |
| Bell | x 946..978, y 9.5..41.5 | x 1266..1298 |
| Member toggle | x 988..1020, y 9.5..41.5 | x 1308..1340 |

(Focused search grows 60px to the left: x 676..936 at 1280.)

---------------------------------------------------------------------------------

## 7. MemberPanel (`components/MemberPanel.tsx:42-229`, `index.css:1411-1544`, `components/VoiceStateIcons.tsx`)

Shown on channel pages (`ChannelPage.tsx:104`) and forced visible on the server
overview (`pages/CommunityOverview.tsx:105`). On desktop the `dialog.member-panel-drawer`
is a static flex child 248 wide, full height (`index.css:429-432`). Toggled off →
not rendered (`display:none`, `chat.css:2002-2006`); the choice is remembered
(`components/memberPanelState.ts`; voice rooms with video start closed).

| Element | Spec |
|---|---|
| `aside.member-panel` | flex column, 1px left border `--lr-border`, bg `--lr-panel` |
| Heading | 52 incl. 1px bottom border; padding 0 16; flex, centred, space-between, gap 8; left `h2` "Mitglieder" 15px weight 650, letter-spacing −0.2px, `--lr-text`; right `p` member count, 12px `--lr-muted` |
| List | scrolls; padding 14px 10px 16px → rows x 1043..1270 at 1280 (227 wide) |
| Group | margin 8px top (first) / 18px bottom; adjacent groups 18px apart (margins collapse) |
| Group heading | flex, gap 7, margin 0 6px 6px; 11px weight 650, UPPERCASE, letter-spacing .6px, `--lr-muted`, line box 16.5; text `"<Label> — <n>"` (em dash), labels "Online", "Abwesend", "Offline" in that order, empty groups omitted, members sorted by name with `localeCompare(…, "de")` (`ws/live.ts:159-177`); a member with no presence entry for this server counts as Offline (`presenceOf` default `"x"`, `ws/live.ts:71-77`) |
| Row | margin-bottom 2; button full width, min-height 42, padding 4px 6px, gap 10, radius 7, flex centred; hover bg `--lr-hover`; title "Profil von <Name>" |
| Avatar | 32×32 circle, initials 600 13px, presence dot (8.1) |
| Name line | 14px weight 600 `--lr-text`, ellipsis; own row appends `" · Du"` as 11px weight 400 `--lr-muted` |
| Status text (optional) | margin-top 2, 12px `--lr-muted`, line box 18, ellipsis |
| Voice line (optional) | margin-top 3, 11px weight 400, line-height 1.4 (15.4), `--lr-muted`, WRAPS (`overflow-wrap:anywhere`, no ellipsis, `index.css:1524-1531`): voice channel name (fallback "Im Sprachkanal"), or `"Live in <channel>"` / "Live" |
| Right column | 22 wide, flex column centred, gap 5, `--lr-muted`: `VoiceStateIcons` (a 32px-wide box overflowing the 22px column, right-aligned, two 14×14 slots 2px apart: slot 1 `MicOffIcon` 14 if muted/deafened else `SpeakerIcon` 14 if in voice; slot 2 `HeadsetOffIcon` 14 if deafened; title "In <ch>" / "Stumm in <ch>" / "Taub in <ch>"), then `LiveIcon` 20 if live |
| Offline row | avatar opacity .55, name `--lr-muted` |

Row heights: name only → 42; with status → 49 (4+21+2+18+4); with voice line →
47.4 (4+21+3+15.4+4); with status AND voice line → 67.4. Row pitch adds 2px.
At 1280×800: group 1 heading text y 74..90.5; first row y 96.5.

---------------------------------------------------------------------------------

## 8. Avatars and presence

### 8.1 Avatar (`components/Avatar.tsx:21-55`, `components/PresenceAvatar.tsx`, `index.css:1234-1332`)

| Size class | Diameter | Initials font | Used in |
|---|---|---|---|
| sm (default) | 32 | 600 12px | DM rows |
| member override | 32 | 600 13px | member list (`index.css:1489-1492`) |
| md | 36 | 600 14px | user panel, status popover |
| participant override | 20 | 600 9px | sidebar voice occupants |
| lg / hero, room | 64 / 112 | 20px / 32px | elsewhere |

Initials colour `--lr-text` on `personTint(name)`; image (no-referrer) covers the circle;
broken image falls back to initials.

Presence dot (`index.css:1313-1332`): absolute at the avatar's bottom-right corner
(right 0, bottom 0), size max(23% of avatar, 10px) → 10×10 for 20–43px avatars,
circle, 2px border in `--lr-bg` (#17191a / #f8f5f0, even on panel backgrounds), so the
fill is a 6px disc. Fill: online `o` `--lr-online`, away `i` `--lr-warning`, offline `x`
`--lr-muted`. Title/label "Online" / "Abwesend" / "Offline".

### 8.2 Avatar decorations (`components/AvatarFrame.tsx`, `index.css:1335-1409`)

Colour `--deco-color` = person's accent (else `--lr-accent`).
ring: `0 0 0 2px --lr-bg, 0 0 0 4px deco`; glow: `0 0 0 2px deco, 0 0 10px 3px deco@55%`;
rainbow: conic ring 3px outside (#ff5f6d, #ffc371, #f9f871, #5ee7a1, #4fb3ff, #a07cff,
#ff5f6d) with a 1.5px `--lr-bg` gap; stars: `0 0 0 2px bg, 0 0 0 3.5px deco` plus two
"✦" glyphs (top −14%/right −10%, size max(10, .36·d); bottom −6%/left −12%, size
max(8, .24·d)).

---------------------------------------------------------------------------------

## 9. UserPanel (bottom-left) (`components/UserPanel.tsx:31-125`, `index.css:1069-1233`)

There are NO microphone / headphone buttons in the user panel. Mute, deafen, camera,
share, Live and leave live in the call bar (section 10). The panel holds: profile
button, settings gear, user-menu chevron.

| Element | Spec |
|---|---|
| `.user-dock` | `margin-top:auto` (bottom of the sidebar column) |
| `footer.user-panel` | min-height 56 incl. 1px top border `--lr-border`; flex, centred, gap 4; padding 0 8px; bg `--lr-panel` |
| Profile button | flex 1, min-width 0; flex centred, gap 9; padding 2px 4px 2px 2px; radius 7; hover or popover open → bg `--lr-hover` |
| Avatar | 36px with presence dot (own presence for the viewed server, fallback online) and decoration |
| Name | 14px weight 600 `--lr-text`, line box 21, ellipsis |
| Subtitle | margin-top 2; 11px `--lr-muted`, line box 16.5, ellipsis; text = "Online" / "Abwesend" / "Offline", plus `" · <status text>"` when set (`components/userPresence.ts:26-33`) |
| Settings | link styled `.shell-icon-button` 30×36 (height `--lr-control-md`), radius 7, `--lr-muted`, hover bg `--lr-hover` + `--lr-text`; `GearIcon` 21px; title "Einstellungen"; opens settings area "darstellung" |
| User menu | `summary.shell-icon-button` 30×36, `ChevronIcon` 18px rotated 180° (points up); aria "Benutzermenü" |

Geometry at 1280×800: panel x 64..287, y 744..800 (content 745..800, 55 tall).
Profile button x 72..211 (139 wide). Its height is set by the TEXT, not the avatar:
name line box 21 (14px × 1.5) + 2 margin + subtitle line box 16.5 (11px × 1.5) = 39.5,
+ 2+2 padding = 43.5 → button y 750.75..794.25 (centred in 745..800). Avatar 36 is
centred in it: x 74..110, y 754.5..790.5. Name line box y 752.75..773.75, subtitle
line box y 775.75..792.25; text from x 119 to 207 (88px wide, ellipsis). Gear
x 215..245, chevron x 249..279, both y 754.5..790.5. (At 1600×900 add 100 to every y.)

User menu popup (`index.css:1196-1233`): absolute, right edge = chevron right, bottom
43px above the chevron's bottom; min-width 180; flex column gap 3; padding 8; bg
`--lr-panel`; border 1px `--lr-border`; radius 7; shadow `0 8px 25px rgba(0,0,0,.2)`;
grows from bottom-right (opacity/scale .96, 180ms ease-enter; closes 130ms ease-exit),
z 35. Items: block, 14px weight 400, line box 21 → 37 tall, padding 8px 10px,
radius 5, colour `--lr-text`, left-aligned, no icons, hover bg `--lr-hover`; in order:
"Profil bearbeiten", "Einstellungen", "Server wechseln …" (desktop app only, title
"Strg+Umschalt+S"), "Abmelden". Popup height with 4 items: 2 + 16 + 4·37 + 3·3 = 175.
At 1280×800: x 99..279 (180 wide, right edge = chevron right 279), bottom y = 790.5 − 43
= 747.5, so it overlaps the panel's top line by 3.5px. Chevron icon points UP at rest
(rotate 180°) and does not rotate further when open.

Status popover (click on the profile button, `UserPanel.tsx:66-79, 127-234`,
`index.css:1140-1195`, `ui.css:636-661`): 320 wide, placed above-start of the button;
popover box padding 8, border 1px `--lr-border`, radius 10, bg `--lr-elevated`, shadow
`--lr-shadow-1`. Card: flex column gap 12, padding 4. Row 1: avatar 36 + name
(`--lr-type-title-sm`, ellipsis) + presence label (`--lr-type-meta`, muted). Form:
label "Status", text input + small button "Speichern" ("Speichert…" while saving);
messages "Gespeichert." / "Nicht gespeichert – erneut versuchen". Footer link
"Profil bearbeiten": min-height 36, padding 0 8, 1px top border, `--lr-type-ui`,
hover `--lr-hover`, bottom corners radius 5, bleeds −4px to the card edges.

---------------------------------------------------------------------------------

## 10. How the user panel joins the call bar (`voice/room.css:933-996, 1161-1251`, `index.css:151-153, 1078-1084`)

- Both are exactly `--lr-dock-height` 56px tall with a 1px `--lr-border` top line and
  `--lr-panel` fill, aligned to the window bottom, so they read as one strip from x=64
  to the right edge. The rail (x 0..63, `--lr-rail`) is not part of the strip.
- Call bar at ≥801px: fixed, left 288, right 0, bottom 0, min-height 56, padding
  8px 16px 8px 16px, z 40, flex column (one row normally), 12px base font. Rows centre
  vertically (auto margins).
- Row layout (`.voice-session-call`, gap 6): status block (`flex: 0 1 340px`: quality
  bars 36×36 or `SignalIcon`/`WarningIcon` 16; two 12px lines: state weight 600 in
  `--lr-online-text` when connected, warning-mix when reconnecting, `--lr-danger` when
  failed; second line destination `--lr-muted`) → people in the call (centred, auto
  margins) → controls (`margin-left:auto`, gap 4) → "Verlassen" button.
- Controls: 36 tall, min-width 36, padding 0 8, radius 7, transparent, `--lr-muted`;
  hover `--lr-hover` + `--lr-text`; on (camera/share) `--lr-selected` + `--lr-accent-text`;
  off (mic/headset closed) `--lr-danger` text on `danger-bg@16%`; Live `--lr-danger-bg`
  + #fff; leave: margin-left 4, padding 0 12, `--lr-danger-bg` + #fff, hover darker
  (1.3), label "Verlassen" (`--lr-type-ui`).
- Joining slides the bar up from below (translateY 100% → 0, 240ms ease-enter);
  leaving slides down (170ms ease-exit).
- While the bar shows, the main column and member panel stop at its top line (section 3).
- The full call bar is specified in the voice/call-bar spec; only the join geometry is
  defined here.

---------------------------------------------------------------------------------

## 11. Icons used by the shell (`components/Icons.tsx`)

All icons: `viewBox 0 0 24 24`, `fill none`, `stroke currentColor`, stroke-width 2
(in viewBox units, so it scales: 2/24 of the drawn size), round caps and joins
(`Icons.tsx:5-19`). Default size 16. Draw by scaling the path; egui: tessellate the
SVG path (e.g. with `lyon` or by rasterising via `resvg` at the target size).

| Icon | Line | Path data | Shell uses (size) |
|---|---|---|---|
| HashIcon | 21 | `M4 9h16M4 15h16M10 3 8 21M16 3l-2 18` | channel row (17), header (18) |
| SpeakerIcon | 29 | `M11 5 6 9H3v6h3l5 4V5Z` + `M15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13` | voice row (17), header (18), member voice slot (14) |
| PlusIcon | 38 | `M12 5v14M5 12h14` | add server (20), menu (16), category action (14), DM new (18) |
| GearIcon | 46 | circle cx12 cy12 r3 + long path (see file) | user panel (21), menu (16) |
| PencilIcon | 55 | `M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z` | draft (13), row actions (14), row menu (18) |
| TrashIcon | 72 | `M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6M10 11v6M14 11v6` | row actions (14), row menu (18) |
| LinkIcon | 80 | `M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1 1` + `M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1-1` | invite button (16), menu (16) |
| ChatIcon | 89 | `M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z` | home/DM (27), Übersicht (20) |
| MicOffIcon | 106 | `M9 9v0a3 3 0 0 0 3 3m3-3V6a3 3 0 0 0-4.4-2.6` + `M19 10v1a7 7 0 0 1-9.9 6.4M5 10v1a7 7 0 0 0 4 6.3M12 18v3M4 4l16 16` | participant (13), member (14) |
| HeadsetOffIcon | 125 | `M4 14v-2a8 8 0 0 1 12.5-6.6M20 11v3` + `M4 14a2 2 0 0 0-2 2v1a2 2 0 0 0 2 2h1v-5H4Z` + `M20 14h-1v2.5M4 4l16 16` | participant (13), member (14) |
| LiveIcon | 155 | filled circle cx12 cy12 r3 (no stroke) + `M5 12a7 7 0 0 1 2-4.9M19 12a7 7 0 0 0-2-4.9M8.5 7.5a9 9 0 0 1 7 0M8.5 16.5a9 9 0 0 0 7 0` | participant (13), member (20) |
| ChevronIcon | 182 | `m6 9 6 6 6-6` | server menu (16), user menu (18, rotated 180°) |
| SearchIcon | 214 | circle cx11 cy11 r7 + `m20 20-4-4` | header search (16) |
| SpeakerOffIcon | 273 | `M11 5 6 9H3v6h3l5 4V5Z` + `m22 9-6 6M16 9l6 6` | locally muted (16) |
| UsersIcon | 282 | circle cx9 cy7.5 r3.5 + `M2.5 20a6.5 6.5 0 0 1 13 0M16 4.5a3.5 3.5 0 0 1 0 6.5M18 14a6 6 0 0 1 3.5 6` | member toggle (18), menu (16) |
| BellIcon | 364 | `M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9` + `M10.3 21a1.94 1.94 0 0 0 3.4 0` | header bell (16), menu (16) |
| BellOffIcon | 373 | `M8.7 3A6 6 0 0 1 18 8a21.3 21.3 0 0 0 .6 5M17 17H3s3-2 3-9a4.67 4.67 0 0 1 .3-1.7` + `M10.3 21a1.94 1.94 0 0 0 3.4 0M2 2l20 20` | muted server (15), header (16), menu (16) |
| MoreIcon | 390 | circles r1 at (5,12),(12,12),(19,12) | touch only |
| CloseIcon / MenuIcon | 240 / 382 | – | phone drawer only |

GearIcon path (`Icons.tsx:50`): `M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z`.

The file has 49 icons (`Icons.tsx:21-460`); the rest belong to other areas.

---------------------------------------------------------------------------------

## 12. German strings in the shell (verbatim)

| Where | String |
|---|---|
| Skip link (`AppShell.tsx:77`) | Zum Inhalt springen |
| Rail | Server (nav label), Direktnachrichten, Server hinzufügen |
| Rail tooltip | `<n> Erwähnung`, `<n> Erwähnungen`, Mehr als 99 Erwähnungen, Ungelesene Nachrichten, Stummgeschaltet |
| Unread a11y | `<n> Benachrichtigungen`, Mehr als 99 Benachrichtigungen, Ungelesene Nachrichten |
| Sidebar header | `Menü für <Server>`, Stummgeschaltet, (stummgeschaltet), Einladen |
| Server menu | Einladen, Benachrichtigungen…, Kanal erstellen, Kategorie erstellen, Servereinstellungen, Mitglieder |
| Sidebar | Kanäle, Übersicht, Sprachkanäle, Leer, Noch keine Kanäle., Entwurf, Live, Jemand, Dein Profil, Profil und Lautstärke, Lokal stummgeschaltet, stumm, taub, `<name> · verbunden` |
| Row actions | Kanal in dieser Kategorie, Kategorie umbenennen, Kategorie löschen, Kanal bearbeiten, Kanal löschen, `Aktionen für <x>` |
| Delete confirms | `Kanal „<name>“ löschen?` / Alle Nachrichten darin werden gelöscht. / Kanal löschen; `Kategorie „<name>“ löschen?` / Die Kanäle darin bleiben erhalten. / Kategorie löschen |
| Home sidebar | Gelabber; Keine Server. Erstelle einen über + in der Serverleiste oder öffne einen Einladungslink. |
| Placeholder | Navigation wird geladen, Erneut versuchen, Wird geladen…, Die Kanäle konnten nicht geladen werden., Die Unterhaltungen konnten nicht geladen werden. |
| DM sidebar | Nachrichten, Neue Nachricht, Noch keine Unterhaltungen., Unterhaltungen |
| Header | Suchen, In dieser Unterhaltung suchen, Wörter oder eine Phrase in Anführungszeichen, Strg F, Benachrichtigungen, Stummgeschaltet, `Benachrichtigungen für <x>`, `… (stumm)`, Mitgliederliste, Mitgliederliste einblenden, Mitgliederliste ausblenden, Live, Beitreten, Zum Kanal |
| Member panel | Mitglieder, Online, Abwesend, Offline, ` — ` (em dash between label and count), ` · Du`, `Profil von <Name>`, Live, `Live in <channel>`, einem Sprachkanal, Im Sprachkanal, `In <ch>`, `Stumm in <ch>`, `Taub in <ch>`, Stumm, Taub |
| User panel | Online, Abwesend, Offline, `<presence> · <status>`, Einstellungen, Benutzermenü, Benutzerkonto, Profil bearbeiten, Server wechseln …, Abmelden, Dein Status, Status, Speichern, Speichert…, Gespeichert., Nicht gespeichert – erneut versuchen |

---------------------------------------------------------------------------------

## 13. Gaps / not covered here

- Message list, composer, typing bar, DM header (`dm-peer-button`), server overview
  page, settings pages, dialogs, and the full call bar internals: other area specs.
- Scrollbars: CSS asks for `scrollbar-width: thin` with the platform colours
  (dark scheme in dark themes); no custom colours exist to port.
- Pixel positions marked "x 9.5" are Chrome's fractional layout; the port may round.
