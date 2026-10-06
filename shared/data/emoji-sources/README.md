# Pinned emoji data

The server validates against the same canonical fully-qualified emoji set as
the lazy-loaded web picker. Display names and search terms use German CLDR.

- Unicode Emoji 18.0: https://unicode.org/Public/18.0.0/emoji/emoji-test.txt
- CLDR 48.2.3 German annotations: https://github.com/unicode-org/cldr-json/tree/48.2.3
- License: `LICENSE.txt` (Unicode License v3).

Run `python3 tools/generate-emoji-data.py` from any directory to reproduce
`shared/data/emoji-18.0.json`. The source corpus is data, not executable code.
Custom uploaded emoji are not supported by this version.

Use `--check` for a read-only parity/hash check (also run in CI).
