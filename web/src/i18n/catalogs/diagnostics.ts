// Voice diagnostics: the connection's status and measurements, the
// Diagnose tab, the downloadable report, and the audio processing's state
// as the settings and the diagnostics show it.
import { defineCatalog } from "../catalog.ts";

export const diagnostics = defineCatalog(
  "diagnostics",
  {
    "processing.inactive": "Mikrofon inaktiv",
    "processing.enhanced": "Lokale Rauschunterdrückung aktiv",
    "processing.browser": "Browserfilter aktiv",
    "processing.original": "Original / Musik aktiv",
    "processing.browserNoFilters":
      "Browser-Modus aktiv; Browser meldet keine Rauschfilter",
    "processing.resuming": "Audioverarbeitung pausiert; Wiederaufnahme läuft",
    "processing.processorFailed":
      "Audioprozessor ausgefallen; {state} (Mikrofonverstärkung 100 %)",
    "processing.noiseFilterUnsupported":
      "Lokaler Rauschfilter nicht unterstützt; {state}",
    "processing.noiseFilterUnavailable":
      "Lokaler Rauschfilter nicht verfügbar; {state} (Mikrofonverstärkung 100 %)",
    "processing.unavailable":
      "Audioverarbeitung nicht verfügbar; {state} (Mikrofonverstärkung 100 %)",
    "processing.micLost": "Mikrofon getrennt; Standardgerät wird aktiviert",
    "processing.processorFallback":
      "Audioprozessor ausgefallen; Ersatzmikrofon wird aktiviert",
    "processing.micFailed":
      "Mikrofon konnte nicht aktiviert werden; du hörst weiter zu",
    "processing.error.modelStalled": "Rauschfilter lädt nicht weiter",
    "processing.error.modelLoad": "Rauschfilter konnte nicht geladen werden",
    "processing.error.modelChecksum": "Rauschfilter-Prüfsumme stimmt nicht",
    "processing.error.ended": "Audioverarbeitung beendet",
    "processing.error.resume":
      "Audioverarbeitung konnte nicht fortgesetzt werden",
    "processing.error.paused": "Audioverarbeitung pausiert",
    "processing.error.pausedAtStart": "Audioverarbeitung pausiert beim Start",
    "processing.error.noiseFilterUnsupported":
      "Lokaler Rauschfilter wird nicht unterstützt",
    "processing.error.workletTimeout": "Rauschfilter startet nicht",
    "processing.error.workletFailed": "Rauschfilter konnte nicht starten",
    "processing.error.cancelled": "Mikrofonanfrage abgebrochen",
    "processing.error.cancelledOrFailed":
      "Mikrofonanfrage abgebrochen oder Audioprozessor ausgefallen",
  },
  {
    "processing.inactive": "Microphone inactive",
    "processing.enhanced": "Local noise suppression on",
    "processing.browser": "Browser filters on",
    "processing.original": "Original / music on",
    "processing.browserNoFilters":
      "Browser mode on; the browser reports no noise filters",
    "processing.resuming": "Audio processing paused; resuming",
    "processing.processorFailed":
      "Audio processor failed; {state} (microphone gain 100%)",
    "processing.noiseFilterUnsupported":
      "Local noise filter not supported; {state}",
    "processing.noiseFilterUnavailable":
      "Local noise filter unavailable; {state} (microphone gain 100%)",
    "processing.unavailable":
      "Audio processing unavailable; {state} (microphone gain 100%)",
    "processing.micLost": "Microphone disconnected; switching to the default",
    "processing.processorFallback":
      "Audio processor failed; switching to a fallback microphone",
    "processing.micFailed":
      "Microphone could not be turned on; you can still listen",
    "processing.error.modelStalled": "Noise filter stopped loading",
    "processing.error.modelLoad": "Noise filter could not be loaded",
    "processing.error.modelChecksum": "Noise filter checksum doesn’t match",
    "processing.error.ended": "Audio processing ended",
    "processing.error.resume": "Audio processing could not resume",
    "processing.error.paused": "Audio processing paused",
    "processing.error.pausedAtStart": "Audio processing paused at start",
    "processing.error.noiseFilterUnsupported":
      "Local noise filter isn’t supported",
    "processing.error.workletTimeout": "Noise filter doesn’t start",
    "processing.error.workletFailed": "Noise filter could not start",
    "processing.error.cancelled": "Microphone request cancelled",
    "processing.error.cancelledOrFailed":
      "Microphone request cancelled or audio processor failed",
  },
);
