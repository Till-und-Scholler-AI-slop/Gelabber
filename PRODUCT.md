# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

- **Today:** one friend group on one instance (7 members, three regular callers). They talk in the evening, share screens and watch each other's streams. Two use the native Linux app and one uses Firefox, all on NVIDIA PCs.
- **Target (decided 2026-11-10):** larger, international private groups. The layout target is up to about 100 members per community and about 25 people in one call. The main case stays 2–8 people in a call.
- **Operators:** Rafael and Silas. Two people build and run it.

## Product Purpose

Chat, voice and live screen sharing for a community. Whoever goes live shows everyone in the voice channel a screen or window, several at once. A group either runs Gelabber on its own server or uses an instance that is run for it.

Success for the next stage: a group that has never met the authors can sign up on the hosted instance, create a server, invite its people and hold a call with streams, in German or English.

## Positioning

- Several Go Live streams at the same time at high quality, from a self-hosted mediasoup SFU.
- Open source (MIT). The hosted service runs the same code as a self-hosted instance; there is no hosted-only fork.
- "Selbst hosten, frei reden" stays true: self-hosting is a first-class path.
- The landing page names no competitor and states only what was measured.

## Operating Context

- Desktop first. Calls run in the native desktop app (Linux, Windows) or in a browser; the PWA covers phones and has not been tested on real devices.
- A typical session: join a voice channel, talk, one or more people go live, the others watch, text chat runs beside it.
- The web client is the visual reference. The native egui app mirrors its tokens and is scored against web screenshots by `desktop/parity` (SSIM ≥ 0.90).
- Deployment today is `docker compose up` from `deploy/compose`, with an operator guide in `deploy/README.md`.

## Capabilities and Constraints

Shipped (v0.9.2): servers with text and voice channels, channel categories, a server image, DMs, reactions, search, link previews, GIFs, voice with camera and Go Live, the stage views Raster, Fokus and Kino, a persistent bottom call bar, themes with an editor, push notifications, email and password accounts with optional account mail, invites, owner plus member roles with one shared set of eight permissions.

All of the following ships as one release, v0.10 (decided 2026-11-10; tracked as GitHub issues in one milestone, not in a roadmap document). Work packages, in build order:

1. **English:** per-account language, string catalogs in both clients. New accounts take English or German from the browser language.
2. **Call view:** the stage fills its area when nobody shares a picture, a Live shows how many people watch it and who, the stage facts and view switch move into the channel header, and the call bar loses its settings button.
3. **Roles and channels:** a custom role builder at server level (name, colour, permissions from the existing fixed list; several roles per member, rights add up; "Alle Mitglieder" is the base role), and channels and voice rooms limited to chosen roles. No per-channel overrides. Kick and ban move out of "Server verwalten" into a new permission, "Mitglieder moderieren".
4. **Hosted beta:** one shared instance, free and by invitation. Landing page, sign-up and first server, operator console (accounts, servers, usage, reports), legal pages, account deletion and data export, sign-in with Google, and the list of signed-in devices under Sicherheit.
5. **Discovery:** the opt-in directory with reports and delisting.

Not in v0.10: managed instances (on request only, nothing built), threads, custom server emoji.

Rules that later work must keep:

- **Hosted tenancy:** one shared instance where people sign up, create a server and invite their group. Not one instance per group.
- **Paid offer:** a dedicated managed instance, on request. No provisioning is built until a group asks.
- **Sign-in with Google:** built as generic OpenID Connect with Google as the first provider. Off by default per instance; on for the hosted instance; self-hosters can enter their own client ID. A Google sign-in with a verified email joins the existing account with that email. Email and password stay for everyone. The desktop app signs in through the system browser and a temporary local port.
- **Discovery:** off by default. An owner opts a community in; a listing shows name, artwork, description, language and member count; the owner chooses open joining or request to join; every listing has a report button and the operator can delist. On self-hosted instances the directory is off unless the operator turns it on.
- **Product rules from `AGENTS.md`:** own WebSocket signaling, mediasoup as the only SFU, client-side audio processing with self-hosted RNNoise assets.

Undecided:

- Pricing. It waits for a measured cost per call hour on the hosted instance.
- The operator of record for the Impressum and terms (Rafael, Silas or a company).
- The contact address for dedicated instances and for reports.

## Brand Commitments

- The name is Gelabber, also for an international audience.
- The interface identity is the shipped one, called "Living Room" in the code (`--lr-*` tokens): rail, sidebar, optional members panel and a bottom call bar, drawn by default in the dark theme "Gelabber Dunkel" with an amber accent. New features are drawn in this identity. Four replacement directions were rejected on 2026-10-27, and three ways to fold the server rail into the sidebar were rejected on 2026-11-10: the rail stays.
- Existing parts stay as shipped unless a change is asked for (2026-11-10: the sidebar participant list, the red of "Verlassen", row spacing).
- People can switch themes (14 presets, a theme editor, three styles), so every design must keep working in all of them.
- Interface copy is German today, informal ("du").

## Evidence on Hand

- Current screenshots come from the parity scenes: `web/scripts/parity/shots.mjs` against a seeded local stack. `docs/assets/readme-*.png` are from v0.8 and no longer show the current call view.
- `docs/design-qa.md` records the approved integration and the compact iteration. Parts of it are stale (widths, font stack).
- There are no testimonials, user counts, benchmarks for strangers' groups or pricing. Future work must not invent them. Nothing above the authors' own group size has been measured.

## Product Principles

1. The call comes first. A newcomer judges Gelabber by the voice room and the streams in it.
2. One codebase for self-hosted and hosted.
3. Private by default. Communities are joined by invite; being findable is an owner's choice.
4. Claims follow measurements.
5. Every web feature gets its native counterpart; the two clients stay at parity.

## Accessibility & Inclusion

- Themes are checked for WCAG contrast when they are derived (`web/src/theme/model.ts`); text needs 4.5:1.
- Motion stops under reduced motion.
- Touch targets are 44 px under a finger.
- Known gaps in the native app: new messages are not announced to screen readers, and there is no right-to-left support.
