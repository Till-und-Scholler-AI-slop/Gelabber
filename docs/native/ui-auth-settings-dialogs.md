# Spec: auth, settings, overview, DMs, dialogs, overlays (egui port)

Area: `ui-auth-settings-dialogs`. Source of truth: `web/src` on branch `claude/native-foundation`.
All paths below are relative to `web/src/`. `file:N` = line N, `file:N-M` = range.

Reading rules for implementers

- **px are CSS px at Textgröße "Kompakt" (zoom 1, the default).** The whole page is `zoom: var(--lr-ui-scale)` (`index.css:193-195`): Kompakt 1, Standard 1.125, Groß 1.25 (`interface/preferences.ts:6-10`). In egui: set `pixels_per_point` multiplier = scale; never scale individual widgets.
- **Box model:** Tailwind preflight → every element `box-sizing: border-box`; headings/paragraphs/lists have margin 0 and inherit font size/weight; `strong`/`b` = weight 700; `html` font-size 16px, `line-height: 1.5` default; lists unstyled.
- **Font:** `"Gelabber Inter"` = Inter 4.1 variable (files `fonts/InterVariable-latin.woff2`, `-latin-ext`, `-Italic-latin`; `index.css:46-78`), antialiased, `font-synthesis: none`. Theme style `terminal` swaps to `ui-monospace, "Cascadia Code", monospace` (`theme/model.ts:370-373`). Default presets are style `clear` → Inter.
- **Line height** given as CSS multiplier; px line box = size × multiplier (e.g. 13px/1.35 = 17.55px).
- Viewport facts used for computed values: rail 64px + sidebar 224px = 288px navigation (`index.css` `:root --lr-rail-width/--lr-sidebar-width`); member list 248px (shown inline only on server overview/channel pages, viewport > 1130px). At **1280×800** main column = 992px (no member list) / 744px (with member list). At **1600×900** main = 1312px / 1064px.
- Breakpoints that matter in this area (all `max-width` unless noted): 1180 (themes layout), 1130 (member list becomes drawer), 800 (navigation drawer, phone), 640/639 (modal → bottom sheet, settings padding), 600 (welcome), 480 (auth card padding, invite panel), 420; `pointer: coarse` (44px targets). **Both 1280 and 1600 are above every one of them**, so the desktop layout applies everywhere; phone variants are noted only briefly.

---

## 0. Theme tokens (resolved)

Source: `theme/model.ts:187-391` (`derivedColors`, `themeVariables`), presets `theme/presets.ts:5-21` + `theme/defaults.json`, `fixContrast` applied (`presets.ts:227`, no change for these two). Values below were computed by running `themeVariables(fixContrast(preset))` — they are exact.

Default theme preference is **"dark"** when nothing stored (`theme/theme.ts:11-27`). Built-in ids: `system`, `dark` ("Gelabber Dunkel"), `light` ("Gelabber Hell"), plus 12 palettes (catppuccin … ristretto, `presets.ts:22-225`).

### 0.1 Colour roles

| CSS var | Role | Dark (`Gelabber Dunkel`) | Light (`Gelabber Hell`) |
|---|---|---|---|
| `--lr-bg` | page background, input background | `#17191a` | `#f8f5f0` |
| `--lr-panel` | sidebars, cards | `#1d2021` | `#f4f0e9` |
| `--lr-surface` | secondary button fill, list fills | `#272b2d` | `#ece7df` |
| `--lr-rail` | server rail edge colour | `#131617` | `#eee9e1` |
| `--lr-elevated` | modals, popovers, toasts | `#252829` | `#fcfbf9` |
| `--lr-text` | primary text | `#eef0ee` | `#252829` |
| `--lr-muted` | secondary text | `#a4abb1` | `#5c646b` |
| `--lr-subtle` | tertiary text, disabled text, placeholders | `#8a8f93` | `#6a6e71` |
| `--lr-border` | hairlines | `#2c3032` | `#dcd6cc` |
| `--lr-border-strong` | input/control borders | `#717475` | `#86837e` |
| `--lr-accent` | fills (primary button, switch on, markers) | `#f3b752` | `#e8a638` |
| `--lr-accent-ink` | text on accent | `#111111` | `#111111` |
| `--lr-accent-text` | accent-coloured text/icons/links | `#f3b752` | `#836026` |
| `--lr-focus` | focus ring / focused border | `#f3b752` | `#836026` |
| `--lr-hover` | hover fill | `#2e3131` | `#e3e0da` |
| `--lr-selected` | selected fill | `#443b2a` | `#f2e3c9` |
| `--lr-online` | online dot, speaking | `#85d69b` | `#287a45` |
| `--lr-online-text` | success text | `#85d69b` | `#277643` |
| `--lr-warning` | away dot, caps-lock hint | `#d8a657` | `#a27e44` |
| `--lr-danger` | error text, danger outline | `#f39b98` | `#a12b29` |
| `--lr-danger-bg` | danger fill, count badge | `#8f302d` | `#b33b38` |
| `--lr-on-danger` | text on danger fill | `#ffffff` | `#ffffff` |
| `--lr-backdrop` | modal backdrop | `rgb(9 12 13 / .72)` | `rgb(37 40 41 / .45)` |
| `--lr-tile-1…8` | server/avatar initials tints | `#634336 #635636 #4e6336 #366349 #365d63 #364963 #433663 #633653` | `#ead2c8 #eae0c8 #daeac8 #c8ead6 #c8e5ea #c8d6ea #d2c8ea #eac8dd` |

Derived mixes used below (computed, sRGB `color-mix`, alpha composited on the stated surface):

| Use | Dark | Light |
|---|---|---|
| Primary button hover (`filter: brightness(1.07)` on accent) | `#ffc458` | `#f8b23c` |
| Danger button hover (`brightness(1.1)`) | `#9d3532` | `#c5413e` |
| `.lr-primary` hover (`brightness(1.08)`) | `#ffc659` | `#fbb33c` |
| Form error / load error fill: danger-bg 18% on panel | `#322323` | `#e8cfc9` |
| … same on page bg (welcome error) | `#2d1d1d` | `#ecd4cf` |
| … same on elevated (inside modal) | `#38292a` | `#efd8d6` |
| Danger-outline hover: danger-bg 14% on bg / on elevated | `#281c1d` / `#34292a` | `#eedbd6` / `#f2e0de` |
| Invite banner: accent 14% + panel 86% | `#3b3528` | `#f2e6d0` |
| Accent badge: accent 16% on bg / panel / elevated | `#3a3223` / `#3f3829` / `#463f30` | `#f5e8d3` / `#f2e4cd` / `#f9edda` |
| Profile-card default banner: accent 65% + panel 35% | `#a88241` | `#ecc076` |
| Range track (text 18%) on bg / panel / elevated | `#3e4040` / `#434546` / `#494c4c` | `#d2d0cc` / `#cfccc6` / `#d5d5d4` |
| "Verbunden" pill: online 14% on bg | `#26332c` | `#dbe4d8` |
| Source-audio warning text: warning 70% + text 30% | `#dfbc84` | `#7d643c` |
| Backdrop composited over page bg (for SSIM reference) | `#0d1011` | `#999996` |

### 0.2 Radii, shadows (`theme/model.ts:290-294, 374-389`)

Style `clear` (both defaults): `xs 3px · sm 5px · md 7px · lg 10px · xl 14px`; `--lr-radius` = 7px, `--lr-radius-small` = 5px, `--lr-radius-full` 999px. (Style `soft`: 6/8/12/16/20; `terminal`: 1/2/2/2/3.)

| Shadow | Dark | Light |
|---|---|---|
| `--lr-shadow-1` (popover, menus) | `0 8px 24px rgba(0,0,0,.32)` | `0 8px 24px rgba(60,45,20,.12)` |
| `--lr-shadow-2` (modal, toast, sticky bars) | `0 18px 48px rgba(0,0,0,.5)` | `0 18px 48px rgba(60,45,20,.18)` |
| `--lr-shadow-card` | none (soft style: `0 6px 22px #00000012`) | none |

### 0.3 Type scale (`index.css:96-105`) — `weight size/line-height`

| Token | Value |
|---|---|
| `--lr-type-caps` | 600 11px/1.3 (always with `letter-spacing: .06em; text-transform: uppercase` where used) |
| `--lr-type-meta` | 400 12px/1.4 |
| `--lr-type-ui` | 500 13px/1.35 |
| `--lr-type-body` | 400 14px/1.5 |
| `--lr-type-title-sm` | 600 16px/1.3 |
| `--lr-type-title` | 600 20px/1.25 |
| `--lr-type-page` | 700 28px/1.15 |
| `--lr-type-hero` | 700 `clamp(28px, 3vw, 36px)`/1.1 → **36px at 1280 and 1600** (3vw = 38.4/48 → clamped). Becomes <36 only below 1200px viewport. |

Spacing grid (`index.css:110-120`): 2,4,6,8,12,16,20,24,32,40. Control heights: xs 28, sm 32, md 36 (all 44 under `pointer: coarse`). Header line: 52px (`--lr-header-height`). Motion: micro 100ms, fast 120ms, standard 180ms, large 240ms, exit 130ms, large-exit 170ms; ease `cubic-bezier(.2,.8,.2,1)`, standard `cubic-bezier(.2,0,0,1)`, exit `cubic-bezier(.4,0,1,1)`; press scale .97 (all 0/1 with reduced motion).

### 0.4 Global focus ring (`index.css:235-240`)

Any focused button/link/input/select/summary (keyboard focus; text inputs always): `outline 2px solid --lr-focus`, `outline-offset 2px`, plus `box-shadow 0 0 0 2px --lr-bg` (a page-coloured 2px gap). Exceptions noted per component (`outline-offset:-2px` on list rows/tabs/disclosure summaries; `h1/h2` headings that receive programmatic focus have no outline).

### 0.5 Icons (`components/Icons.tsx:5-19`)

All icons: 24×24 viewBox drawn at `size` px (default 16), `fill none`, `stroke currentColor`, `stroke-width 2`, round caps/joins. Names used in this area: `PlusIcon, LinkIcon, HashIcon, SpeakerIcon, ChatIcon, PencilIcon, CloseIcon, ChevronIcon` (points **down**; rotated where noted), `CheckIcon, WarningIcon, EyeIcon, EyeOffIcon, CopyIcon, ShareIcon, MoreIcon, ReplyIcon, TrashIcon, MicIcon, BellIcon, BellOffIcon, ScreenIcon, DownloadIcon`. Path data: copy from `Icons.tsx` verbatim.

---

## 1. Primitives (used by every screen below)

### 1.1 Button (`components/ui/Button.tsx:39-88`, `components/SubmitButton.tsx:21-37`, CSS `components/ui/ui.css:12-98`)

Layout: inline-flex, centred, `gap 8px`, `border 1px solid transparent`, radius 5px, font `--lr-type-ui` (500 13px/1.35), centred text, optional leading icon (no shrink).

| Size | min-height | padding-x | font |
|---|---|---|---|
| `sm` | 32 | 12 | 13px |
| `md` (default) | 36 | 16 | 13px |
| `lg` | 44 | 20 | **14px** |

| Variant | Rest | Hover (not disabled) |
|---|---|---|
| `primary` | bg accent, text accent-ink, **weight 600** | `brightness(1.07)` → see 0.1 |
| `secondary` (default) | border border-strong, bg surface, text | bg hover |
| `ghost` | transparent, text | bg hover |
| `danger` | bg danger-bg, text `#fff`, weight 600 | brightness(1.1) |
| `danger-outline` | border danger, transparent, text danger | bg danger-bg 14% |

Disabled (and **not** pending): border `--lr-border`, bg surface, text subtle, cursor not-allowed — overrides every variant (`ui.css:90-96`). Pending: label swapped to `pendingLabel`, disabled, but **keeps its variant look** (`aria-busy`). Press: scale .97 for 100ms. `GhostButton` = ghost (or danger-outline with `tone="danger"`) (`Button.tsx:91-103`). `SubmitButton` = primary submit; while a rate-limit cooldown runs it is disabled (disabled look) and its label is "Erneut versuchen in {n} s". Default variant of `Button`/`buttonClass` is **secondary** (`Button.tsx:40`).

Height note: a md button with one 13px line = 17.55 + 2 border → 36 (min-height wins); lg = 44.

### 1.2 IconButton (`ui/IconButton.tsx:21-51`, `ui.css:101-142`)

Square, transparent, `border 1px transparent`, radius 5, colour muted; hover bg hover + colour text. Sizes xs 28, sm 32, md 36. `variant="secondary"`: border border-strong, bg surface, colour text. `aria-pressed=true`: bg accent, colour accent-ink. Disabled: colour subtle. Tooltip = label (title attr).

### 1.3 Text field (`ui/Input.tsx`, `ui.css:175-233`) and FormField (`ui/FormField.tsx:11-43`)

FormField = column, `gap 6px`: label (`--lr-type-ui`, colour text) → control → one line below: **error** (12px/1.4 colour danger) *or* **hint** (12px/1.4 muted), never both.

