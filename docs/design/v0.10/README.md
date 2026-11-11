# v0.10 design references

Rendered from the design canvas "Gelabber Next" on 2026-11-10. Product decisions are in `PRODUCT.md` at the repo root. `today/` holds screenshots of v0.9.2 for comparison; anything a reference does not change stays as shipped.

| File | Shows |
|---|---|
| `call-no-pictures-4.png`, `call-no-pictures-25.png` | A call with no camera and no Live: person tiles fill the stage. Layout logic in `source/call-no-pictures.dc.html` (same rule as `fitGrid` in `web/src/voice/stage.ts`). |
| `call-watching-live.png` | Watching a Live in Fokus: viewer count on the picture's label, "Schaut zu" and "Ist live" on the person tiles. |
| `call-you-are-live.png` | Your own Live: the Live button in the call bar shows the count and opens the list of who watches. |
| `call-phone-390.png` | The call without pictures at 390 px. The phone chrome is approximated. |
| `settings-language-de.png`, `settings-language-en.png` | Einstellungen → Darstellung with the new "Sprache" section, in German and in English. |
| `roles-editor.png` | Servereinstellungen → Rollen (replaces the tab "Rechte"). |
| `roles-members.png` | Servereinstellungen → Mitglieder with role chips and the picker. |
| `channel-access.png` | "Kanal bearbeiten" with the third field "Zugriff". |
| `hosted-landing.webp` | Public landing page, English. `landing-preview-en.png` is the drawn preview of its screenshot; the page ships real ones (see below). |
| `hosted-sign-in.webp` | Sign-in with "Mit Google anmelden" and the legal links. The "G" mark is a stand-in; the real button follows Google's branding rules. |
| `hosted-security.png` | Einstellungen → Sicherheit: password and Google, signed-in devices, data export, account deletion. |
| `hosted-operator-console.png` | Operator area "Betrieb", page "Server". Sample data. |
| `hosted-imprint.png` | Impressum with placeholders the operator fills in. |
| `directory.png` | "Entdecken": the opt-in directory. |

## Decisions that the pictures do not state

- The stage facts ("4 im Kanal · 1 Live" or "Du bist hier mit …"), the Raster/Fokus switch and Kino sit in the channel header. The separate stage bar is gone.
- The call bar has no settings button. Call settings open from the gear in the user panel; a click on the connection status opens the diagnostics.
- In Raster with pictures, people without a camera stay small chips, as in v0.9.2.
- The sidebar lists every participant of a voice channel, as in v0.9.2.
- The server rail stays. Servers without an image keep one shared fallback colour.
- Roles: several per member, rights add up, "Alle Mitglieder" is the base role, the owner is fixed. Kick and ban move from "Server verwalten" to the new right "Mitglieder moderieren".
- A channel or voice room limited to roles is invisible to everyone else; the owner sees every channel.

## Screenshots on the landing page and in the README

The landing page shows `web/public/images/landing/app-en.png` and `app-de.png`, real screenshots of the app in the shape of `landing-preview-en.png`: Lena in the text channel while Jonas is live in Sofa. `web/src/public/landingImage.test.mjs` fails if either is the drawn preview. `docs/assets/readme-*.png` (overview, text channel, Sofa with a call) come from the same English server. To take them again, run a local stack as for the parity shots (API, the media service and Vite), then in `web/`:

```sh
node scripts/landing/seed.mjs en http://127.0.0.1:5180
node scripts/landing/seed.mjs de http://127.0.0.1:5180
node scripts/landing/shot.mjs en       # public/images/landing/app-en.png
node scripts/landing/shot.mjs de       # public/images/landing/app-de.png
node scripts/landing/shot.mjs readme   # ../docs/assets/readme-{hero,chat,voice}.png
```

`seed.mjs` makes accounts of its own per language (`lena.en@example.test`, …), so it can run next to the parity seed; it seeds once per database. `shot.mjs` sets Lena's account language, keeps Jonas live in Sofa with `scripts/parity/call.mjs` and writes the pictures at 1280×800 (the README's at device scale 1.5). Look at each picture before committing it.
