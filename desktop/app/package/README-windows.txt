Gelabber Desktop (Windows x64)

Installieren
  gelabber-desktop-windows-x64-setup.exe ausführen. Das Setup braucht keine
  Administratorrechte: Es installiert für das angemeldete Konto nach
  %LOCALAPPDATA%\Gelabber und legt einen Eintrag im Startmenü an.

  Die Datei ist nicht signiert. Windows SmartScreen meldet deshalb beim
  Start des Setups "Der Computer wurde durch Windows geschützt". Dort auf
  "Weitere Informationen" klicken, dann auf "Trotzdem ausführen". Auch der
  Browser kann beim Herunterladen nachfragen, ob die Datei behalten werden
  soll. Das Setup nur von der Release-Seite laden:
    https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases

Voraussetzungen
  - Windows 10 oder 11 (64 Bit, x64).
  - Microsoft Edge WebView2 Runtime. Windows 11 bringt sie mit; fehlt sie,
    lädt das Setup sie nach und braucht dafür eine Internetverbindung.
  - Ein Gelabber-Server ab Version 0.6. Ältere Server bieten in der App auch
    das an, was unter Windows noch fehlt (siehe unten); es schlägt dort fehl.

Starten
  Über das Startmenü ("Gelabber"). Beim ersten Start die Adresse des Servers
  eingeben; die App merkt sie sich. Oder mit Adresse starten:
    "%LOCALAPPDATA%\Gelabber\gelabber-desktop.exe" --server https://dein-gelabber-server
  Strg+Umschalt+S führt zurück zur Serverauswahl, F5 lädt die Seite neu.

Noch nicht verfügbar unter Windows
  - Bildschirm teilen
  - Go Live senden
  - Anwendungston teilen (Ton anderer Programme)
  Bildschirme und Go-Live-Streams anderer lassen sich ansehen. Ein Server ab
  Version 0.6 blendet die fehlenden Funktionen in der App aus.

Einstellungen und Daten
  Serveradresse:
    %APPDATA%\io.github.till-und-scholler-ai-slop.gelabber\desktop.json
  WebView2-Profil (Anmeldung, Cookies, Cache):
    %LOCALAPPDATA%\io.github.till-und-scholler-ai-slop.gelabber\EBWebView

Aktualisieren
  Die App aktualisiert sich nicht selbst. Gelabber beenden, das Setup der
  neuen Version von der Release-Seite laden und ausführen: Es ersetzt die
  installierte Version. Serveradresse und Anmeldung bleiben erhalten.

Entfernen
  Einstellungen > Apps > Installierte Apps (Windows 10: Apps & Features) >
  Gelabber > Deinstallieren. Wer dabei das Löschen der Anwendungsdaten
  ankreuzt, entfernt auch Serveradresse und Anmeldung, also die beiden
  Ordner oben.

Fehlersuche
  Protokoll der Medienschicht, in der Eingabeaufforderung (cmd.exe):
    set GELABBER_MEDIA_LOG=1
    "%LOCALAPPDATA%\Gelabber\gelabber-desktop.exe" 2> "%USERPROFILE%\gelabber.log"

Lizenzen
  Hinweise zu den enthaltenen Bibliotheken: THIRD-PARTY-NOTICES.txt im
  Installationsordner.
