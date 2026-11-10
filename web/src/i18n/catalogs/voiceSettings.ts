// Audio & video settings: the settings dialog and page, the quick call
// settings, push to talk and the microphone test.
import { defineCatalog } from "../catalog.ts";

export const voiceSettings = defineCatalog(
  "voiceSettings",
  {
    "dialog.title": "Audio & Video",
    "tabs.label": "Bereiche",
    "tabs.audio": "Audio",
    "tabs.video": "Video & Live",
    "tabs.diagnostics": "Diagnose",
    "panel.loadFailed": "Die Einstellungen konnten nicht geladen werden.",
    percent: "{percent} %",
    "form.intro":
      "Passe Mikrofon, Lautsprecher und Kamera an. Die Übertragung hat standardmäßig keine Bitratengrenze durch Gelabber.",
    "form.applyNote":
      "Änderungen gelten beim nächsten Start und werden, soweit unterstützt, auch auf laufende Streams angewendet.",
    "input.title": "Eingabe",
    "input.microphone": "Mikrofon",
    "input.testMicrophone": "Mikrofon testen und vergleichen",
    "input.gain": "Mikrofonverstärkung",
    "input.gainHint":
      "100 % fügt keine Verstärkung hinzu. Andere Werte laufen über Web Audio.",
    "input.filter": "Filter",
    "output.title": "Ausgabe",
    "output.speaker": "Lautsprecher",
    "output.speakerUnsupported":
      "Dieser Browser kann den Lautsprecher nicht wählen. Der Ton folgt der Ausgabe deines Geräts.",
    "output.calls": "Gespräche",
    "output.callsHint":
      "Über 100 % wird der Ton digital verstärkt. Nutze dafür Kopfhörer: Über Lautsprecher können die anderen sonst ihr eigenes Echo hören.",
    "output.liveSound": "Live-Ton",
    "output.liveSoundHint":
      "Der Ton von Lives hat eine eigene Lautstärke. Taub schaltet auch diesen Ton aus.",
    "output.muteLiveSound": "Live-Ton stummschalten",
    "output.callSounds": "Signaltöne",
    "output.callSoundsHint":
      "Töne für Beitreten, Verlassen, Stumm, Taub, Kamera und Live.",
    "output.otherMuteSounds": "Töne, wenn andere sich stumm schalten",
    "output.callSoundVolume": "Lautstärke der Signaltöne",
    "output.callSoundVolumeSpeakerHint":
      "Nutzt deinen gewählten Lautsprecher und die Wiedergabelautstärke.",
    "output.callSoundVolumeHint": "Nutzt die Wiedergabelautstärke.",
    "output.testSound": "Testton abspielen",
    "advanced.title": "Echo und Browserfilter",
    "advanced.subtitle": "Für Musik oder Lautsprecher",
    "advanced.echo": "Echo",
    "advanced.echoHint":
      "Echo-Unterdrückung (AEC). Bei Lautsprechern empfohlen. Für Musik mit Kopfhörern kannst du sie ausschalten.",
    "advanced.noise": "Rauschfilter",
    "advanced.noiseHint": "Rauschunterdrückung des Browsers.",
    "advanced.autoGain": "Auto-Gain",
    "advanced.autoGainHint":
      "Auto-Gain des Browsers. Gleicht leise und laute Mikrofone auf einen hörbaren Pegel an.",
    "camera.title": "Kamera",
    "camera.device": "Gerät",
    "devices.revealMicFailed":
      "Der Browser gibt die Gerätenamen ohne Mikrofonzugriff nicht heraus.",
    "devices.revealMicHint":
      "Die Gerätenamen zeigt der Browser erst, wenn du den Mikrofonzugriff erlaubst.",
    "devices.revealMic": "Gerätenamen anzeigen",
    "devices.revealCameraFailed":
      "Der Browser gibt die Kameranamen ohne Kamerazugriff nicht heraus.",
    "devices.revealCameraHint":
      "Die Kameranamen zeigt der Browser erst, wenn du den Kamerazugriff erlaubst.",
    "devices.revealCamera": "Kameranamen anzeigen",
    "devices.default": "Standard",
    "devices.defaultNamed": "Standard – {name}",
    "devices.chosen": "Gewähltes Gerät",
    "quality.label": "Qualität",
    "quality.resolution": "Auflösung",
    "quality.resolutionAuto": "Auto (bis 1080p)",
    "quality.frameRate": "Bildrate",
    "quality.frameRateAuto": "Auto (bis 30 FPS)",
    "quality.auto": "Automatisch",
    "quality.appliedLive": "Auf den laufenden Stream angewendet.",
    "quality.appliesNext":
      "Bitrate und FPS-Limit gelten sofort. Die Auflösung gilt ab dem nächsten Stream.",
    "live.title": "Live",
    "live.qualityHint":
      "Höhere Werte brauchen mehr Rechenleistung und Upload. Die erreichbare Qualität hängt von Quelle, Gerät und Verbindung ab.",
    "live.soundLabel": "Ton im Live",
    "live.shareSound": "Ton mitsenden",
    "live.shareSoundHint":
      "Gilt ab deinem nächsten Live; Ausschalten wirkt sofort. Wähle den Ton im Browserdialog aus; je nach Browser und Quelle ist nur Video verfügbar. Der Live-Ton wird als Stereo-Musik übertragen, ohne Mikrofonfilter.",
    "upload.title": "Upload",
    "upload.noLimitBadge": "ohne Limit",
    "upload.economy": "Sparmodus",
    "upload.economyOn":
      "Ausdrückliche Obergrenzen: Sprache {voice} kbit/s, Live-Ton 128 kbit/s, Video gemeinsam {video}.",
    "upload.economyOff":
      "Für wenig Upload: Sprache {voice} kbit/s, Live-Ton 128 kbit/s, Video gemeinsam {video}.",
    "upload.videoLimit": "Video-Limit",
    "upload.economyLimit": "Sparmodus ({limit})",
    "upload.noLimit": "Ohne Limit",
    "upload.customLimit": "Eigenes Limit",
    "upload.customLimitUnit": "Mbit/s für alle Videos zusammen",
    "upload.customLimitLabel": "Eigenes gemeinsames Video-Limit (Mbit/s)",
    "upload.limitHint":
      "Der Browser passt die tatsächliche Bitrate an die Verbindung an. Das Limit ist eine Obergrenze, keine feste Datenrate.",
    "mode.enhanced": "Verbesserte Sprache",
    "mode.enhancedHint":
      "Lokale Rauschunterdrückung und Auto-Gain; Browserfilter als Ersatz",
    "mode.browser": "Browserfilter",
    "mode.browserHint": "Rauschunterdrückung und Auto-Gain deines Browsers",
    "mode.original": "Original / Musik",
    "mode.originalHint":
      "Stereo sofern unterstützt, ohne Rauschfilter, Auto-Gain oder Sprechpausen-Erkennung",
    "ptt.send": "Senden",
    "ptt.modeLabel": "Sprachübertragung",
    "ptt.activity": "Sprachaktivierung",
    "ptt.activityHint": "Das Mikrofon sendet, sobald du sprichst.",
    "ptt.ptt": "Push-to-Talk",
    "ptt.pttHint": "Das Mikrofon sendet nur, solange du die Taste hältst.",
    "ptt.key": "Taste",
    "ptt.recording": "Taste oder Maustaste drücken … (Esc bricht ab)",
    "ptt.setKey": "Taste festlegen",
    "ptt.noKeyHint": "Ohne Taste bleibt dein Mikrofon im Gespräch stumm.",
    "ptt.keyHint":
      "Im Browser wirkt die Taste nur, solange Gelabber im Vordergrund ist. Die Desktop-App hört sie auch im Hintergrund.",
    "ptt.releaseDelay": "Nachlaufzeit",
    "ptt.releaseDelayHint":
      "So lange sendet das Mikrofon nach dem Loslassen weiter.",
    "micTest.title": "Mikrofon testen",
    "micTest.intro": "Prüfe dein Mikrofon und vergleiche die Verarbeitung.",
    "micTest.stopped":
      "Test beendet. Mikrofon und Aufnahmen wurden freigegeben.",
    "micTest.local":
      "Der Test läuft lokal, ohne Übertragung. Aufnahmen dauern höchstens acht Sekunden und werden beim Schließen gelöscht.",
    "micTest.inputLevel": "Eingangspegel",
    "micTest.processed": "Nach Verarbeitung",
    "micTest.processing": "Verarbeitung",
    "micTest.clipping":
      "Übersteuerung erkannt. Verringere die Mikrofonverstärkung oder den Gerätepegel.",
    "micTest.stop": "Test beenden",
    "micTest.start": "Test starten",
    "micTest.stopRecording": "Aufnahme beenden",
    "micTest.startRecording": "A/B-Aufnahme starten",
    "micTest.recording": "Lokale Aufnahme läuft (maximal acht Sekunden).",
    "micTest.comparison": "Lokaler Audiovergleich",
    "micTest.inputClip": "Eingang (einschließlich Browser-Capture)",
    "micTest.deleteClips": "Aufnahmen löschen",
    "micTest.switchNote":
      "Ein Wechsel der Verarbeitung startet den Test neu und löscht die Aufnahmen.",
    "micTest.error.notAllowed":
      "Der Mikrofonzugriff wurde nicht erlaubt. Du kannst die Berechtigung in den Browser-Einstellungen ändern und es erneut versuchen.",
    "micTest.error.notFound":
      "Es wurde kein Mikrofon gefunden. Schließe ein Mikrofon an und versuche es erneut.",
    "micTest.error.notReadable":
      "Das Mikrofon ist gerade nicht verfügbar. Prüfe, ob eine andere Anwendung darauf zugreift.",
    "micTest.error.security":
      "Der Browser erlaubt hier keinen Mikrofonzugriff. Der Test benötigt eine sichere Verbindung oder localhost.",
    "micTest.error.unsupported":
      "Dein Browser unterstützt hier keinen Mikrofonzugriff. Nutze einen aktuellen Browser mit HTTPS oder localhost.",
    "micTest.error.ended":
      "Die Mikrofonverbindung wurde beendet. Du kannst den Test erneut starten.",
    "micTest.error.failed":
      "Der Mikrofontest konnte nicht gestartet werden. Prüfe dein Audiogerät und versuche es erneut.",
    "micTest.allowBrowser":
      "Bitte erlaube den lokalen Mikrofonzugriff im Browser.",
    "micTest.speak": "Sprich, um den Pegel zu prüfen.",
    "micTest.allow": "Bitte erlaube den lokalen Mikrofonzugriff.",
    "micTest.error.noiseFilter":
      "Rauschfilter ausgefallen. Beende den Test und starte ihn erneut.",
    "micTest.error.connectionEnded":
      "Mikrofonverbindung beendet. Starte den Test erneut.",
    "micTest.error.denied":
      "Mikrofonzugriff nicht erlaubt. Prüfe die Browserberechtigung.",
    "micTest.error.startFailed":
      "Mikrofontest konnte nicht starten. Prüfe Gerät und Berechtigung.",
    "micTest.recordingUnsupported":
      "Dieser Browser unterstützt keine lokale Aufnahme.",
    "micTest.recordingFailed":
      "Aufnahme fehlgeschlagen; Mikrofontest läuft weiter.",
    "micTest.recordingUnsupportedHere":
      "Lokale Aufnahme wird hier nicht unterstützt.",
    "quick.callSettings": "Anrufeinstellungen",
    "quick.volume": "Lautstärke",
    "quick.unmuteLiveSound": "Live-Ton wieder einschalten",
    "quick.deafened": "Taubgeschaltet.",
    "quick.micDenied": "Kein Zugriff aufs Mikrofon.",
    "quick.namesAfterPermission":
      "Namen zeigt der Browser erst nach deiner Erlaubnis.",
    "quick.noKey": "Keine Taste festgelegt. Das Mikrofon bleibt stumm.",
    "quick.allSettings": "Alle Einstellungen …",
    "quick.sendLevel": "Pegel, den die anderen hören",
  },
  {
    "dialog.title": "Audio & video",
    "tabs.label": "Sections",
    "tabs.audio": "Audio",
    "tabs.video": "Video & Live",
    "tabs.diagnostics": "Diagnostics",
    "panel.loadFailed": "The settings could not be loaded.",
    percent: "{percent}%",
    "form.intro":
      "Adjust your microphone, speakers and camera. By default, Gelabber puts no bitrate limit on what you send.",
    "form.applyNote":
      "Changes apply from the next start and, where supported, to running streams too.",
    "input.title": "Input",
    "input.microphone": "Microphone",
    "input.testMicrophone": "Test and compare microphone",
    "input.gain": "Microphone gain",
    "input.gainHint": "100% adds no gain. Other values go through Web Audio.",
    "input.filter": "Filter",
    "output.title": "Output",
    "output.speaker": "Speaker",
    "output.speakerUnsupported":
      "This browser can’t choose the speaker. Sound follows your device’s output.",
    "output.calls": "Calls",
    "output.callsHint":
      "Above 100%, sound is boosted digitally. Use headphones for that: through speakers, the others may hear their own echo.",
    "output.liveSound": "Live audio",
    "output.liveSoundHint":
      "Live audio has its own volume. Deafen turns it off too.",
    "output.muteLiveSound": "Mute Live audio",
    "output.callSounds": "Call sounds",
    "output.callSoundsHint":
      "Sounds for join, leave, mute, deafen, camera and Live.",
    "output.otherMuteSounds": "Sounds when others mute",
    "output.callSoundVolume": "Call sound volume",
    "output.callSoundVolumeSpeakerHint":
      "Uses your chosen speaker and the playback volume.",
    "output.callSoundVolumeHint": "Uses the playback volume.",
    "output.testSound": "Play test sound",
    "advanced.title": "Echo and browser filters",
    "advanced.subtitle": "For music or speakers",
    "advanced.echo": "Echo",
    "advanced.echoHint":
      "Echo cancellation (AEC). Recommended with speakers. For music on headphones, you can turn it off.",
    "advanced.noise": "Noise filter",
    "advanced.noiseHint": "The browser’s noise suppression.",
    "advanced.autoGain": "Auto gain",
    "advanced.autoGainHint":
      "The browser’s auto gain. Brings quiet and loud microphones to an audible level.",
    "camera.title": "Camera",
    "camera.device": "Device",
    "devices.revealMicFailed":
      "Your browser won’t share device names without microphone access.",
    "devices.revealMicHint":
      "Your browser shows device names once you allow microphone access.",
    "devices.revealMic": "Show device names",
    "devices.revealCameraFailed":
      "Your browser won’t share camera names without camera access.",
    "devices.revealCameraHint":
      "Your browser shows camera names once you allow camera access.",
    "devices.revealCamera": "Show camera names",
    "devices.default": "Default",
    "devices.defaultNamed": "Default – {name}",
    "devices.chosen": "Chosen device",
    "quality.label": "Quality",
    "quality.resolution": "Resolution",
    "quality.resolutionAuto": "Auto (up to 1080p)",
    "quality.frameRate": "Frame rate",
    "quality.frameRateAuto": "Auto (up to 30 FPS)",
    "quality.auto": "Automatic",
    "quality.appliedLive": "Applied to the running stream.",
    "quality.appliesNext":
      "Bitrate and FPS limit apply at once. Resolution applies from the next stream.",
    "live.title": "Live",
    "live.qualityHint":
      "Higher values need more processing power and upload. The quality you get depends on source, device and connection.",
    "live.soundLabel": "Sound in a Live",
    "live.shareSound": "Share sound",
    "live.shareSoundHint":
      "Applies from your next Live; turning it off works at once. Pick the sound in the browser dialog; depending on browser and source, only video may be available. Live audio is sent as stereo music, without microphone filters.",
    "upload.title": "Upload",
    "upload.noLimitBadge": "no limit",
    "upload.economy": "Data saver",
    "upload.economyOn":
      "Explicit limits: voice {voice} kbit/s, Live audio 128 kbit/s, video {video} combined.",
    "upload.economyOff":
      "For low upload: voice {voice} kbit/s, Live audio 128 kbit/s, video {video} combined.",
    "upload.videoLimit": "Video limit",
    "upload.economyLimit": "Data saver ({limit})",
    "upload.noLimit": "No limit",
    "upload.customLimit": "Custom limit",
    "upload.customLimitUnit": "Mbit/s for all videos combined",
    "upload.customLimitLabel": "Custom combined video limit (Mbit/s)",
    "upload.limitHint":
      "The browser adapts the actual bitrate to the connection. The limit is a cap, not a fixed rate.",
    "mode.enhanced": "Enhanced voice",
    "mode.enhancedHint":
      "Local noise suppression and auto gain; browser filters as a fallback",
    "mode.browser": "Browser filters",
    "mode.browserHint": "Your browser’s noise suppression and auto gain",
    "mode.original": "Original / music",
    "mode.originalHint":
      "Stereo where supported, without noise filter, auto gain or silence detection",
    "ptt.send": "Send",
    "ptt.modeLabel": "Voice transmission",
    "ptt.activity": "Voice activity",
    "ptt.activityHint": "Your microphone sends as soon as you speak.",
    "ptt.ptt": "Push to talk",
    "ptt.pttHint": "Your microphone sends only while you hold the key.",
    "ptt.key": "Key",
    "ptt.recording": "Press a key or mouse button… (Esc cancels)",
    "ptt.setKey": "Set key",
    "ptt.noKeyHint": "Without a key, your microphone stays muted in calls.",
    "ptt.keyHint":
      "In the browser, the key only works while Gelabber is in front. The desktop app hears it in the background too.",
    "ptt.releaseDelay": "Release delay",
    "ptt.releaseDelayHint":
      "How long your microphone keeps sending after you let go.",
    "micTest.title": "Test microphone",
    "micTest.intro": "Check your microphone and compare the processing.",
    "micTest.stopped":
      "Test ended. Microphone and recordings have been released.",
    "micTest.local":
      "The test runs locally and sends nothing. Recordings last eight seconds at most and are deleted when you close it.",
    "micTest.inputLevel": "Input level",
    "micTest.processed": "After processing",
    "micTest.processing": "Processing",
    "micTest.clipping":
      "Clipping detected. Lower the microphone gain or the device level.",
    "micTest.stop": "Stop test",
    "micTest.start": "Start test",
    "micTest.stopRecording": "Stop recording",
    "micTest.startRecording": "Start A/B recording",
    "micTest.recording": "Recording locally (eight seconds at most).",
    "micTest.comparison": "Local audio comparison",
    "micTest.inputClip": "Input (including browser capture)",
    "micTest.deleteClips": "Delete recordings",
    "micTest.switchNote":
      "Switching the processing restarts the test and deletes the recordings.",
    "micTest.error.notAllowed":
      "Microphone access was not allowed. You can change the permission in your browser settings and try again.",
    "micTest.error.notFound":
      "No microphone was found. Connect a microphone and try again.",
    "micTest.error.notReadable":
      "The microphone isn’t available right now. Check whether another app is using it.",
    "micTest.error.security":
      "Your browser doesn’t allow microphone access here. The test needs a secure connection or localhost.",
    "micTest.error.unsupported":
      "Your browser doesn’t support microphone access here. Use a current browser with HTTPS or localhost.",
    "micTest.error.ended":
      "The microphone connection ended. You can start the test again.",
    "micTest.error.failed":
      "The microphone test could not start. Check your audio device and try again.",
    "micTest.allowBrowser":
      "Please allow local microphone access in your browser.",
    "micTest.speak": "Speak to check the level.",
    "micTest.allow": "Please allow local microphone access.",
    "micTest.error.noiseFilter":
      "Noise filter failed. Stop the test and start it again.",
    "micTest.error.connectionEnded":
      "Microphone connection ended. Start the test again.",
    "micTest.error.denied":
      "Microphone access not allowed. Check the browser permission.",
    "micTest.error.startFailed":
      "The microphone test couldn’t start. Check the device and permission.",
    "micTest.recordingUnsupported":
      "This browser doesn’t support local recording.",
    "micTest.recordingFailed":
      "Recording failed; the microphone test keeps running.",
    "micTest.recordingUnsupportedHere": "Local recording isn’t supported here.",
    "quick.callSettings": "Call settings",
    "quick.volume": "Volume",
    "quick.unmuteLiveSound": "Unmute Live audio",
    "quick.deafened": "Deafened.",
    "quick.micDenied": "No access to the microphone.",
    "quick.namesAfterPermission":
      "Your browser shows names once you allow access.",
    "quick.noKey": "No key set. Your microphone stays muted.",
    "quick.allSettings": "All settings…",
    "quick.sendLevel": "Level the others hear",
  },
);
