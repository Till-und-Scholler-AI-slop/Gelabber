# WebView localStorage fixtures

localStorage stores as the Tauri app (up to 0.8) left them, for the readers in
`desktop/ui/src/platform/webview/` and the import in `legacy.rs`.

- `page/`: the page that fills them (`index.html`, two phases: a table, a log,
  Snappy, an overwrite and deletions) and `serve.py`, its server. The
  end-to-end steps of `desktop-native.yml` use them too.
- `chromium/`: headless Chromium's LevelDB store, the format WebView2 keeps.
  Regenerate with `./capture-chromium.sh` (Docker, offline).
- `webkitgtk/`: the 0.8.9 Tauri app's WebKitGTK databases with their WAL, for
  two origins. Regenerate with
  `./capture-webkitgtk.sh <gelabber-desktop-linux-x64.tar.gz of v0.8.9>`
  (Docker; the image build downloads WebKitGTK, the capture runs offline).
