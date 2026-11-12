// Installing Gelabber: the web app (Einstellungen → App, and the disclosure
// where signing in comes first) and the desktop app for Windows and Linux.
// Web only; the native app has no install page. Menu and dialog names in
// quotes are the ones the browser or system shows in that language.
import { defineCatalog } from "../catalog.ts";

export const pwa = defineCatalog(
  "pwa",
  {
    "desktop.label": "Desktop-App",
    "desktop.title": "Desktop-App für {platform}",
    "desktop.pushToTalk":
      "Push-to-Talk wirkt auch, wenn Gelabber im Hintergrund ist.",
    "desktop.other": "Für {platform}",
    "desktop.server":
      "Beim ersten Start fragt die App nach dem Server: <code>{server}</code>",
    "windows.download": "Für Windows herunterladen",
    "windows.run":
      "Starte die heruntergeladene Datei. Zeigt Windows „Der Computer wurde durch Windows geschützt“, wähle „Weitere Informationen“ und dann „Trotzdem ausführen“.",
    "windows.updates": "Neue Versionen installiert die App danach selbst.",
    "arch.title": "Arch Linux und Omarchy",
    "arch.terminal": "Im Terminal:",
    "arch.body":
      "Das richtet die Paketquelle von Gelabber ein und installiert die App. Updates kommen mit <code>sudo pacman -Syu</code> (oder <code>yay -Syu</code>) zusammen mit dem Rest des Systems.",
    "linux.title": "Andere Distributionen",
    "linux.body":
      "<link>gelabber-native-linux-x64.tar.gz</link> herunterladen, entpacken und darin <code>./gelabber</code> starten. Neue Versionen meldet die App; entpacke sie dann über die alte.",
    "offer.text": "Gelabber gibt es auch als Desktop-App für {platform}.",
    "offer.view": "Ansehen",

    "install.disclosure": "Gelabber als App installieren",
    "install.label": "Gelabber installieren",
    "install.title": "Gelabber als App",
    "install.mobile": "Direkt vom Startbildschirm öffnen.",
    "install.desktop": "In einem eigenen Fenster öffnen.",
    "install.installed":
      "Gelabber ist bereits als App geöffnet oder installiert.",
    "install.insecure":
      "Öffne Gelabber über HTTPS, damit du es als App installieren kannst.",
    "install.button": "App installieren",
    "install.opening": "Installation öffnen…",
    "install.accepted":
      "Installation bestätigt. Dein Browser richtet die App ein.",
    "install.failed":
      "Die Installation konnte nicht geöffnet werden. Nutze das Menü deines Browsers oder versuche es später erneut.",
    "ios.safari": "Öffne Gelabber in Safari.",
    "ios.share": "Tippe auf „Teilen“, dann auf „Zum Home-Bildschirm“.",
    "ios.add":
      "Aktiviere „Als Web-App öffnen“, falls angezeigt, und tippe auf „Hinzufügen“.",
    "mobile.alsoMenu":
      "Du kannst auch im Browsermenü „App installieren“ oder „Zum Startbildschirm hinzufügen“ wählen.",
    "mobile.menu":
      "Wähle im Browsermenü „App installieren“ oder „Zum Startbildschirm hinzufügen“. Bietet dein Browser das nicht an, öffne Gelabber in Chrome.",
    "desktopBrowser.where":
      "Die Installation findest du in der Adressleiste oder im Browsermenü, z. B. in Chrome oder Edge.",
    "install.online": "Die App braucht weiterhin eine Internetverbindung.",
  },
  {
    "desktop.label": "Desktop app",
    "desktop.title": "Desktop app for {platform}",
    "desktop.pushToTalk":
      "Push to talk works even when Gelabber is in the background.",
    "desktop.other": "For {platform}",
    "desktop.server":
      "On first start, the app asks for the server: <code>{server}</code>",
    "windows.download": "Download for Windows",
    "windows.run":
      "Run the downloaded file. If Windows shows “Windows protected your PC”, choose “More info” and then “Run anyway”.",
    "windows.updates": "After that, the app installs new versions by itself.",
    "arch.title": "Arch Linux and Omarchy",
    "arch.terminal": "In a terminal:",
    "arch.body":
      "This sets up Gelabber’s package repository and installs the app. Updates come with <code>sudo pacman -Syu</code> (or <code>yay -Syu</code>) along with the rest of the system.",
    "linux.title": "Other distributions",
    "linux.body":
      "Download <link>gelabber-native-linux-x64.tar.gz</link>, unpack it and run <code>./gelabber</code> inside. The app tells you about new versions; unpack them over the old one.",
    "offer.text": "Gelabber is also available as a desktop app for {platform}.",
    "offer.view": "View",

    "install.disclosure": "Install Gelabber as an app",
    "install.label": "Install Gelabber",
    "install.title": "Gelabber as an app",
    "install.mobile": "Open it straight from your home screen.",
    "install.desktop": "Open it in a window of its own.",
    "install.installed": "Gelabber is already open or installed as an app.",
    "install.insecure": "Open Gelabber over HTTPS to install it as an app.",
    "install.button": "Install app",
    "install.opening": "Opening installation…",
    "install.accepted":
      "Installation confirmed. Your browser is setting up the app.",
    "install.failed":
      "The installation could not be opened. Use your browser’s menu or try again later.",
    "ios.safari": "Open Gelabber in Safari.",
    "ios.share": "Tap “Share”, then “Add to Home Screen”.",
    "ios.add": "Turn on “Open as Web App” if shown, and tap “Add”.",
    "mobile.alsoMenu":
      "You can also choose “Install app” or “Add to Home screen” in the browser menu.",
    "mobile.menu":
      "Choose “Install app” or “Add to Home screen” in the browser menu. If your browser doesn’t offer that, open Gelabber in Chrome.",
    "desktopBrowser.where":
      "You’ll find the installation in the address bar or the browser menu, e.g. in Chrome or Edge.",
    "install.online": "The app still needs an internet connection.",
  },
);
