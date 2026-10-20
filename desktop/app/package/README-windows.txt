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
  - Ein Gelabber-Server, empfohlen ab Version 0.6. Mit einem älteren
    Server kann ein abgebrochenes oder fehlgeschlagenes "Go Live" (etwa
    die Auswahl geschlossen) den Kanal für Go Live belegt lassen.
  - Für "Bildschirm teilen" und "Go Live": Windows 10 ab Version 1903.
  - Für den Anwendungston: Windows 10 ab Version 2004.

Starten
  Über das Startmenü ("Gelabber"). Beim ersten Start die Adresse des Servers
  eingeben; die App merkt sie sich. Oder mit Adresse starten:
    "%LOCALAPPDATA%\Gelabber\gelabber-desktop.exe" --server https://dein-gelabber-server
  Strg+Umschalt+S führt zurück zur Serverauswahl, F5 lädt die Seite neu.

Bildschirm teilen und Go Live
  Beide öffnen die Auswahl von Windows: Dort einen Bildschirm oder ein
  Fenster wählen. Erst danach nimmt die App etwas auf; wer die Auswahl
  schließt, teilt nichts. Solange aufgenommen wird, rahmt Windows das
  Geteilte gelb ein. Wird ein geteiltes Fenster geschlossen, endet die
  Freigabe.

Anwendungston
  Beim Teilen und bei Go Live kann der Ton anderer Programme mitgehen:
  entweder der Ton aller Programme oder der eines gewählten Programms. Zur
  Wahl stehen die Programme, die gerade Ton abspielen; ein Programm, das
  erst später zu spielen beginnt, wird innerhalb einer Sekunde
  aufgenommen. Gelabber selbst, also der Ton des Gesprächs, wird nie
  mitgeschickt. Unter Windows vor Version 2004 geht die Freigabe ohne Ton
  hinaus.

Push-to-Talk
  In den Spracheinstellungen "Push-to-Talk" wählen und eine Taste oder
  eine seitliche Maustaste festlegen. Im Gespräch hört die App die Taste
  auch, wenn ein anderes Fenster vorne ist; die Taste erreicht dieses
  Fenster trotzdem. Ausnahme: Läuft das Programm vorne mit
  Administratorrechten, lässt Windows die App die Taste nicht sehen.

Einstellungen und Daten
  Serveradresse:
    %APPDATA%\io.github.till-und-scholler-ai-slop.gelabber\desktop.json
  Automatische Update-Suche (an oder aus):
    %APPDATA%\io.github.till-und-scholler-ai-slop.gelabber\update.json
  WebView2-Profil (Anmeldung, Cookies, Cache):
    %LOCALAPPDATA%\io.github.till-und-scholler-ai-slop.gelabber\EBWebView

Aktualisieren
  Die App sucht kurz nach dem Start und danach alle 6 Stunden nach einer
  neuen Version auf der Release-Seite. Gibt es eine, erscheint oben in der
  Mitte "Gelabber <Version> ist da" mit "Neu starten" und "Später". "Neu
  starten" fragt noch einmal in einem Fenster von Windows nach (Ja/Nein,
  vorausgewählt ist Nein). Nach "Ja" lädt die App das Setup, prüft seine
  Signatur, installiert es und startet neu; ein Setup mit falscher oder
  fehlender Signatur wird nicht ausgeführt. Während eines Anrufs erscheint der Hinweis nicht und die App
  startet nicht neu; er kommt nach dem Auflegen. Serveradresse und
  Anmeldung bleiben erhalten.

  Die automatische Suche lässt sich auf der Serverauswahl (Strg+Umschalt+S)
  abschalten, dort lässt sich auch sofort suchen. Von Hand geht es wie
  bisher: Gelabber beenden, das Setup der neuen Version von der
  Release-Seite laden und ausführen.

Entfernen
  Einstellungen > Apps > Installierte Apps (Windows 10: Apps & Features) >
  Gelabber > Deinstallieren. Wer dabei das Löschen der Anwendungsdaten
  ankreuzt, entfernt auch Serveradresse, Update-Einstellung und Anmeldung,
  also die beiden Ordner oben.

Fehlersuche
  Protokoll der Medienschicht, in der Eingabeaufforderung (cmd.exe):
    set GELABBER_MEDIA_LOG=1
    "%LOCALAPPDATA%\Gelabber\gelabber-desktop.exe" > "%USERPROFILE%\gelabber.log" 2>&1
  Beide Umleitungen sind nötig: Ein Teil des Protokolls geht auf die
  Standardausgabe, der andere auf die Fehlerausgabe.

  Kleines Video: Meldet eine Videokachel "Geringe Bildqualität" und nennt
  beim Darüberfahren die Content-Security-Policy des Servers, dann versperrt
  diese Richtlinie der App den schnellen Weg für Bilder. Wer den Server
  betreibt, muss in connect-src zusätzlich ipc: und http://ipc.localhost
  erlauben; siehe deploy/README.md im Gelabber-Repository.

Lizenzen
  THIRD-PARTY-NOTICES.txt im Installationsordner (englisch) nennt die
  enthaltene Software Dritter mit ihren Lizenzen und Lizenztexten. Teil 1
  der Datei sagt auch, was für die Windows-Version noch nicht erfasst ist.