Input `.ui-field`: width 100%, min-height 36, padding `7px 11px`, border 1px border-strong, radius 5, bg **`--lr-bg`** (also inside cards/modals), text colour, font 400 14px/1.5 (→ rendered height 37px: 21 line + 14 pad + 2 border). Placeholder colour subtle. Focus: border `--lr-focus` (+ focus ring §0.4). Invalid: border danger. Disabled/readonly: bg surface; disabled text subtle. Textarea: min-height 88, vertical resize. Select (`ui/Select.tsx:13-21`): same field, `padding-right 34px`, `ChevronIcon 14` absolutely at right 11px, vertically centred, colour muted.

### 1.4 PasswordField (`ui/PasswordField.tsx:22-81`, `ui/passwordToggle.ts:2-8`, `ui.css:252-280`)

Input with `padding-right 48px`; toggle IconButton absolutely at right 0, centred, 44×44 but `max-height 100%` (→ 44×37), icon `EyeIcon 18` (hidden) / `EyeOffIcon 18` (shown); accessible name always "Passwort anzeigen", tooltip `title` swaps **"Passwort anzeigen"** (hidden) / **"Passwort verbergen"** (shown), `aria-pressed` = shown; hover/pressed: transparent bg, colour text (not accent fill). Caps-lock line directly below the field (above the FormField error/hint line, so the column gap 6 applies) while CapsLock is on: inline-flex gap 6, `WarningIcon 14` + **"Feststelltaste ist an"**, 12px/1.4 colour warning.

### 1.5 FormError (`components/FormError.tsx:7-14`, `ui.css:518-535`)

Renders nothing when there is no message (no gap is taken in the flex column). Block: padding `10px 12px`, `border-left 3px solid danger`, radius 5, bg danger-bg 18% (see 0.1), colour danger, font body 14px/1.5. `LoadError` (`components/LoadError.tsx:3-24`) = same box as column, gap 12, message text colour **text**, then secondary sm button "Erneut versuchen" (pending "Wird geladen…").

### 1.6 InlineStatus (`ui/InlineStatus.tsx`, `ui.css:501-517`)

inline-flex, gap 6, min-height 20 (0 when empty), 12px/1.4 muted. success: colour online-text + `CheckIcon 14`; error: colour danger + `WarningIcon 14`; neutral: muted, no icon. Success and neutral lines clear themselves after **4000 ms**; errors stay until replaced (`ui/useInlineStatus.ts:8-25`).

### 1.7 Switch (`ui/Switch.tsx:13-41`, `ui.css:283-322`)

Track 38×22, border 1 border-strong, radius full, bg surface; knob 14×14 circle at (3,3) inside the border, colour muted. On: track bg+border accent, knob accent-ink, knob moved +16px. Transition 180ms. Disabled opacity .5. With label: `.ui-choice-row--between` row: flex, centre, gap 12, min-height 36, padding-y 4; text column (`.ui-choice-text`: 500 13px/1.35, gap 2) + `<small>` description 12px/1.4 muted; switch on the right.

### 1.8 Checkbox / radio `.ui-check` (`ui.css:323-364`)

20×20, border 1 border-strong, radius 3 (radio: circle), bg page bg. Checked: bg+border accent; check mark = 12×12 polygon `14% 52%, 0 66%, 38% 100%, 100% 22%, 86% 8%, 38% 72%` in accent-ink (radio: 8px dot). Disabled opacity .5.

### 1.9 RadioCard (`ui/RadioCard.tsx:14-27`, `ui.css:391-418`)

Label row: flex, align start, gap 12, min-height 44, padding 12, border 1 border-strong, radius 7, bg panel. Radio (`.ui-check`, margin-top 1) + text column (label 500 13px, description 12px muted). Hover bg hover. Checked: border accent, bg selected. Disabled opacity .6.

### 1.10 Chip / ChipGroup (`ui/ChipGroup.tsx:16-67`, `ui.css:463-498`)

Group: flex wrap, gap 8. Chip: min-height 32, padding 0 12, border 1 border-strong, radius full, transparent, 500 13px; hover bg hover; selected (`aria-checked`): border accent, bg selected, weight 600; disabled colour subtle. Single tab stop, arrow keys move+select.

### 1.11 Badge (`ui/Badge.tsx`, `ui.css:421-462`)

Tag: inline-flex, gap 4, min-height 20, padding 0 6, radius 3, caps 600 11px/1.3, letter-spacing .06em, uppercase, nowrap. Tones: neutral bg surface/text muted; accent bg accent 16% / text accent-text; success online 14% / online-text; warning warning 16% / text; danger danger-bg 18% / danger. `count` (unread pill): min 18×18, padding 0 5, radius full, bg danger-bg, text `#fff`, 700 11px/1, no caps; shows `99+` above 99.

### 1.12 CopyButton (`ui/CopyButton.tsx:13-74`, `ui.css:146-173`)

A Button whose content is `[CopyIcon|CheckIcon](16) + label|"Kopiert"`; both states occupy the same grid cell (width = wider of the two), crossfade 120ms (icons also scale .6↔1). Copied state lasts **1500 ms** (`ui/useCopied.ts:4`); copied icon colour online-text (inherit on primary).

### 1.13 Avatar (`components/Avatar.tsx:21-55`, `index.css:1234-1307`)

Circle; image `object-fit: cover`, else initials (first letter of first two words, uppercased, `?` if blank; `components/initials.ts`) in 600 weight on tint `--lr-tile-N` where N = FNV-1a hash of the name (`initials.ts:43-52`, port exactly), text colour text.

| size | px | initials font |
|---|---|---|
| sm (default) | 32 | 12px |
| md | 36 | 14px |
| lg | 64 | 20px |
| hero/room | 112 | 32px |

PresenceAvatar dot (`index.css:1313-1332`): bottom-right, 23% of avatar (min 10px), 2px border in `--lr-bg` (inside a profile card `--lr-bg` = panel), colours: online `o` online, away `i` warning, offline `x` muted. Decorations (`index.css:1335-1408`): ring = `0 0 0 2px bg, 0 0 0 4px accent`; glow = `0 0 0 2px accent, 0 0 10px 3px accent@55%`; rainbow = conic ring 3px outside (`#ff5f6d,#ffc371,#f9f871,#5ee7a1,#4fb3ff,#a07cff,#ff5f6d`) + 1.5px bg gap; stars = 1.5px bg gap + 3.5px accent ring and two "✦" glyphs (top −14%/right −10% at max(10, 0.36×size); bottom −6%/left −12% at max(8, 0.24×size)). "accent" = the person's `accent_color` else theme accent.

### 1.14 Range slider `.lr-range` (`index.css:2071-2122`) and Slider row (`voice/settingsLayout.tsx:94-147`, `voice/quality.css:160-184`)

Track 4px high radius 2: filled part accent up to value %, rest text 18% (0.1). Thumb 14px circle accent with `0 0 0 3px accent@22%` halo. Input height 20. Disabled opacity .45. Slider row: grid `1fr auto`, gap 4/12, min-height 36: range | value text (`min-width 5ch`, 500 13px muted, tabular, right-aligned) ; optional scale row (three labels spaced between, 12px subtle, tabular) ; optional hint (12px muted) spanning.

### 1.15 Skeletons (`ui/Skeleton.tsx`, `ui.css:537-633`)

Invisible for 150ms then fade in 180ms. Block: radius 5, bg surface with moving 6% text highlight (1.4s linear). Rows: gap 16; line 10px tall, widths cycle 38/86/62/74/48/92%; avatar rows 32px circle + two lines (22% and next width). Card: column gap 12, padding 20, border, radius 10, bg panel, lines 45%×14, 90%, 70%. `MainSkeleton overview`: padding 24, gap 24, one block `min(420px,70%)`×32 then grid of 2 cards (`auto-fill minmax(240px,1fr)`, gap 16).

---

## 2. Overlays

### 2.1 Modal (`components/Modal.tsx:50-167`, `ui.css:737-870`)

Native modal dialog, centred (`margin:auto`).

| Property | Value |
|---|---|
| width | `calc(100% - 32px)`, max **448px** (default) / **512px** (`wide`) / **640px** (`large`) |
| max-height | `100dvh − 32px` → **768px @800**, **868px @900** |
| border | 1px `--lr-border`, radius **14px** (xl) |
| background | `--lr-elevated` (dark `#252829`, light `#fcfbf9`) |
| shadow | `--lr-shadow-2` |
| backdrop | `--lr-backdrop` |
| header | padding `20px 64px 12px 24px`; `<h2>` title 600 16px/1.3, wraps anywhere |
| close button | IconButton md (36×36), `CloseIcon 18`, label "Schließen", absolutely `top 12px; right 12px` |
| body | column, gap **20px**, padding `8px 24px 24px`, scrolls (y), font 400 14px/1.5 |
| footer (optional) | flex wrap, justify end, gap 8, padding `12px 24px 16px`, border-top 1px border |
| enter | opacity 0→1, scale .97→1, 240ms ease-enter; backdrop fades |
| exit | opacity →0, scale .97, 170ms ease-exit |

So the content width of a default modal = 448 − 2 − 48 = **398px**; wide 462; large 590.
Behaviour: Esc / "Schließen" close; backdrop click closes unless `dirty`; when `dirty`, Esc/close asks **"Änderungen verwerfen?"** / "Deine Eingaben gehen verloren." / confirm **"Verwerfen"** (`components/modalClose.ts:25-29`). Focus starts on the first `autoFocus` field, else first focusable; returns to opener.
Under 640px: bottom sheet (full width, radius 14 14 0 0, max-height 90dvh, header padding-x 16/60, close at 8/8, footer buttons flex 1).

### 2.2 Confirm dialog (`components/ConfirmHost.tsx:13-73`, `components/confirm.ts`)

A Modal (448px) with title = request title, body = optional message paragraph (14px/1.5, colour **muted**; without a message the body still renders empty → 32px tall: padding 8 + 24), footer = ghost **"Abbrechen"** + confirm button. Confirm button: `danger` variant (default tone) or `primary` (tone `"neutral"`), label = `confirmLabel`. Initial focus: "Abbrechen" when danger, confirm when neutral. One question at a time (a new one cancels the old). Enter submits.

All confirm texts in this area:

| Where | Title | Message | Button | Tone |
|---|---|---|---|---|
| Dirty modal (`modalClose.ts:25`) | Änderungen verwerfen? | Deine Eingaben gehen verloren. | Verwerfen | danger |
| Profilkarte leave (`pages/ProfileCardForm.tsx:72-81`) | Ungespeicherte Änderungen verwerfen? | Deine Änderungen an der Profilkarte gehen verloren. | Verwerfen | danger |
| Theme draft (`theme/discardDraft.ts:5-7`) | Ungespeicherten Theme-Entwurf verwerfen? | Deine Änderungen an diesem Theme gehen verloren. | Verwerfen | danger |
| Theme pending (`theme/ThemesPanel.tsx:287-292`) | Lokale Änderung verwerfen? | Gelabber lädt dann wieder den Theme-Stand deines Accounts. | Account-Stand laden | danger |
| Theme delete (`ThemesPanel.tsx:548-551`) | Theme „{name}“ löschen? | — | Löschen | danger |
| Logout all (`pages/ProfilePage.tsx:331-336`) | Auf allen Geräten abmelden? | Auch dieses Gerät. Danach musst du dich überall neu anmelden. | Überall abmelden | danger |
| Revoke invite (`pages/ServerSettingsPage.tsx:443-450`) | Link widerrufen? | Der Link funktioniert danach nicht mehr. Bestehende Mitglieder bleiben. | Widerrufen | danger |
| Unban (`ServerSettingsPage.tsx:556-565`) | {name} wieder zulassen? | Ein Beitritt per Einladungslink ist dann wieder möglich. | Entsperren | **neutral** |
| Delete server (`ServerSettingsPage.tsx:610-616`) | Server „{name}“ endgültig löschen? | Kanäle, Nachrichten, Einladungen und Mitgliedschaften werden gelöscht. | Server löschen | danger |
| Leave server (`ServerSettingsPage.tsx:620-624`) | Server „{name}“ verlassen? | — | Verlassen | danger |
| Kick (`components/MemberActions.tsx:38-43`) | {name} vom Server entfernen? | Mit einem neuen Einladungslink ist ein erneuter Beitritt möglich. | Entfernen | danger |
| Ban (`MemberActions.tsx:49-54`) | {name} sperren? | Sie können nicht wieder beitreten, auch nicht mit einem Einladungslink. | Sperren | danger |

German quotes are „…“ (U+201E/U+201C).

### 2.3 Toasts (`components/Toasts.tsx`, `components/toasts.ts`, `ui.css:872-961, 1050-1080`)

Stack: fixed, `right 16px`, column, align end, gap 8, width `min(380px, 100vw − 32px)`; info toasts above error toasts (two regions). Vertical position:
- outside the workspace (auth pages): `bottom 16px`;
- in the workspace: anchored **12px above the composer's top edge**, right edge 16px inside the composer's right edge (CSS anchor positioning). On pages without a composer (settings, overview) the anchor fallback gives **bottom 12px, right 16px**. (Engines without anchor support use `bottom 74px`.)
- with an active call dock, the floor rises by the dock height (`--lr-media-space`, 64px default).

