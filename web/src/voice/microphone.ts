export type MicrophoneTestState = {
  phase: "idle" | "pending" | "active" | "error";
  message: string;
};

type MicrophoneEnvironment = {
  mediaDevices?: Pick<MediaDevices, "getUserMedia">;
  AudioContextClass?: typeof AudioContext;
  requestFrame: typeof requestAnimationFrame;
  cancelFrame: typeof cancelAnimationFrame;
};

const errorMessages: Record<string, string> = {
  NotAllowedError:
    "Der Mikrofonzugriff wurde nicht erlaubt. Du kannst die Berechtigung in den Browser-Einstellungen ändern und es erneut versuchen.",
  NotFoundError:
    "Es wurde kein Mikrofon gefunden. Schließe ein Mikrofon an und versuche es erneut.",
  NotReadableError:
    "Das Mikrofon ist gerade nicht verfügbar. Prüfe, ob eine andere Anwendung darauf zugreift.",
  SecurityError:
    "Der Browser erlaubt hier keinen Mikrofonzugriff. Der Test benötigt eine sichere Verbindung oder localhost.",
};

/** Local, disposable meter. A late permission result never revives a closed test. */
export function createMicrophoneSession(
  {
    onState,
    onLevel,
  }: {
    onState: (state: MicrophoneTestState) => void;
    onLevel: (level: number) => void;
  },
  environment: MicrophoneEnvironment = {
    mediaDevices: navigator.mediaDevices,
    AudioContextClass: window.AudioContext,
    requestFrame: window.requestAnimationFrame.bind(window),
    cancelFrame: window.cancelAnimationFrame.bind(window),
  },
  audio: MediaTrackConstraints | boolean = true,
) {
  let disposed = false;
  let started = false;
  let stream: MediaStream | undefined;
  let context: AudioContext | undefined;
  let source: MediaStreamAudioSourceNode | undefined;
  let analyser: AnalyserNode | undefined;
  let frame: number | undefined;
  let lastUpdate = 0;
  const endedListeners: Array<[MediaStreamTrack, () => void]> = [];

  function stop() {
    if (disposed) return;
    disposed = true;
    if (frame !== undefined) environment.cancelFrame(frame);
    for (const [track, listener] of endedListeners)
      track.removeEventListener("ended", listener);
    source?.disconnect();
    analyser?.disconnect();
    stream?.getTracks().forEach((track) => track.stop());
    if (context && context.state !== "closed")
      void context.close().catch(() => {});
  }

  async function start() {
    if (started || disposed) return;
    started = true;
    if (!environment.mediaDevices?.getUserMedia) {
      onState({
        phase: "error",
        message:
          "Dein Browser unterstützt hier keinen Mikrofonzugriff. Nutze einen aktuellen Browser mit HTTPS oder localhost.",
      });
      return;
    }
    onState({
      phase: "pending",
      message: "Bitte erlaube den lokalen Mikrofonzugriff im Browser.",
    });
    try {
      // Resume in the explicit click gesture, before waiting for permission.
      if (environment.AudioContextClass) {
        context = new environment.AudioContextClass();
        void context.resume().catch(() => {});
      }
      stream = await environment.mediaDevices.getUserMedia({
        audio,
        video: false,
      });
      if (disposed) {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
      for (const track of stream.getTracks()) {
        const onEnded = () => {
          if (!disposed)
            onState({
              phase: "error",
              message:
                "Die Mikrofonverbindung wurde beendet. Du kannst den Test erneut starten.",
            });
          stop();
        };
        track.addEventListener("ended", onEnded);
        endedListeners.push([track, onEnded]);
      }
      if (!context) throw new Error("AudioContext unavailable");
      if (context.state === "suspended") await context.resume();
      if (disposed) return;
      if (context.state !== "running")
        throw new Error("AudioContext not running");
      analyser = context.createAnalyser();
      analyser.fftSize = 1024;
      source = context.createMediaStreamSource(stream);
      source.connect(analyser);
      // No destination connection: no playback, recording or transmission.
      const meter = analyser;
      const values = new Uint8Array(meter.fftSize);
      onState({
        phase: "active",
        message:
          "Sprich ein paar Worte. Der Pegel zeigt, ob dein Mikrofon dich hört.",
      });
      function measure(time: number) {
        if (disposed) return;
        if (time - lastUpdate >= 80) {
          meter.getByteTimeDomainData(values);
          const energy = values.reduce(
            (sum, sample) => sum + ((sample - 128) / 128) ** 2,
            0,
          );
          onLevel(
            Math.min(100, Math.round(Math.sqrt(energy / values.length) * 350)),
          );
          lastUpdate = time;
        }
        frame = environment.requestFrame(measure);
      }
      frame = environment.requestFrame(measure);
    } catch (error) {
      if (!disposed) {
        const name = error instanceof Error ? error.name : "";
        onState({
          phase: "error",
          message:
            errorMessages[name] ??
            "Der Mikrofontest konnte nicht gestartet werden. Prüfe dein Audiogerät und versuche es erneut.",
        });
        stop();
      }
    }
  }
  return { start, stop };
}
