Gelabber, native Desktop-App (Windows x64, Beta)

Installieren
  gelabber-native-windows-x64-setup.exe ausführen. Das Setup braucht keine
  Administratorrechte: Es installiert für das angemeldete Konto nach
  %LOCALAPPDATA%\Gelabber, also an die Stelle der bisherigen Desktop-App
  (Tauri), und legt Einträge im Startmenü und auf dem Desktop an. Eine
  installierte Tauri-App wird dabei ersetzt; ihre Serveradresse bleibt.

  Die Datei ist nicht signiert. Windows SmartScreen meldet deshalb beim
  Start des Setups "Der Computer wurde durch Windows geschützt". Dort auf
  "Weitere Informationen" klicken, dann auf "Trotzdem ausführen". Das Setup
  nur von der Release-Seite laden:
    https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases

  Zurück zur Tauri-App: deren Setup (gelabber-desktop-windows-x64-setup.exe)
  von der Release-Seite ausführen.

Voraussetzungen
  - Windows 10 oder 11 (64 Bit, x64). Eine WebView2 Runtime braucht die
    App nicht.
  - Für "Live gehen": Windows 10 ab Version 1903.

Starten
  Über das Startmenü ("Gelabber"). Beim ersten Start die Adresse des Servers
  eingeben; die App merkt sie sich. Oder mit Adresse starten:
    "%LOCALAPPDATA%\Gelabber\gelabber.exe" --server https://dein-gelabber-server

Einstellungen und Daten
  %APPDATA%\io.github.till-und-scholler-ai-slop.gelabber\
  (desktop.json mit der Serveradresse teilt sie mit der Tauri-App). Beim
  ersten Start übernimmt die App die von der Tauri-App exportierten
  Einstellungen (legacy-settings.json dort). Die Anmeldung wird nicht
  übernommen: einmal neu anmelden.

Aktualisieren
  Die App sucht kurz nach dem Start und danach alle 6 Stunden nach einer
  neuen Version (latest-native.json der neuesten Version auf der
  Release-Seite). Gibt es eine, erscheint "Gelabber <Version> ist da. Zum
  Installieren startet die App neu." mit "Neu starten" und "Später".
  "Neu starten" lädt das Setup, prüft seine Signatur, beendet die App und
  installiert ohne Rückfragen in denselben Ordner; danach startet die neue
  Version. Während eines Anrufs erscheint der Hinweis nicht, ein schon
  geladenes Update wartet bis nach dem Auflegen.

Entfernen
  Einstellungen > Apps > Installierte Apps > Gelabber > Deinstallieren.
  Einstellungen und Anmeldung bleiben, außer man bestätigt ihr Löschen.

Fehlersuche
  In der Eingabeaufforderung (cmd.exe):
    set RUST_LOG=info
    set GELABBER_MEDIA_LOG=1
    "%LOCALAPPDATA%\Gelabber\gelabber.exe" > "%USERPROFILE%\gelabber.log" 2>&1

Lizenzen
  THIRD-PARTY-NOTICES.txt im Installationsordner (englisch) nennt die
  enthaltene Software Dritter, Schriften und Daten mit ihren Lizenzen und
  Lizenztexten.