Card: flex, centre, gap 12, padding `12px 8px 12px 12px`, border 1 border, radius 10, bg elevated, shadow-2, font 14px/1.5. Error: `border-left 3px danger` + leading `WarningIcon 16` in danger. Message flex 1, wraps anywhere. Optional action: ghost sm button, text accent-text 600, margin-y −6. Close: IconButton sm 32 `CloseIcon 16` "Schließen", margin-y −6.
Timing: info 4000ms, error ≥10000ms; hover/focus pauses; resume ≥1500ms left. Duplicate (same tone+text, no action) suppressed; max 3 errors. Enter: opacity 0 + translateY 8px → 0 over 180ms; exit opacity→0 scale .98 130ms.
Toast texts in this area: "Passwort geändert. Andere Geräte sind abgemeldet." (`pages/ResetPasswordPage.tsx:29`), "E-Mail-Adresse bestätigt." (`pages/VerifyEmailPage.tsx:35`), "Server nicht gefunden oder kein Zugriff." (`pages/WorkspaceLayout.tsx:230`). Error toasts use `notifyError` (`toasts.ts:153-160`): optional prefix + `errorMessage(code)` (§3.9).

### 2.3a New-message toasts (top right) (`components/MessageToasts.tsx:12-91`, `messages/toasts.ts`, `messages/notify.ts:136-170`, `ui.css:915-1046, 1074-1079`)

Missing from the shell/chat specs; shown for chat messages in a channel/DM you are not viewing, when Benachrichtigungen › "Popup bei neuen Nachrichten in anderen Chats" is on (default). Separate stack from §2.3.

| Property | Value |
|---|---|
| stack | fixed, **top 64px, right 16px**, column gap 8, width `min(100vw − 32px, 22rem)` → **352px** at 1280 and 1600; newest **last** (bottom); max **4** (oldest dropped) |
| card | relative, flex, align start, gap 12, padding `12px 40px 12px 12px`, border 1 border, radius 10, bg elevated, shadow-2 |
| card hover | bg `color-mix(elevated 92%, text)` → dark **`#353839`**, light **`#ebeae8`** |
| avatar | Avatar sm **32** (initials 12px on tint) of the author |
| text column | the whole card is one button (column, gap 2, left-aligned, no bg/border); each line single-line ellipsis |
| line 1 "where" | 400 12px/1.4 muted: channel label `#{channel}` (DM: peer name; fallbacks "Kanal"/"DM"); when stacked: `{label} · {n} neu` |
| line 2 "who" | 600 13px/1.35 text: author name; mention adds `<small>` " hat dich erwähnt" (400, muted, same size) |
| line 3 preview | 400 14px/1.5 muted: whitespace-collapsed text, >140 chars → first 137 + "…"; empty → "GIF" / "Datei" (attachment) / "Neue Nachricht"; mentions shown as names; system messages as their sentence |
| close | IconButton **xs 28×28**, `CloseIcon 14`, "Schließen", absolute top 6 / right 6, above the card button |
| timing | hold **6000 ms**; a message in the same channel within **1600 ms** bumps the count and restarts the timer; a new burst in a channel replaces its old card; hover/focus pauses, resume with ≥1500 ms left |
| motion | enter opacity 0 + translateY **−8px** → 0, 180ms ease-enter; exit opacity 0, scale .98, 130ms ease-exit |
| click | dismisses and navigates to `/d/{id}` or `/s/{server}/c/{channel}` |

### 2.4 Popover (anchored panel) (`components/ui/Popover.tsx`, `ui/popoverPosition.ts`, `ui.css:636-661`)

