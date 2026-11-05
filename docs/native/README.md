# Native client specs

Extracted from the web client (`web/src`) and the backend (`backend/`) on
2026-11-05 for the egui desktop client (`desktop/ui`, `desktop/client`).
Each file cites `file:line`; when a spec and the source disagree, the source
wins. The parity check in `desktop/parity` decides whether a screen matches.

| File | Covers |
|---|---|
| `ui-tokens-shell.md` | theme tokens, fonts, rail, channel sidebar, header, member panel, user panel |
| `ui-chat.md` | message pane, rows, markdown, attachments, reactions, composer |
| `ui-voice.md` | voice room, stage views, tiles, call bar, native media calls |
| `ui-auth-settings-dialogs.md` | sign-in pages, overview, DMs, settings, dialogs, menus |
| `protocol-rest-auth.md` | REST endpoints, cookies, CSRF, errors, native cookie mode |
| `protocol-gateway.md` | `/ws` and `/media/ws` frames, timers, resume |
