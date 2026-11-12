# Texts for the public legal pages

The `web` service mounts this folder read-only at `/etc/gelabber/legal`. At
every start it builds the privacy and terms pages, and extra Impressum
sections, from the files below. Gelabber ships no sample texts: the operator
writes their own. `.gitignore` keeps `*.html` in this folder out of commits.

| File | Page |
|---|---|
| `privacy.de.html`, `privacy.en.html` | Datenschutz (`/de/datenschutz`) / Privacy (`/en/privacy`) |
| `terms.de.html`, `terms.en.html` | Nutzungsbedingungen (`/de/nutzungsbedingungen`) / Terms (`/en/terms`) |
| `imprint.de.html`, `imprint.en.html` | Optional sections appended to the Impressum, for example a register entry or a VAT ID |

- **Format:** UTF-8 HTML fragments, without `<html>`, `<head>` or `<body>`.
  The page brings its own `h1`, so headings start at `h2`; use paragraphs,
  lists and links. The text is inserted as it is.
- **What the page allows:** no scripts, no inline styles and no pictures from
  other sites (the pages' Content-Security-Policy blocks them).
- **No file in either language:** the page is not published, no link to it
  appears, and the web container's log names the missing files.
- **A file in one language only:** the text is shown under both languages'
  addresses; the other language's page says that the text is only available
  in this language.
- **The Impressum itself** comes from the `GELABBER_OPERATOR_*` values in
  `.env`, not from a file. Without name, street, city and email it is not
  published.

The pages are built only at start: after changing a file, run
`docker compose restart web` (a changed value in `.env` needs
`docker compose up -d web` instead), then check
`docker compose logs web` for the lines of `50-gelabber-public-pages.sh`.
More in `deploy/README.md` ("Public pages").