Fixed panel: min-width 180, max-width `min(360px, 100vw − 16px)`, max-height `100dvh − 16px`, padding 8, border 1 border, radius 10, bg elevated, shadow-1, scrolls. Position: below the anchor with **6px gap** (above if it doesn't fit), `bottom-start` aligns left edges, `bottom-end` aligns right edges; clamped 8px inside viewport. Grows from the anchor point: opacity 0/scale .96 → 1 in 180ms; exit 130ms. Esc, outside press, tabbing out close it; focus starts on first focusable inside. (Touch/≤639px: bottom sheet with grip 36×4 and title — not needed on desktop.)

### 2.5 Image lightbox (`components/ImageLightbox.tsx:22-87`, `components/chat.css:1081-1156`)

Full-viewport modal dialog, transparent, padding `56px 16px 16px`, image centred: `max-width/height 100%`, `object-fit: contain`, radius 7. Backdrop `rgba(0,0,0,.85)`. Top bar fixed across the top: flex, centre, gap 16, padding `10px 16px`, 14px, colour `#fff`: filename (flex 1, ellipsis, title tooltip) → link **"Herunterladen"** (underlined, offset 2; after click shows "Download gestartet") only when downloadable → close button 40×40 circle, bg `rgba(255,255,255,.12)`, `CloseIcon 18`, aria "Schließen" (initial focus). Esc / click outside the image closes. Enter: opacity 0→1 240ms, image scale .97→1.

---

## 3. Front door (auth) screens

### 3.1 Frame (`pages/AuthLayout.tsx:18-30`, `pages/auth.css:2-61`)

| Element | Spec |
|---|---|
| page | full window, bg `--lr-bg`, grid centred both axes, min-height 100vh, padding `32px 16px` |
| artwork (viewport ≥ 800px, i.e. always on desktop) | fixed full-window layer behind content: `public/images/living-room/community-banner.png` `cover` centred at **opacity .18**, then over it a radial gradient `ellipse at center: transparent 30% → --lr-bg 95%` |
| column | flex column, **gap 24px**, width `min(100%, 384px)` |
| brand (top, centred) | inline-flex, centre, gap 10: `public/icons/icon-192.png` drawn **40×40, radius 7** + text **"Gelabber"** 700 20px/1.2, letter-spacing −0.02em, colour text; links to `/` |
| card `.lr-auth-card` | flex column, padding **32px**, border 1 border, radius **10px**, bg panel, colour text → inner width **318px** |
| card `h1` | 700 28px/1.15, letter-spacing −0.03em, wraps anywhere; programmatically focused h1 has no outline |
| `.auth-lead` | margin-top 4, 400 14px/1.5, muted |
| `.auth-form` | column, **gap 20**, margin-top 24 |
| `.auth-result` | column, gap 12, margin-top 20, 14px/1.5 (`.auth-quiet` muted) |
| `.auth-actions` | column, gap 8, margin-top 24 (buttons stretch to 318px; links with `self-start` stay left) |
| `.auth-footer` | margin-top 24, 14px/1.5 |
| `.auth-link` | accent-text, weight 500, underline offset 2px; disabled subtle, no underline |
| `.auth-recovery` | min-height 20, muted, 400 13px/1.4 (line 18.2 → box 20) |
| `.auth-banner` | margin-bottom 16, padding `10px 12px`, border-left 3px border-strong, radius 5, bg surface, 500 13px/1.35 text |
| `.auth-banner--invite` | border-left accent, bg accent 14% over panel (`#3b3528` / `#f2e6d0`) |

Vertical position (login, no banners, mail state known): card = 2 border + 64 padding + h1 32.2 + lead (4 + 21) + form margin 24 + form [email field 60.55 (label 17.55 + gap 6 + input 37) + 20 + password 60.55 + 20 + button 44 + 20 + recovery 20 = 245.1] = **392.3px**; column = brand 40 + gap 24 + 392.3 = **456.3px**. Centred in the 100vh grid → column top **≈172px @800** (card top ≈236), **≈222px @900** (card top ≈286). Horizontal: column centred (x = 448 @1280, 608 @1600).

Banner texts: invite redirect → **"Einladung erhalten. Melde dich an oder registriere dich, um beizutreten."** (`AuthLayout.tsx:33-39`); `?reason=expired` → **"Sitzung abgelaufen. Melde dich erneut an."** (`pages/LoginPage.tsx:71-75`).

### 3.2 Login `/login` (`pages/LoginPage.tsx:20-153`)

Order inside card: [invite banner] [expired banner] **h1 "Anmelden"** → lead → form → [install disclosure: web only, skip in desktop].
Lead: normal: **"Noch kein Konto? "** + link **"Registrieren"**. Invite redirect: lead **"Neu bei Gelabber?"** then secondary **lg** button-link **"Konto anlegen"** (margin-top 12, stretched to card width 318) (`LoginPage.tsx:77-99`).
Form fields:

| # | Field | Label | Type/autocomplete | Notes |
|---|---|---|---|---|
| 1 | email | **E-Mail-Adresse** | email, autofocus | |
| 2 | password | **Passwort** | PasswordField, current-password | |
| 3 | FormError | | | only when set |
| 4 | submit | **Anmelden** (pending **"Anmelden…"**) | primary lg, full width | cooldown label **"Erneut versuchen in {n} s"** |
| 5 | recovery line | link **"Passwort vergessen?"** (passes typed email) if server mail is on; text **"Passwort vergessen? Frag die Person, die diesen Gelabber-Server betreibt."** if off; empty (20px) while unknown | | |

Client validation: email via `validateEmail`, password non-empty → "Passwort fehlt.".

### 3.3 Register `/register` (`pages/RegisterPage.tsx:32-162`)

[invite banner] **h1 "Konto anlegen"**, lead **"Schon registriert? "** + link **"Anmelden"**. Fields: **Name** (text, nickname, autofocus, maxLength 64, hint **"Für andere sichtbar."**), **E-Mail-Adresse**, **Passwort** (new-password, hint **"Mindestens 8 Zeichen."**), FormError, submit **"Registrieren"** (pending **"Konto wird angelegt…"**).

"Postfach prüfen" state (`RegisterPage.tsx:168-258`) when the server mailed a link: h1 **"Postfach prüfen"**; result block: "Bestätigungslink an **{email}** gesendet. Er gilt 24 Stunden." · [invite] "Danach geht es weiter zur Einladung." · quiet "Keine Mail erhalten? Prüfe den Spam-Ordner oder sende den Link erneut."; actions: secondary lg **"Link erneut senden"** (disabled with **"Erneut senden ({s} s)"** for a 60s cooldown that starts immediately; pending "Wird gesendet…"), InlineStatus (success **"Neuer Link gesendet."**), ghost lg **"Andere Adresse verwenden"**; footer link **"Zur Anmeldung"**.

### 3.4 Forgot password `/forgot-password` (`pages/ForgotPasswordPage.tsx:18-165`)

h1 **"Passwort vergessen"**, lead **"Du erhältst einen Link, um ein neues Passwort festzulegen."**, field **E-Mail-Adresse** (prefilled from `?email`), submit **"Link schicken"** (pending "Wird gesendet…"), footer link **"Zurück zur Anmeldung"**.
Sent state: h1 **"Postfach prüfen"**, "Falls zu **{email}** ein Konto existiert, wurde ein Link gesendet. Er gilt eine Stunde.", quiet line as 3.3, button **"Erneut senden"** / **"Erneut senden ({s} s)"**, status "Neuer Link gesendet.", ghost **"Andere Adresse verwenden"**, footer **"Zurück zur Anmeldung"**.

### 3.5 Reset password `/reset-password?token=` (`pages/ResetPasswordPage.tsx:17-104`)

h1 **"Neues Passwort"**, lead **"Danach bist du auf allen anderen Geräten abgemeldet."**, PasswordField **"Neues Passwort"** (autofocus, hint "Mindestens 8 Zeichen."), submit **"Passwort speichern"** (pending "Wird gespeichert…"). Dead/empty token → h1 **"Link abgelaufen"**, text "Dieser Link ist abgelaufen oder wurde schon benutzt.", left link **"Neuen Link anfordern"**.

### 3.6 Verify email `/verify-email?token=` (`pages/VerifyEmailPage.tsx:23-112`)

h1 **"E-Mail-Adresse bestätigen"**, lead **"Gib zur Bestätigung dein Passwort ein."**, PasswordField **"Passwort"** (autofocus), submit **"Bestätigen"** (pending "Wird bestätigt…"), recovery line link **"Passwort vergessen?"**. Dead link: h1 "Link abgelaufen", "Dieser Link ist abgelaufen oder wurde schon benutzt.", quiet "Melde dich an, um einen neuen zu erhalten.", link **"Zur Anmeldung"**.

### 3.7 Invite landing `/invite/{code}` (`pages/InvitePage.tsx:34-183`, `auth.css:158-181, 241-245`)

Loading: skeleton card min-height 300, padding 32. Loaded card:

| Element | Spec |
|---|---|
| header block | column, centred, gap 8, text centred |
| tile | 56×56, radius 10, bg `--lr-tile-N` (N = server-id hash, `initials.ts:27-31`), text 600 20px/1, ls .02em; uploaded icon fills tile; fallback = `initials(server.name)` (2 letters), margin-bottom 8 |
| line 1 | **"Einladung zu"** or **"Du bist bereits Mitglied von"** — 500 13px muted |
| h1 | server name (28px/700) |
| line 2 | "1 Mitglied" / "{n} Mitglieder" + (non-member, link with an end) " · läuft {in 6 Tagen / morgen / in 3 Stunden} ab" (`pages/InvitePage.tsx:131-158`); 500 13px muted |
| actions | `.auth-actions` (column gap 8, margin-top 24): member: primary lg link **"Zum Server"**; else a form (column **gap 12**) with FormError + primary lg **"Beitreten"** (pending "Beitreten…"); then ghost lg link **"Abbrechen"** (→ `/`) |

Data (`servers/types.ts:78-89`, `GET /invites/{code}` preview): `{code, server:{id,name,icon_url?,member_count}, expires_at: string|null, member: bool}`; joining is `POST /invites/{code}/join` (→ `Server`) and navigates to `/s/{server.id}` (`servers/api.ts:88-92`). Expiry text (`servers/inviteCode.ts:54-73`): "abgelaufen" when past; else largest unit of day/hour/minute with `RelativeTimeFormat('de',{numeric:'auto'})`, e.g. "läuft in 6 Tagen ab", "läuft morgen ab", "läuft in 2 Minuten ab" (min 1).

Dead invite (not_found/invite_invalid/banned): h1 **"Einladung nicht nutzbar"** (banned: **"Kein Zutritt"**), text "Dieser Einladungslink existiert nicht oder wurde widerrufen." (not_found) else errorMessage, quiet "Frag nach einem neuen Link." (not for banned), secondary lg link **"Zu deinen Servern"**. Other load error: h1 **"Einladung konnte nicht geladen werden"**, lead = errorMessage, primary lg **"Erneut versuchen"** (pending "Wird geladen…"), ghost lg **"Zu deinen Servern"**.

### 3.8 Misc front-door states

- Not found (`pages/NotFoundPage.tsx:6-28`): auth card centred `max-width 384`, h1 **"Seite nicht gefunden"**, lead **"Diese Adresse gibt es in Gelabber nicht."**, footer link **"Zur Startseite"**. Whole page: container max 768, padding 40/24.
- Boot placeholder (`AuthLayout.tsx:49-64`, `auth.css:184-215`): centred grid gap 12: icon 48×48 radius 12, **"Gelabber"** 700 20px/1.2 ls −.02em, **"Verbinde…"** 500 13px muted. Non-first appearance fades in after 300ms (span after 600ms).
- Session check failed (`components/SessionRecoveryNotice.tsx:50-109`): auth frame card, h1 **"Server antwortet nicht"** (offline: **"Keine Verbindung zu Gelabber"**), lead "Gelabber ist nicht erreichbar. Möglicherweise startet der Server neu." / "Dein Gerät ist offline. Sobald es wieder verbunden ist, geht es weiter.", status line (min-height 20, margin-top 12, 13px muted): "Wird geprüft…" or "Neuer Versuch in {s} s", primary lg **"Jetzt erneut versuchen"**.
- Logout failed strip (`SessionRecoveryNotice.tsx:112-138`, `auth.css:223-236`): margin 12/16, padding 12/16, border 1 border + left 3px warning, radius 7, bg elevated, 13px; message + sm **"Erneut abmelden"** (pending "Wird abgemeldet…").

### 3.9 Validation and error copy (`auth/rules.ts:314-422`, `auth/useFormErrors.ts`)

Field limits (`auth/rules.ts:8-15`): password 8–128 chars; name ≤64 (trimmed, no control chars); email ≤254, one `@`, domain with an inner dot; avatar URL ≤2048 `https://…`; bio ≤190; pronouns ≤40; status ≤80; server/category/channel name ≤100 (`servers/rules.ts`).

`fieldMessage(field, code)` — label table: email "E-Mail-Adresse", password "Passwort", current_password "Aktuelles Passwort", name "Name", avatar_url "Avatar-URL", bio "Über mich", pronouns "Pronomen", status_text "Status", accent_color "Akzentfarbe", banner_color "Bannerfarbe", banner_url "Banner", avatar_decoration "Avatar-Rahmen", kind "Kanaltyp", category_id "Kategorie", member_permissions "Rechte", max_uses "Maximale Nutzungen", expires_in_hours "Gültigkeit", content "Nachricht", before/after "Cursor", limit "Anzahl", user_id "Nutzer", filename "Dateiname", content_type "Dateityp", size "Datei", attachment_ids "Anhang", gif "GIF", q "Suche", token "Link", unknown field "Eingabe" (`auth/rules.ts:280-311`).

| code | message |
|---|---|
| required | "{Label} fehlt." |
| too_short | password: "Mindestens 8 Zeichen." else "{Label} ist zu kurz." |
| too_long | "{Label} ist zu lang." (size: "Die Datei ist zu groß (max. 25 MB)."; attachment_ids: "Höchstens 10 Dateien pro Nachricht.") |
| too_large | "Alle Dateien zusammen sind zu groß (max. 100 MB)." |
| mismatch | "Der Inhalt der Datei passt nicht zu ihrem Typ." |
| exclusive / unavailable / not_editable | "Ein GIF wird ohne Text und Anhang gesendet." / "GIFs sind auf diesem Server nicht eingerichtet." / "Ein GIF lässt sich nicht bearbeiten." |
| taken | "Diese E-Mail-Adresse ist schon registriert." |
| wrong | "Das Passwort stimmt nicht." |
| stale | "Dein Bild wurde inzwischen woanders geändert. Lade die Seite neu." |
| invalid | email "Das sieht nicht wie eine E-Mail-Adresse aus."; avatar_url "Bitte eine vollständige https-URL angeben."; colours "Bitte eine Farbe im Format #rrggbb wählen."; bio "Nur Text und Zeilenumbrüche, keine Steuerzeichen."; pronouns/status "Nur eine Zeile Text, keine Steuerzeichen."; content_type "Dieser Dateityp ist nicht erlaubt."; else "{Label} ist ungültig." |

`errorMessage(code)` (form-level): invalid_credentials "E-Mail oder Passwort ist falsch." · email_taken "Diese E-Mail-Adresse ist schon registriert." · email_unverified "Bitte bestätige zuerst deine E-Mail-Adresse. Wir haben dir gerade einen neuen Link geschickt." · token_invalid "Dieser Link ist abgelaufen oder wurde schon benutzt." · unauthenticated "Bitte melde dich erneut an." · csrf_invalid "Sicherheits-Token abgelaufen. Bitte noch einmal versuchen." · insecure_transport "Anmelden geht nur über eine verschlüsselte Verbindung. Öffne Gelabber mit https:// (oder auf diesem Rechner über http://localhost)." · forbidden "Dafür fehlt dir die Berechtigung." · not_found "Das gibt es nicht (mehr)." · invite_invalid "Diese Einladung ist abgelaufen oder aufgebraucht." · banned "Du bist auf diesem Server gesperrt." · rate_limited "Zu viele Anfragen. Bitte kurz warten und noch einmal versuchen." · theme_conflict "Auf einem anderen Gerät geändert. Bitte erneut versuchen." · validation_failed "Bitte die markierten Felder prüfen." · timeout "Der Server antwortet nicht. Bitte noch einmal versuchen." · network "Keine Verbindung zum Server." · bad_request/internal "Da ist etwas schiefgelaufen. Bitte noch einmal versuchen." · quota_exceeded "Tageslimit für Uploads erreicht." · upstream_unavailable "Der GIF-Dienst antwortet gerade nicht. Bitte gleich noch einmal versuchen." · upstream_quota "Dieser Server hat sein GIF-Kontingent gerade aufgebraucht. Bitte später noch einmal versuchen." (`auth/rules.ts:368-412`).

Form behaviour (`auth/useFormErrors.ts:40-90`): ≥2 field errors → FormError "Bitte die markierten Felder prüfen."; one field error → no FormError, focus jumps to that field. Rate limit with Retry-After → FormError **"Zu viele Versuche. Bitte {n} Sekunden warten."** and the submit counts down "Erneut versuchen in {n} s". Typing in a field clears its error and the form error.

---

## 4. Workspace screens (main column only; rail/sidebar/user panel belong to the shell spec)

Shared page header `.lr-channel-header` (`components/chat.css:4-28`): height 52, flex centre, gap 10, padding `0 12px 0 20px`, border-bottom 1 border, bg page bg; `h1` 650 15px (line-height 1.5), ls −0.1px, single line ellipsis; svg muted.

### 4.1 First run: `/` without servers (`pages/WorkspaceIndexPage.tsx:23-111`, `pages/welcome.css:2-96`)

Sidebar (`pages/WorkspaceLayout.tsx` `Workspace`): brand line 52px **"Gelabber"** (16px 700, ls −0.3px, padding 0 16, border-bottom) and, once the list is known to be empty, `.sidebar-empty` (padding 24/22, 14px/1.6 muted): **"Keine Server. Erstelle einen über + in der Serverleiste oder öffne einen Einladungslink."**

Main (`.welcome`): scrolls, padding `40px 32px 32px`.

| Element | Spec |
|---|---|
| h1 | **"Willkommen, {name}"** (or "Willkommen"), 700 **36px**/1.1 (hero clamp at both viewports), ls −0.035em, balanced wrap |
| load error (optional) | flex wrap, centre, gap 12, margin-top 20, max-width 736, padding 10/12, left 3px danger, radius 5, bg danger-bg 18% on page bg, 14px danger: "Die Server konnten nicht geladen werden." + sm secondary "Erneut versuchen" |
| choices | grid 2 columns × `minmax(0,360px)`, gap 16, margin-top 32 → two 360px cards (left x = 288+32) |
| card | column gap 8, padding 20, border 1 border, radius 10, bg panel; h2 600 16px/1.3; p 14px/1.5 muted, margin-bottom 12; action pinned to bottom (margin-top auto) |
| card 1 | h2 **"Eigenen Server erstellen"**, p **"Mit #allgemein und einem Sprachkanal."**, primary lg full-width **"Server erstellen"** → CreateServerDialog |
| card 2 | h2 **"Server beitreten"**, inline JoinServerForm (§5.3): field + secondary md **"Beitreten"** beside it, the button offset down by 23.55px (13×1.35+6) to sit level with the input |
| next row | margin-top 24, max-width 736, 13px; link **"Profilbild festlegen ›"** (muted, hover text) only if no avatar |

≤600px: single column, padding 32/16/24.

### 4.2 Server overview `/s/{id}` (`pages/CommunityOverview.tsx:41-220`, `pages/overview.css:1-157`)

Layout: overview column (scrolls) + member list (248px, forced visible). Overview column width: **744px @1280**, **1064px @1600**.

| Element | Spec |
|---|---|
| bar | height 52, padding 0 24, flex gap 10, border-bottom, 600 15px/1.3 text: `ChatIcon 18` (muted) + **"Übersicht"** |
| body | column, gap **32**, width 100%, max-width **760**, padding **32** (left-aligned, not centred) → content width **680 @1280**, **696 @1600** |
| head | column gap 6: h1 server name 700 28px/1.15 ls −0.01em; p 400 13px/1.4 muted: **"{n} Mitglied(er)"** + optionally **" · {k} im Sprachkanal"** / **" · {k} in Sprachkanälen"** (`servers/overview.ts` `overviewLine`) |
| alone row (exactly 1 member) | margin-top 10, flex wrap gap 8/16: p **"Noch keine weiteren Mitglieder."** + sm secondary **"Einladen"** (opens InviteDialog) |
| no voice channel | column gap 6: h2 (flex gap 10, min-height 28, 600 16px) `SpeakerIcon 18` muted + **"Kein Sprachkanal"**; p (manage_channels only) **"Erstelle einen Sprachkanal in der Seitenleiste."** |

Voice card (the chosen room: current call → last visited → occupied → first; `overview.ts` `overviewRoom`), rendered by `voice/VoiceRoom.tsx` with `overview` (`VoiceRoom.tsx:428-436, 632-689`; CSS `voice/room.css`, overrides `overview.css:78-157`):

| Element | Spec |
|---|---|
| section | padding 0 |
| heading | flex wrap, centre, gap 10, min-height 28: `SpeakerIcon 18` (muted; **online** colour when you are in it), h2 room name 600 16px/1.3, pill **"Verbunden"** when joined (inline-flex gap 6, height 22, padding 0 8, radius full, bg online 14%, colour online-text, 500 12px, leading 6px dot), red **"Live"** badge when someone is live (radius 3, bg danger-bg, `#fff`, padding 2/6, caps 11px) |
| participants | margin-top 12, flex wrap gap 12; each tile **168×136**, padding 12, border 1 border, radius 10, bg surface, column centred: avatar **56×56** (initials 20px on tint, else panel), name margin-top 8 600 13px/1.35, status line only for "Kamera an"/"Live" (12px muted). Speaking: border + 1px ring in online colour. Mute icons top-left 8/8 (muted); "…" button top-right 6/6 visible on hover. Connection dot and speaker icon hidden here. |
| live/camera stage | above participants: spotlight full width + filmstrip (see voice spec) |
| switch note | when in another room: 12px muted "Du verlässt dabei {Raum}." |
| actions (not joined) | margin-top 16, flex wrap gap 8: primary lg (44px, min-width 180) `SpeakerIcon 20` + **"Beitreten"** / **"Hierher wechseln"**; or text **"Keine Berechtigung für Sprachkanäle."** (500 13px muted); then IconButton secondary 36 `MicIcon 18` "Mikrofon testen"; IconButton secondary 36 `GearIcon 17` "Voice-Einstellungen" |

"Ungelesen" list (only when ≥1 unread text channel; max 5; newest first) (`CommunityOverview.tsx:112-220`, `overview.css:160-245`):

| Element | Spec |
|---|---|
| block | column gap 10 |
| title | **"UNGELESEN"** caps 600 11px/1.3, ls .06em, muted |
| list | border 1 border, radius 10, bg **surface**, clip; rows separated by 1px border |
| row (a link) | grid columns `18px 120px 1fr auto minmax(18px,auto)`, column-gap 12, min-height 52, padding 0 14, text colour; hover bg hover; focus: 2px focus outline inset (−2px) |
| col 1 | `HashIcon 18` muted |
| col 2 | channel name 600 14px, ellipsis; + `PencilIcon 13` muted if a draft exists |
| col 3 | preview 13px muted, one line ellipsis: "{author}: " in text colour (author "Du" for self) + text (mentions as names; "GIF"/"Anhang" fallback) |
| col 4 | time 12px subtle tabular: "18:28" today, "Gestern", "21.10.", other year "21.10.25" (`overview.ts` `shortWhen`) |
| col 5 | unread count pill (§1.11) right-aligned |

### 4.3 Direct messages

**Sidebar** (`components/DmSidebar.tsx:25-191`, `index.css:974-1022`): `.channel-sidebar` (224px, bg panel, border-right).

| Element | Spec |
|---|---|
| header | height 52, flex space-between, gap 8, padding `0 8px 0 16px`, border-bottom; h2 **"Nachrichten"** 600 15px/1.3; IconButton sm 32 `PlusIcon 18` label **"Neue Nachricht"** |
| empty | padding 24/22, column gap 12, 14px/1.6 muted: **"Noch keine Unterhaltungen."** + sm secondary button `PlusIcon 16` **"Neue Nachricht"** |
| list | scroll area padding `8px 8px 12px` (`index.css:776-783, 994-996`); virtual rows every **48px** (`DmSidebar.tsx:23`), i.e. 4px between 44px rows |
| row | height **44**, flex centre, gap 10, padding 0 10, radius 7, 14px muted: PresenceAvatar 32 + name (flex 1, ellipsis) + `PencilIcon 13` (draft, not on active row) + count pill |
| hover | bg hover, text colour — declared after `.is-active`, so hovering the **active** row shows the hover fill, not selected (`index.css:1015-1022`) |
| active | bg selected, text colour |
| unread | name colour text, weight 600; accent bar 3×16 at x = −8px (in the list gutter), radius 0 3 3 0, vertically centred |
| muted conversation | avatar + name at opacity .5 |

User panel at the bottom (shell spec).

**`/d` index** (`pages/DmIndexPage.tsx:25-111`, `index.css:1023-1045`): main column, `.dm-index-hint` centred in both axes: column, centre, gap 14, max-width 460, padding 32, muted, centred text. With conversations: p **"Wähle eine Unterhaltung."** (16px/1.5) + secondary md button `PlusIcon 16` **"Neue Nachricht"**. None: h1 **"Keine Unterhaltungen"** (600 20px/1.25, text colour) + **primary** md button "Neue Nachricht". Error: LoadError "Die Direktnachrichten konnten nicht geladen werden." in padding 24.

**`/d/{id}`** (`pages/DmChannelPage.tsx:70-141`): the regular message pane (chat spec) whose header title is a button `.dm-peer-button` (`chat.css:29-42`): inline-flex gap 10, margin-left −6, padding 4/6, radius 7, hover bg hover; PresenceAvatar 32 + h1 peer name (650 15px). Tooltip "Profil von {name}"; opens UserProfileDialog (§8.2). Error: LoadError "Diese Unterhaltung konnte nicht geladen werden.".

**New message dialog** (`components/NewMessageDialog.tsx:20-86`): Modal **"Neue Nachricht"** containing a Picker (§4.4): label "Person", placeholder **"Name suchen"**, options = avatar 28 (initials 11px) + name + detail (**"Unterhaltung fortsetzen"** if a DM exists, else comma-joined shared server names). Empty texts: **"Wird geladen…"**, **"Keine Personen. Direktnachrichten gehen an Mitglieder deiner Server."**, **"Niemand gefunden für „{query}“."**

### 4.4 Picker, QuickSwitcher, shortcut sheet (`components/Picker.tsx:23-140`, `components/QuickSwitcher.tsx:33-194`, `components/shell.css:58-182`)

Picker: column gap 8: search input (`.ui-field`, full width, no visible label) → listbox: max-height `min(360px, 55vh)` (= 360 at both viewports), margin 0 −8 (bleeds 8px into modal padding), scrolls. Option: flex centre gap 10, min-height 44, padding 4/8, radius 5; highlighted (mouse move or ↑/↓) bg selected; icon box 28×28 centred muted; text column: label 500 13px ellipsis, detail 12px muted ellipsis; trailing badge. Empty: p padding 16/0, 500 13px muted centred.

QuickSwitcher (Strg/⌘+K): Modal **"Wechseln zu"**, picker (screen-reader label "Server, Kanal oder Unterhaltung", placeholder **"Server, Kanal oder Unterhaltung suchen"**), then hint (12px/1.4 muted) **"↑ ↓ auswählen · Enter öffnen · Esc schließen"** — picker and hint are both direct children of the modal body, so the hint sits **20 (body gap) + 12 (margin-top) = 32px** below the list (`QuickSwitcher.tsx:116-130`). DM entry avatar is forced to 28px with 11px initials (`shell.css:92-95`). Entries: text channel label "#{name}" icon `HashIcon 18`, voice `SpeakerIcon 18`, detail = server name; DM avatar, detail **"Direktnachricht"**; server tile 28×28 radius 5 tint bg, 600 12px initials (`tileInitials`: one-word names keep 2nd letter lower-case, "Gaming"→"Ga"), detail **"Server"**. Empty **"Nichts gefunden für „{q}“."**

Shortcut sheet (Strg/⌘+/): Modal **"Tastenkürzel"**, rows (padding 10/0, border-bottom except last, space-between, gap 16): description (500 13px) left, keys right as `kbd` (min-width 26, padding 2/7, border 1 border-strong with 2px bottom, radius 3, bg surface, 600 12px/1.4). Labels (`components/shellShortcuts.ts:84-102`; non-mac keys "Strg"/"Alt"/"Umschalt", mac "⌘"/"⌥"/"⇧"): Strg K "Server, Kanal oder Unterhaltung öffnen"; Strg F "In der Unterhaltung suchen"; Alt ↑ / ↓ "Vorheriger oder nächster Kanal"; Alt Umschalt ↑ / ↓ "Vorheriger oder nächster ungelesener Kanal"; Strg Alt ↑ / ↓ "Vorheriger oder nächster Server"; Esc "Unterhaltung als gelesen markieren"; Umschalt Esc "Ganzen Server als gelesen markieren"; K "Kino im Sprachkanal ein oder aus"; Strg / "Diese Übersicht".

---

## 5. Server dialogs (`components/ServerDialogs.tsx`, `components/invite.css`)

All are default 448px Modals; forms are columns with **gap 20**; button rows `flex justify-end gap 8` at the end of the body (not in the modal footer).

### 5.1 Add server (rail "+") (`ServerDialogs.tsx:48-109`, `invite.css:81-122`)

Title **"Server hinzufügen"** → two choice buttons (grid gap 8). Choice: flex, align start, gap 12, min-height 64, padding 12/16, border 1 border-strong, radius 7, bg panel, left-aligned; hover: border accent + bg hover. Icon 20 in accent-text, margin-top 2. Text column gap 2: strong 600 **14px**/1.3; small 12px/1.4 muted.
1. `PlusIcon` **"Server erstellen"** / "Mit #allgemein und einem Sprachkanal."
2. `LinkIcon` **"Server beitreten"** / "Mit einem Einladungslink oder Code."
Choosing switches the title to "Server erstellen" / "Server beitreten" and shows the form with a **"Zurück"** ghost button.

### 5.2 Create server (`ServerDialogs.tsx:111-192`)

Title **"Server erstellen"**. Field **"Name"** (autofocus, maxLength 100, hint "Mit #allgemein und einem Sprachkanal."), FormError, buttons ghost **"Abbrechen"** (or "Zurück") + primary **"Erstellen"** (pending "Erstellen…"). On success navigates to the server and opens the InviteDialog automatically.

### 5.3 Join server form (`ServerDialogs.tsx:199-265`, `invite.css:125-137`)

Field **"Einladungslink oder Code"**, placeholder **"Link oder Code einfügen"** (no autocorrect). Dialog variant: buttons [ghost "Zurück"] + primary **"Weiter"**. Inline variant (welcome card): flex align-start gap 8, field flex 1 + secondary **"Beitreten"**. Accepts full URL, host/invite/…, `/invite/<code>`, or a bare 12-char code from `abcdefghjkmnpqrstuvwxyz23456789`. Errors: empty "Bitte einen Einladungslink oder Code eingeben."; foreign host "Dieser Link gehört zu einem anderen Gelabber-Server."; else "Das sieht nicht wie ein Einladungslink aus." (`servers/inviteCode.ts:43-52`; parsing `inviteCode.ts:5-41`, codes are lower-cased first). Valid → navigate to `/invite/{code}` (§3.7).

### 5.4 Category dialog (`ServerDialogs.tsx:272-348`)

Title **"Kategorie erstellen"** / **"Kategorie umbenennen"**; field **"Name"** (autofocus, max 100); buttons "Abbrechen" + primary **"Erstellen"** / **"Speichern"**. Closes immediately (optimistic).

### 5.5 Channel dialog (`ServerDialogs.tsx:356-500`)

Title **"Kanal erstellen"** / **"Kanal bearbeiten"**.
1. (create only) fieldset: legend **"Kanaltyp"** (500 13px, margin-bottom 8) + grid 2 equal columns gap 8 of RadioCards (§1.9): `HashIcon 14` + **"Text"** / "Nachrichten, Dateien"; `SpeakerIcon 14` + **"Voice"** / "Sprache, Live" (icon–label gap 6).
2. Field **"Name"** (autofocus, max 100); for text channels a live hint **"Wird gespeichert als #{slug}"** when the slug differs (lower-case, spaces/underscores → `-`, `#` dropped, runs collapsed).
3. Select **"Kategorie"**: first option **"Ohne Kategorie"**, then categories.
4. FormError, then buttons ghost "Abbrechen" + primary "Erstellen"/"Speichern" (no pending label; dialog closes optimistically). Create sends `POST /servers/{id}/channels` `{name, kind: "text"|"voice", category_id: string|null}` (`ServerDialogs.tsx:420`, `servers/api.ts:55-60`). The type fieldset is a column gap 8 whose legend (500 13px/1.35) has margin-bottom 8 (`ServerDialogs.tsx:428-456`).

### 5.6 Invite dialog (`components/InviteDialog.tsx:39-260`, `invite.css:1-78`)

Title **"Zu {server} einladen"**. Panel column gap 12:

| Row | Spec |
|---|---|
| link | flex gap 8: readonly input (`.ui-field`, flex 1, monospace `ui-monospace, SFMono-Regular, Menlo, Consolas` 13px, ellipsis, selects all on focus; shows "Link wird erstellt…" while creating) + primary CopyButton **"Link kopieren"** (initial focus) |
| status | flex, min-height 20: either InlineStatus or the terms summary 12px muted, e.g. **"gültig 7 Tage · unbegrenzt nutzbar"** (`servers/inviteTerms.ts:46-57`) |
| share | secondary button `ShareIcon 16` **"Teilen…"** only where a system share sheet exists (skip on desktop) |
| new-link error | 12px danger "Der neue Link konnte nicht erstellt werden. Der bisherige gilt weiter." |
| disclosure | margin-top 4, border-top 1, padding-top 12; summary inline-flex gap 4, min-height 32, 500 13px muted (hover text): **"Ablauf & Limit ändern"** + `ChevronIcon 14` rotated −90° (closed) / 0° (open) |
| chips | column gap 8, margin-top 8: caps label **"LÄUFT AB NACH"** (margin-top 8), ChipGroup "1 Stunde", "24 Stunden", "7 Tage" (default), "Nie"; caps **"WIE OFT NUTZBAR"**, ChipGroup "1", "5", "25", "Unbegrenzt" (default) |

Status messages: copied "Link kopiert · {summary}" (success), "Link wird erstellt…", "Kopieren fehlgeschlagen. Kopiere den Link manuell." (error), after a terms change "Neuer Link · {summary}". Summary words: expiry "gültig 1 Stunde/24 Stunden/7 Tage" or "unbegrenzt gültig"; uses "{n}-mal nutzbar" or "unbegrenzt nutzbar". Create failure: LoadError "Der Einladungslink konnte nicht erstellt werden. {errorMessage}". Request: `POST /servers/{id}/invites` with body from `inviteInput` (`inviteTerms.ts:37-44`, `servers/api.ts:82-83`): `expires_in_hours` 1 / 24 / 168 (omitted for "Nie") and `max_uses` 1 / 5 / 25 (omitted for "Unbegrenzt") — default terms send `{"expires_in_hours":168}`. The summary is replaced by the InlineStatus while one is shown (`InviteDialog.tsx:208-211`).

---

## 6. Personal settings `/settings/{area}`

### 6.1 Frame (`pages/UserSettingsPage.tsx:42-185`, `pages/settings.css:1-127`, `pages/settingsAreas.ts`)

Sidebar `.settings-sidebar` (224px, panel, border-right):

| Element | Spec |
|---|---|
| brand line | 52px, padding 0 16, border-bottom, **"Einstellungen"** 700 16px ls −0.3px |
| scroll | padding `4px 8px 12px` |
| group heading | height 34, items aligned to bottom, padding `0 10px 7px`, caps 600 11px ls .06em muted: **"KONTO"**, **"APP"**; second group margin-top 8 |
| link | min-height 34, flex space-between, padding 0 10, radius 7, 14px muted |
| hover | bg hover, text |
| current | bg selected, text, weight 600, plus accent bar: 3px wide, from y=7 to bottom−7, at x=0, radius 0 3 3 0 |

Areas (order; `settingsAreas.ts:15-76`): Konto → **Profil** (`profil`), **Profilkarte** (`profilkarte`), **Sicherheit** (`konto`); App → **Darstellung** (`darstellung`, default for `/settings`), **Themes**, **Audio**, **Video**, **Benachrichtigungen**, **App** (only when installable — hidden in the desktop app). User panel at bottom (shell spec).

Main: `.settings-screen` column: header `.lr-channel-header` with h1 **"Einstellungen"** (gap 4), then a scroll area containing `.settings-area`: max-width **1040**, padding **32 40 48** (left-aligned). Content width **912 @1280**, **960 @1600** (1312 − 80 > 1040 − 80).

Area heading (margin-bottom 24): title row flex wrap gap 8/12: h2 label 600 **20px**/1.25 ls −0.01em + scope badge (§1.11; accent tone **"FÜR DEINEN ACCOUNT"** or neutral **"NUR DIESES GERÄT"**); description p margin-top 6, max 60ch, 14px/1.5 muted.

| Area | Badge | Description |
|---|---|---|
| Profil | Für deinen Account | Name und Profilbild. |
| Profilkarte | Für deinen Account | So sehen dich andere. |
| Sicherheit | Für deinen Account | Passwort und angemeldete Geräte. |
| Darstellung | — (per group) | Farbschema, Textgröße und Bewegung. |
| Themes | Für deinen Account | Farbpaletten und eigene Themes. |
| Audio | Nur dieses Gerät | Mikrofon, Lautsprecher und Lautstärke. |
| Video | Nur dieses Gerät | Kamera, Live gehen und Stream-Qualität. |
| Benachrichtigungen | — (per group) | Hinweise auf neue Nachrichten. |
| App | — | Gelabber als App installieren. |

Shared building blocks (`settings.css:129-270`): `.settings-form` column gap 20, max-width 560. Group (`components/SettingsGroup.tsx`): fieldset, legend flex wrap gap 6/10, margin-bottom 8, 600 16px/1.3 + scope badge; consecutive groups 32px apart. `.settings-row` (a Switch row): min-height 64, padding-y 12, border-bottom 1 border, gap 18; description small line-height 1.5, margin-top 2. `.settings-hint` 12px/1.5 muted. `.settings-actions` flex wrap centre gap 8/12. `.settings-note`: margin-top 24 (32 after a group), padding 14/16, border, radius 7, bg panel, 14px/1.5. `.settings-danger`: max 560, margin-top 40, padding-top 24, border-top. Loading area body: 4 skeleton rows; load failure "Dieser Bereich konnte nicht geladen werden.".

### 6.2 Profil (`pages/ProfilePage.tsx:74-294`)

`.settings-form` (560 max):
1. Avatar row (flex centre gap 16): Avatar **lg 64** (preview of typed URL after 400ms) + actions column: sm secondary **"Bild hochladen"** (pending "Lädt hoch…"; file types png/jpeg/webp/gif; applies immediately) and, when an avatar is set, sm ghost **"Entfernen"** (pending "Entfernt…").
2. Field **"Name"** (max 64).
3. Field **"Avatar-URL"** (type url, placeholder "https://…" or "Hochgeladenes Bild"); hint: "Eine URL hier ersetzt das hochgeladene Bild." (uploaded) / "Wird beim Speichern entfernt." / "Wird bei jedem Aufruf von der fremden Seite geladen."
4. FormError.
5. Actions: primary **"Speichern"** (pending "Speichern…", disabled while unchanged) + hint **"Keine Änderungen."** while unchanged + InlineStatus **"Gespeichert."**.
Image errors: "Das Bild ist zu groß (höchstens 20 MB)." / "Dieses Bild kann der Browser nicht lesen. PNG, JPEG, WebP oder GIF gehen." (`auth/avatarImage.ts:133-136`).

### 6.3 Profilkarte (`pages/ProfileCardForm.tsx:61-478`, `settings.css:219-261, 366-492, 653-699`)

Container ≥760px (true at 1280 and 1600): grid `minmax(0,1fr) 320px`, gap 40; form left (**552px @1280**, 600 @1600), preview right **320px**, sticky top 24. ("Ganze Karte" toggle hidden on desktop.)

Preview = ProfileCardView (§8.1) of the form state.

Form (`.settings-form`, gap 20):
1. **"Banner"** label, then actions: sm secondary **"Bannerbild hochladen"** (pending "Lädt hoch…"), sm ghost **"Bannerbild entfernen"** (if set), InlineStatus "Banner gespeichert."; hint **"Wird auf 3 : 1 zugeschnitten und sofort gespeichert."**
2. Two columns (gap 20): colour fields **"Bannerfarbe"** and **"Akzentfarbe"**: swatch button 48×36 (padding 2, border 1 border-strong, radius 5, colour picker), value text (12px muted) `#rrggbb` or **"Standard"**, and when set an IconButton sm `CloseIcon 16` "{Label} zurücksetzen". Unset swatch shows what the card uses (banner: accent colour or accent 65%/panel mix; accent: theme accent). Picker start colour when unset `#7c6cf2`.
3. fieldset **"Avatar-Rahmen"**: grid `repeat(auto-fill, minmax(76px,1fr))` gap 8; option: column centre gap 8, padding 12/6/8, border 1 border, radius 7, 12px; avatar md 36 with the decoration; label **"Keiner"**, **"Ring"**, **"Leuchten"**, **"Regenbogen"**, **"Sterne"**; hover bg hover; checked border accent + 1px accent ring.
4. Field **"Pronomen"** (max 40, placeholder "z. B. sie/ihr").
5. Field **"Status"** (max 80, hint "Steht in der Mitgliederliste unter deinem Namen.").
6. **"Über mich"** textarea, `rows=4` → **100px** tall (4×21 + 14 padding + 2 border, above the 88 min), vertical resize; then optional field error, then counter "{n} / 190 Zeichen" (12px/1.4 muted; danger colour when over) (`ProfileCardForm.tsx:351-373`).
7. FormError.
8. Save bar: when dirty becomes sticky bottom 12, padding 10/12/10/16, border, radius 10, bg elevated, shadow-2, flex wrap gap 8/12: text **"Ungespeicherte Änderungen"** (500 13px, flex 1) + ghost **"Verwerfen"** + primary **"Speichern"** (pending "Speichern…"); InlineStatus "Gespeichert." after save.
Leaving the area while dirty → confirm (§2.2).

### 6.4 Sicherheit (`ProfilePage.tsx:297-408`)

Form: hint "Angemeldet als **{email}**" (email 600 text colour); PasswordField **"Aktuelles Passwort"**; PasswordField **"Neues Passwort"** (hint "Andere Geräte werden dabei abgemeldet."); FormError; primary **"Passwort ändern"** (pending "Ändern…") + InlineStatus "Geändert. Andere Geräte sind abgemeldet.". Danger section: h3 **"Überall abmelden"** (600 16px) + danger-outline **"Auf allen Geräten abmelden"** (confirm §2.2).

### 6.5 Darstellung (`pages/AppearanceSettings.tsx:16-154`, `settings.css:272-364`)

Group **"Farbschema"** [FÜR DEINEN ACCOUNT]:
- Three mode cards in a grid of 3 equal columns, gap 12, max-width 640, margin-top 4 → each **205.3px** wide. Card: grid `auto 1fr`, gap 10, padding 10/12/12, border 1 border-strong, radius 7, bg panel; hover bg hover; checked: border accent + inset 1px accent (2px total); keyboard focus outline 2px focus offset 2.
  - Row 1 (spans both columns): swatch 56px tall, border 1 (preset border colour), radius 5, bg preset background; left 30% strip in preset panel; text bar at top 14, left 40%, 40%×6, radius 3, opacity .8 in preset text; accent bar at bottom 12, left 40%, 24%×10, radius 3 in preset accent. Colours come from the built-in presets (dark: bg `#17191a`, panel `#1d2021`, text `#eef0ee`, accent `#f3b752`, border `#2c3032`; light: `#f8f5f0`, `#f4f0e9`, `#252829`, `#e8a638`, `#dcd6cc`). "System" swatch = left half light swatch, right half dark swatch (inner borders removed).
  - Row 2: radio (§1.8, margin-top 1) + label/hint: **"Hell"** / "Gelabber Hell"; **"Dunkel"** / "Gelabber Dunkel"; **"System"** / "Folgt dem Gerät".
- Hint line (margin-top 10, 12px muted): optional "Gerade aktiv: {theme name}. " + link **"Weitere Themes"** (accent-text, underlined).
- InlineStatus "Gespeichert." / "Nicht gespeichert – erneut versuchen".

Group **"Ansicht"** [NUR DIESES GERÄT] (groups sit directly in `.settings-area`, not in a 560px `.settings-form`, so these rows and their bottom borders span the full content width: **912 @1280 / 960 @1600**; `AppearanceSettings.tsx:120-151`):
- Stacked row (min-height 64, padding-y 12, border-bottom, column gap 10): **"Textgröße"** + small "Gilt für die ganze Oberfläche."; ChipGroup **"Kompakt"** (1.0, default), **"Standard"** (1.125), **"Groß"** (1.25). Applies zoom instantly.
- Switch row **"Kompakte Sprachkanäle"** / "Kleinere Avatare und weniger Abstand."
- Switch row **"Bewegung reduzieren"** / "Berücksichtigt auch die Einstellung deines Geräts."

### 6.6 Themes (`theme/ThemesPanel.tsx:118-604`, `theme/themes.css`, `theme/ThemePreview.tsx`)

Vertical order: toolbar → (sync box) → notice → filter chips → two-column layout → (preview bar) → footnote.

- Toolbar (flex space-between wrap gap 12/16): sm secondary **"+ Eigenes Theme"**, sm secondary **"Importieren"** (accepts `.json`; max 64 KiB; errors "Die Datei darf höchstens 64 KiB groß sein.", "Die Datei enthält kein gültiges JSON.", "Das Theme-Format ist ungültig oder wird noch nicht unterstützt.").
- Sync box (`themes.css:404-416`; margin-top 16, padding 12/14, bg panel, border, radius 5, 13px/1.5): "Wird mit deinem Account synchronisiert …" or the error; buttons sm "Erneut versuchen"/"Erneut anwenden" and "Account-Stand laden".
- Notice: InlineStatus, margin-top 12 when not empty ("„{name}“ angewendet.", "Theme gespeichert und angewendet.", "Theme gelöscht.", "Nicht gespeichert – erneut versuchen").
- Filter ChipGroup (margin 20 0 16): **"Alle"**, **"Hell"**, **"Dunkel"**, **"Eigene"**.
- Layout (viewport > 1180px — true at 1280/1600): grid `minmax(0,1fr) minmax(280px,340px)`, gap 24 → workbench **340px**; gallery **548px @1280** / **596px @1600**. ≤1180px: one column, workbench below, not sticky.
- Gallery: grid 2 columns gap 12 → card width **268px @1280** / **292px @1600**. Radiogroup with roving focus.
  - Theme card (`themes.css:47-131`): column, clip, border 1 border-strong, radius 7, bg panel, left text; hover bg hover; selected: border accent + inset 1px accent and a 22px accent circle at top 8/right 8 with `CheckIcon 14` in accent-ink. Content: small preview (115px tall, see below) → description (padding 12, border-top 1 border-strong): name 700 13px; "Dunkel · Klar" style line 12px muted margin-top 4 (styles: Klar/Weich/Terminal); 4 swatch dots 14px (bg, surface, text, accent) with 1px border-strong, gap 5, margin-top 10. Active theme: **"Aktiv"** at bottom 12/right 10, 600 11px accent-text.
  - System card (first, only in "Alle"): min-height 180, padding 18, gap 9, column centred **vertically only** (text stays left-aligned): `ScreenIcon 32` accent-text, strong **"System"** (700 **16px**/1.5 — inherited page size; only the description `strong` is set to 13px), 12px muted **"Folgt dem Gerät"** (`ThemesPanel.tsx:362-370`, `themes.css:122-133`).
  - Empty "Eigene": "Keine eigenen Themes." (13px/1.6 muted).
- Workbench (sticky top 0, column gap 16): eyebrow caps **"VORSCHAU"** / **"DEIN ENTWURF"**; h3 (margin-top 4, 600 16px) theme name, "Wie dein Gerät" for System, "Neues Theme" for an unnamed draft; large preview; contrast box when issues (margin-top 16, padding 12/14, panel, border, radius 5, 13px/1.5: strong **"Kontrast prüfen"**, issue lines muted margin 6 0 — "Text ist auf mindestens einer Fläche schwer lesbar.", "Sekundärtext braucht mehr Kontrast.", "Schrift auf der Akzentfarbe ist schwer lesbar.", "Links in der Akzentfarbe heben sich zu wenig ab.", "Rahmen von Eingabefeldern heben sich zu wenig ab." — plus sm **"Kontraste verbessern"** in a draft).
  - Without draft: sm buttons **"Als Vorlage verwenden"**, [custom only] **"Bearbeiten"**, danger-outline **"Löschen"**, then **"Exportieren"** (downloads `{name}.gelabber-theme.json`).
  - Draft editor (`themes.css:338-403`): column gap 16; labels column gap 6 12px; inputs/selects min-height 40, border 1 border-strong, radius 5, padding 8, bg **panel**: **"Name"** (max 60); pair (2 cols gap 10) **"Modus"** (Dunkel/Hell) and **"Stil"** (Klar/Weich/Terminal); fieldset legend **"Farben"** (600 13px, mb 10) with a 2-column grid (gap 10) of 8 colour fields in order **Hintergrund, Seitenleiste, Flächen, Serverleiste, Text, Sekundärtext, Akzent, Rahmen**: each label text + row (gap 5): colour swatch 32×36 (padding 2, panel bg, border-strong, radius 5) + hex input (monospace 11px, padding 5). Buttons: primary **"Speichern und anwenden"**, ghost **"Abbrechen"**. New theme name: "{orig ≤48 chars} – Eigene".
- ThemePreview (large; drawn with **the previewed theme's own variables**; `ThemePreview.tsx:4-67`, `themes.css:173-291, 460-498`): grid columns 30%/70%, rows `1fr auto`, min-height 240, border 1 border, radius 7, theme font, shadow-card.
  - Sidebar (panel, padding 16/10, gap 15, border-right): "Gelabber" (700 16px), "KANÄLE" (10px muted), selected item (bg selected, accent-text, radius 5, padding 8/5, 11px) `ChatIcon 12` "Übersicht", then "# allgemein" (`HashIcon 12`), "Wohnzimmer" (`SpeakerIcon 12`) at 10px muted, icon gap 4.
  - Chat (bg = the previewed theme's `--lr-bg`, padding 14, column gap 16; composer pushed to the bottom with margin-top auto): header "# allgemein" (12px bold) + "Allgemeiner Chat" (10px muted); two messages (gap 8, 11px): avatar box (surface, radius 7, padding 7, 9px "MA"/"DU"), name bold "Mara"/"Du" + small 9px muted "18:42"/"18:43", text "Wer ist heute Abend dabei?" / "Bin im Wohnzimmer." (margin-top 4); composer at bottom (surface, muted, border, padding 9, 10px, radius 5, space-between): "Nachricht an #allgemein" + bold accent-text "Senden".
  - Footer (spans, panel, border-top, padding 10, 10px, space-between): 6px online dot + "Verbunden · Wohnzimmer"; pill "Mikrofon an" (accent bg, accent-ink, radius 5, padding 5/8, 500).
  - Small variant (gallery, 115px fixed height, no border/radius): sidebar padding 10/6 gap 7, "Gelabber" 8px, spans 5px, selected 6px padding 4/2, icons 7px; chat padding 8 gap 7, header 7px (subtitle hidden), messages 6px gap 4 (only the first message; times hidden), avatar 5px padding 4, composer 5px padding 5; footer hidden.
- Preview bar while a card is only "tried on" (sticky bottom 0, margin-top 20, padding 12/16, border, radius 10, elevated, shadow-2, 500 13px, flex space-between wrap): "**„{name}“** wird angezeigt" + primary **"Behalten"** + ghost **"Zurück zu {active name}"**. Esc reverts. Clicking a card previews it app-wide immediately.
- Footnote (margin-top 24, 13px/1.6 muted): "Mit deinem Account synchronisiert." (or "Deine Themes gehören zu deinem Account.") + " Paletten nach " + underlined link "Omarchy" + ", für Gelabber angepasst."

### 6.7 Audio / Video (`voice/VoiceSettings.tsx:69-516`, `voice/settingsLayout.tsx`, `voice/quality.css:92-289`, overrides `pages/settings.css:584-614`)

Form: column gap 20, max-width **720** (container 720 > 480 → two-column rows at 1280/1600). Overrides inside settings: every `<button>` min-height 36 and **text colour accent-text**; selects full width with bg **surface** and border-strong.
- Group (`.media-group`): head (flex baseline space-between, gap 12, padding-top 6): h3 600 12px/1.4 **subtle**; optional value pill (padding 2/8, radius 5, bg selected, accent-text, 12px).
- Row (`.media-row`): grid `136px 1fr`, column gap 16, row gap 6, padding 12/0, border-bottom (none on a group's last row); label 500 13px text, padding-top 8; control column gap 8. Hints 12px/1.4 muted. SwitchLine: hint text left, switch right, min-height 36, gap 16.

Audio area content (strings verbatim; desktop-app variants where they differ):
- **Eingabe**: "Mikrofon" device select (first option = system default); "Mic-Gain" slider 0–200 %, scale "0 %"/"100 %"/"200 %", hint "100 % fügt keine Verstärkung hinzu."; "Senden" ChipGroup **"Sprachaktivierung"** / **"Push-to-Talk"** + hint "Das Mikrofon sendet, sobald du sprichst." / "Das Mikrofon sendet nur, solange du die Taste hältst."; (PTT) "Taste" sm button "Taste festlegen" / key name / "Taste oder Maustaste drücken … (Esc bricht ab)", hint "Ohne Taste bleibt dein Mikrofon im Gespräch stumm."; (PTT) "Nachlaufzeit" slider "{n} ms", hint "So lange sendet das Mikrofon nach dem Loslassen weiter."; "Filter" RadioCards (column gap 6; `voice/settings.ts:44-73`): desktop **"Verbesserte Sprache"** / "Lokale Rauschunterdrückung", **"WebRTC-Filter"** / "Rauschunterdrückung und Auto-Gain von WebRTC", **"Original / Musik"** / "Stereo sofern unterstützt, ohne Rauschfilter, Auto-Gain oder Sprechpausen-Erkennung" (web reference screenshots instead show "Verbesserte Sprache" / "Lokale Rauschunterdrückung und Auto-Gain; Browserfilter als Ersatz" and **"Browserfilter"** / "Rauschunterdrückung und Auto-Gain deines Browsers"); then status line + sm button **"Mikrofon testen"**.
- **Ausgabe**: "Lautsprecher" select; "Gespräche" slider (0 % … 100 % middle … max), hint "Über 100 % wird der Ton digital verstärkt."; "Stream-Ton" slider + Switch "Stream-Ton stummschalten", hint "Der Ton von Lives hat eine eigene Lautstärke. Taub schaltet auch diesen Ton aus."; "Signaltöne" SwitchLine "Töne für Beitreten, Verlassen, Stumm, Taub, Kamera und Live.", Switch "Töne, wenn andere sich stumm schalten", field label (12px subtle) "Lautstärke der Signaltöne" + slider (default 35 %), hint "Nutzt die Wiedergabelautstärke. Signaltöne spielen auf dem Standard-Ausgabegerät des Systems.", sm button **"Testton abspielen"**.
- Disclosure (`quality.css:226-280`; in settings it is collapsed; border 1 border, radius 7; summary flex centre gap 12, min-height 44, padding 0 14, 600 13px text, hover bg hover; **leading** chevron = 6×6 box with 2px right+bottom borders in currentColor, rotated −45° (points right) closed / 45° (points down) open, 180ms; then title; then right-aligned (margin-left auto) small "Für Musik oder Lautsprecher" 12px/1.4 subtle; body padding 0 14, last row no border): title **"Echo und WebRTC-Filter"** (web: "Echo und Browserfilter"); rows "Echo" ("Echo-Unterdrückung (AEC). Bei Lautsprechern empfohlen. Für Musik mit Kopfhörern kannst du sie ausschalten."), "Rauschfilter" (only in WebRTC-filter mode: "Rauschunterdrückung von WebRTC."), "Auto-Gain" (not in Original: "Auto-Gain von WebRTC. Gleicht leise und laute Mikrofone auf einen hörbaren Pegel an.").

Video area content:
- **Kamera** [pill "Automatisch" or "1080p · 60 FPS"]: "Gerät" select; "Qualität": two selects side by side (grid 2 cols gap 12) with 12px subtle labels "Auflösung" ("Auto (bis 1080p)", "…p", "4K · 2160p") and "Bildrate" ("Auto (bis 30 FPS)", "{n} FPS"); apply note "Auf den laufenden Stream angewendet." / "Bitrate und FPS-Limit gelten sofort. Die Auflösung gilt ab dem nächsten Stream."
- **Live** [pill]: "Qualität" as above + hint "Höhere Werte brauchen mehr Rechenleistung und Upload. Die erreichbare Qualität hängt von Quelle, Gerät und Verbindung ab."; (if app audio capable) "Ton mitsenden" SwitchLine + "Ton von" select (default "Alle Anwendungen außer Gelabber" / "Alle Anwendungen"; "(gerade still)" suffix).
- **Upload** [pill "ohne Limit" or bitrate]: "Sparmodus" SwitchLine ("Für wenig Upload: Sprache 64 kbit/s, Stream-Ton 128 kbit/s, Video gemeinsam …"); "Video-Limit" two bordered radio pills (flex wrap gap 10; each min-height 36, padding 0 12, border 1 border, radius 7, 500 13px; checked border accent + bg selected) **"Ohne Limit"** (or "Sparmodus (2,5 Mbit/s)") and **"Eigenes Limit"**; custom: number input 8rem right-aligned + "Mbit/s für alle Videos zusammen"; hint "Die Desktop-App passt die tatsächliche Bitrate an die Verbindung an. Das Limit ist eine Obergrenze, keine feste Datenrate."
- Footer line 12px subtle "Änderungen gelten beim nächsten Start und werden, soweit unterstützt, auch auf laufende Streams angewendet."

In-call dialog variant (`components/VoiceSettingsDialog.tsx:12-90`, `quality.css:52-90`): Modal **"Voice & Video"** large (640), footer ghost **"Fertig"**; sticky tab row (gap 4, border-bottom, bg elevated, margin-bottom 20) with tabs **"Audio"**, **"Video & Stream"**, **"Diagnose"**: min-height 36, padding 0 12, 500 13px muted, 2px transparent bottom border; selected text colour, 600, accent bottom border; hover text.

### 6.8 Benachrichtigungen (`components/MessageAlertSettings.tsx`, `NotificationPrivacy.tsx`, `NotificationPermission.tsx`)

Group **"Nachrichten"** [NUR DIESES GERÄT] switch rows: "Popup bei neuen Nachrichten in anderen Chats" (default on); desktop: "Benachrichtigung, wenn das Gelabber-Fenster nicht im Vordergrund ist" (web: "Browser-Benachrichtigung, wenn der Tab nicht im Vordergrund ist"; default off); "Ton bei Benachrichtigungen am Computer" / "Ein Ton für mehrere Nachrichten kurz hintereinander, keiner, während du taubgeschaltet bist. Auf dem Handy gilt die Geräteeinstellung." (disabled while the previous is off); desktop only: "Fenster schließen minimiert in den Infobereich" / "Ein laufender Anruf geht weiter. Beenden über das Symbol im Infobereich." (`components/CloseToTray.tsx`).
Group **"Sperrbildschirm"** [FÜR DEINEN ACCOUNT]: "Inhalt in Benachrichtigungen verbergen" / "Benachrichtigungen zeigen nur „Neue Nachricht von …“, ohne Text und ohne Chat. Gilt für alle Geräte deines Accounts."; error "Die Einstellung konnte nicht gespeichert werden. Versuche es erneut."
Note box (§6.1): desktop app with system notifications: "Benachrichtigungen laufen über dein System. Sperrbildschirm und „Nicht stören“ stellst du dort ein." (browser texts: `messages/notify.ts:86-100`).

---

## 7. Server settings `/s/{id}/settings` (`pages/ServerSettingsPage.tsx`, `settings.css:494-582`)

Sidebar = the server's channel sidebar (shell spec). Header h1 **"Servereinstellungen"** + span " · {server}" (muted, 500). Scroll area:
- Section nav (sticky top 0, z above content, flex gap 4, padding 8/32, border-bottom, bg page bg): pill links min-height 32, padding 0 12, radius full, 500 13px muted, hover bg hover/text. Managers see **Allgemein · Rechte · Einladungen · Mitglieder · Sperren · Gefahrenzone**; others **Mitglieder · Gefahrenzone**. Click scrolls the section heading to the top (scroll-margin 64).
- Body `.settings-area` (as 6.1, padding 32/40/48, max 1040) column gap **40**; each section column gap 14, max-width **720**; heading h2 600 20px/1.25; hint p margin-top 4, 14px muted. Lists: border-top 1; row min-height 56, padding-y 8, border-bottom, flex wrap centre gap 8/12; name 500 13px ellipsis.

| Section | Content |
|---|---|
| Allgemein | icon row (flex centre gap 16): server tile **64×64**, radius 22 (`calc(lg×2.2)`), tint bg, initials 600 **20px**/1, ls .01em (`tileInitials`) or image (`index.css:523-537`, `settings.css:228-232`). It reuses the rail's `.server-tile` rules, so on **hover** the radius animates to 10px and a 4×16 muted pill appears 10px left of the tile (`index.css:474-541`); actions column gap 8 holds a `.settings-actions` row; sm "Bild hochladen" (pending "Lädt hoch…"), sm ghost "Entfernen" (pending "Entfernt…"), status "Gespeichert."; form: field **"Servername"** (max 100) + primary "Speichern" (pending "Speichert…", disabled unchanged) + status |
| Rechte für Mitglieder | hint "Gilt für alle Mitglieder außer dem Inhaber. Änderungen wirken sofort." Switch rows (§6.1 row style) — Server verwalten / "Umbenennen, Rechte ändern, Einladungen, Kick und Ban."; Kanäle verwalten / "Kategorien und Kanäle anlegen, umbenennen, löschen."; Nachrichten löschen / "Fremde Nachrichten im Chat löschen."; @everyone und @here / "Alle Mitglieder auf einmal erwähnen und benachrichtigen."; Nachrichten schreiben / "In Textkanälen schreiben."; Dateien senden / "Anhänge hochladen."; Voice beitreten / "Sprachkanälen beitreten."; Live gehen / "Im Sprachkanal einen Bildschirm oder ein Fenster für alle dort zeigen." (`servers/permissions.ts:6-51`) |
| Einladungen | hint "Alle aktiven Links. Jedes Mitglied kann Links erstellen."; secondary **"Neuen Link erstellen"** (opens §5.6); "Lade Einladungen…" / "Keine aktiven Einladungen."; row: code (monospace 13px) over hint "{uses}/{max or ∞} benutzt · läuft in 6 Tagen ab / läuft nie ab / abgelaufen" (`servers/inviteText.ts`), actions sm CopyButton **"Kopieren"** + sm danger-outline **"Widerrufen"**; copy failure "Kopieren ging nicht. Der Code ist {code}." |
| Mitglieder ({n}) | rows: PresenceAvatar 32 + name + badge **"INHABER"** (accent) / **"MITGLIED"** (neutral) + member "…" menu (§9.2) |
| Gesperrte Konten | hint "Gesperrte User können auch mit einem Einladungslink nicht wieder beitreten."; "Lade Sperren…" / "Niemand ist gesperrt."; rows: avatar (offline dot) + name + sm secondary **"Entsperren"** |
| Gefahrenzone | hint owner "Löscht Kanäle, Kategorien, Nachrichten, Einladungen und Mitgliedschaften endgültig." / member "Wieder beitreten kannst du nur mit einem neuen Einladungslink."; danger-outline **"Server löschen"** / **"Server verlassen"** |

Load failure: LoadError "Die Servereinstellungen konnten nicht geladen werden.".

---

## 8. Profile card and profile dialogs

### 8.1 ProfileCardView (`components/ProfileCard.tsx:28-99`, `components/profileCard.css`)

| Part | Spec |
|---|---|
| card | relative, clip, border 1 border, **radius 14**, bg panel; inside, `--lr-bg` is redefined to panel (affects presence-dot border and decoration gap rings) |
| banner | height **96**; uploaded banner image cover, else banner colour, else accent colour, else accent 65% + panel (`#a88241` / `#ecc076`) |
| avatar | absolute top 60, left 18, padding 4, circle bg panel (→ 72px disc) holding Avatar **lg 64** (+ decoration, + presence dot when known) |
| body | column gap 12, padding `46px 18px 18px` |
| name | h3 650 **22px** (line-height 1.5), wraps |
| pronouns / presence | 13px muted, margin-top 2 ("Online", "Abwesend", "Offline") |
| status | padding 8/10, border-left 3px profile accent, radius 8, bg surface, 14px |
| Über mich | h4 650 11px uppercase ls .6px muted mb 4 **"ÜBER MICH"**; p 14px/1.45, keeps line breaks |

### 8.2 UserProfileDialog (DM peer) (`components/UserProfileDialog.tsx:11-36`)

Modal **"Profil von {name}"** (448; card 398px wide); body `.member-profile` column gap 22: card + actions (flex wrap gap 12) ghost **"Schließen"**.

### 8.3 MemberProfileDialog (`components/MemberProfileDialog.tsx:19-154`, `overview.css:249-305`)

Modal title **"Dein Profil"** (self) or **"Profil von {name}"**. Body column gap 22:
1. ProfileCardView with presence.
2. Details `dl` grid gap 12; rows flex baseline space-between gap 20; dt 13px muted, dd 14px right-aligned: **"Server"** {name}; **"Mitgliedschaft"** "Inhaber"/"Mitglied"; **"Dabei seit"** `dd.mm.yyyy` (de-DE medium).
3. If in voice: link (flex gap 10, accent-text) `SpeakerIcon 20` "In {channel}".
4. If in your call: per-user volume control (voice spec).
5. Actions flex wrap gap 12: `.lr-primary` (inline-flex gap 10, padding 12/18, radius **9**, 650 14px, accent bg/ink, hover brightness 1.08, disabled opacity .55): self → **"Profil bearbeiten"**; others → `ChatIcon 19` **"Nachricht senden"** (pending "Unterhaltung wird geöffnet…"); then ghost **"Schließen"**.
6. Moderation disclosure (managers, not owner/self) (`components/MemberActions.tsx:62-86`, `memberActions.css:2-40`): padding-top 12, border-top 1; summary min-height 32, caps 11px muted **"MODERATION"** + `ChevronIcon 14` (margin-left 4, rotated 180° open), hover text; actions margin-top 8 flex gap 8: sm danger-outline **"Vom Server entfernen"**, sm danger **"Sperren"**.

---

## 9. Context menus

### 9.1 Message actions (`components/MessageActionSheet.tsx:55-175`, `chat.css:833-893`)

Popover (§2.4) `bottom-end` at the message's "Mehr" button (or pointer for right-click), aria "Nachricht von {author}". Menu column, min-width **220** (+16 popover padding):
- Reactions row (if allowed): flex wrap gap 4, padding 2/2/8, margin-bottom 4, border-bottom: six 36px circles (bg surface, 18px emoji, hover bg hover) **👍 ❤️ 😂 😮 😢 🎉**, own reactions with inset 1px accent ring + bg selected; then a `PlusIcon 18` circle "Andere Reaktion auswählen".
- Items: flex centre gap 10, min-height **32**, padding 0 10, radius 5, 500 13px text, icon 18 muted; hover/focus bg hover: `ReplyIcon` **"Antworten"**, `PencilIcon` **"Bearbeiten"**, `CopyIcon` **"Text kopieren"**; divider (1px border, margin 4/0); `TrashIcon` **"Löschen"** in danger (tooltip "Löschen (Umschalt: ohne Nachfrage)").
Arrow keys/Home/End move focus.

### 9.2 Member row menu (`MemberActions.tsx:89-137`, `memberActions.css:41-68`)

IconButton sm 32 `MoreIcon 18` "Aktionen für {name}" → Popover `bottom-end`: column min-width 200; items min-height 36, padding 0 12, radius 5, 500 13px, hover bg hover: **"Vom Server entfernen"** (text), **"Sperren"** (danger). Each asks first (§2.2).

### 9.3 Notification settings dialog (header bell, or "Benachrichtigungseinstellungen…" in a row menu) (`components/NotificationSettingsDialog.tsx:53-296`, `components/notificationSettings.css`)

Modal **"Benachrichtigungen: {name}"**; column gap 20:
1. Section h3 **"Stummschalten"** (600 14px/20px), gap 8: not muted → one row (`.notify-mute-row`, flex-wrap, gap 8) of `.ui-chip` pills (32 tall, padding 0 12, `flex: 1 0 auto`, centred, `title` the full length): "15 Min.", "1 Std.", "8 Std.", "24 Std.", "Unbegrenzt". Muted → box (flex wrap, centre, gap 8, padding 8/12, radius 7, bg surface, 500 13px/1.35, space-between): inline-flex gap 8 `BellOffIcon 16` + "Stumm bis {14:30 Uhr | Mi., 5. Nov., 14:30 Uhr}" or "Stumm, bis du es aufhebst" + sm **"Aufheben"**. Hint 12px muted: DM "Stumm: kein Zähler, keine Benachrichtigung und kein Ton." else "Stumm: kein Punkt, keine Benachrichtigung und kein Ton. @Erwähnungen zählen weiter." (+ " Der ganze Server ist gerade stumm.").
3. (not DMs) fieldset (column gap 8, disabled while saving) with legend **"Benachrichtigen bei"** (600 14px/20px, mb 4), RadioCards: channel only "Wie der Server ({level})"; "Alle Nachrichten" / "Jede neue Nachricht"; "Nur @Erwähnungen" / "Wenn dich jemand erwähnt oder @everyone/@here schreibt"; "Nichts" / "Keine Benachrichtigungen, kein Zähler".
4. Error 12px danger "Nicht gespeichert. Versuch es noch einmal." / "Nicht mehr verfügbar.".
Saving (`messages/notificationSettings.ts:26-56`): `PUT /servers/{id}/notification-settings` or `PUT /channels/{id}/notification-settings` (DMs are channels) with `{level: "all"|"mentions"|"none"|null, muted: bool, mute_minutes?: number}` — a mute chip sends `mute_minutes` 15/60/480/1440 (none for "Bis ich es wieder einschalte"); "Aufheben" sends `muted:false`; a channel's "Wie der Server" is `level:null`, a server's "Alle Nachrichten" is also sent as `null`. List: `GET /notification-settings` → `[{scope:"server"|"channel", id, level, muted, muted_until}]`. The `{level}` in "Wie der Server ({level})" is the inherited level's label ("Alle Nachrichten", "Nur @Erwähnungen", "Nichts"). Time format: today `HH:MM Uhr`, else `de-DE {weekday:short, day:numeric, month:short}` + ", HH:MM Uhr" (`NotificationSettingsDialog.tsx:300-308`).
5. Footer (`.notify-footer`, space-between, gap 8): sm secondary **"Als gelesen markieren"** / **"Server als gelesen markieren"** / **"Alle Direktnachrichten als gelesen markieren"** (left), ghost **"Fertig"** (right, initial focus).

---

## 10. Computed geometry summary (desktop, Kompakt)

| Screen | 1280×800 | 1600×900 |
|---|---|---|
| Auth column x / width | 448 / 384 (card inner 318) | 608 / 384 |
| Auth login column top / height (≈) | 172 / 456 | 222 / 456 |
| Modal default / wide / large width | 448 / 512 / 640 | same |
| Modal max height | 768 | 868 |
| Welcome cards | 2 × 360 at x=320 and 696 | same x |
| Overview column / body content | 744 / 680 | 1064 / 696 |
| Settings content width | 912 | 960 |
| Settings mode card width | 205.3 | 205.3 |
| Profilkarte form / preview | 552 / 320 | 600 / 320 |
| Themes gallery / workbench / card | 548 / 340 / 268 | 596 / 340 / 292 |
| Media settings form | 720 (row label col 136) | 720 |
| Server settings sections | 720 max | 720 max |
| Toast stack | right 16, bottom 12 (no composer) / 12 above composer | same |
| New-message toasts | top 64, right 16, width 352 | same |
| Appearance switch rows | full content width 912 | 960 |

Hero/clamp: welcome title 36px at both. No `vw`-dependent sizes otherwise in this area.

---

## 11. Gaps / not covered here

- User panel (bottom of sidebars), server rail, channel sidebar, member list, message pane/composer, and the voice room tile internals are owned by the shell/chat/voice specs; only their touch points are listed here.
- `MicrophoneTest`, `UserVolumeControl`, `VoiceDiagnostics` ("Diagnose" tab) and the PWA `InstallApp` area/disclosure (web-only, hidden in the desktop app) are not specified in detail.
- Rendered heights of text inputs (37px) and the auth column top are computed from CSS, not measured in a browser; verify with a screenshot before tuning SSIM.
- Toast vertical position depends on CSS anchor positioning (Chromium); WebKitGTK without it uses bottom 74px — pick per reference screenshot engine.
- Theme palettes other than the two defaults are listed only by source line (`theme/presets.ts:22-225`); derive their tokens with `themeVariables(fixContrast(p))` exactly as for §0.
