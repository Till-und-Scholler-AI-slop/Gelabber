// The call's controls: microphone, headset, camera, "Live gehen", the
// volumes while a Live plays and the settings, as icon buttons. Each toggle keeps one name and says its state
// through aria-pressed; the icon shows the state, and media that is on
// sits on a pill: the camera accent, a Live red. The click flips local
// state first. Leaving is the session's own button.
//
// Camera and Live start only where this client can capture
// (voice/capabilities.ts), Live only with the server's go_live. Stopping is
// always offered. Whether a Live carries sound is decided in its menu.

import {
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import {
  CameraIcon,
  CameraOffIcon,
  ChevronIcon,
  GearIcon,
  HeadsetIcon,
  HeadsetOffIcon,
  LiveIcon,
  MicIcon,
  MicOffIcon,
} from "./Icons.tsx";
import { Button } from "./ui/Button.tsx";
import { Popover } from "./ui/Popover.tsx";
import {
  retryMicrophone,
  toggleCamera,
  toggleDeafen,
  toggleMute,
  toggleShare,
  useVoice,
} from "../voice/session.ts";
import { useCapabilities, type Capabilities } from "../voice/capabilities.ts";
import {
  CallSoundButton,
  QuickCallSettingsButton,
} from "../voice/QuickCallSettings.tsx";
import { isDesktopApp } from "../voice/native/bridge.ts";
import {
  callShortcutKeys,
  callShortcutLabel,
  holdToTalk,
  usePushToTalk,
} from "../voice/pushToTalk.ts";
import {
  SOURCE_AUDIO_CARRIES_CALL,
  sharesSourceAudio,
  sourceAudioCarriesCall,
  sourceAudioChoice,
  sourceAudioScope,
  useMediaSettings,
} from "../voice/settings.ts";
import { notify } from "./toasts.ts";
import "../voice/room.css";

const BUTTON = "voice-control voice-control-compact";

/** Mute and deafen, for the call bar and wherever the call is watched. */
export function VoiceAudioToggles({
  className = BUTTON,
}: {
  className?: string;
}) {
  const muted = useVoice((s) => s.muted);
  const deafened = useVoice((s) => s.deafened);
  const mic = useVoice((s) => s.mic);
  const micOff = muted || deafened;
  const pushToTalk = usePushToTalk((s) => s.active);
  // With push-to-talk the button shows the key at work: lit while it opens
  // the microphone, dimmed while it holds it closed.
  const talking = usePushToTalk((s) => s.active && s.open) && !micOff;
  const idle = mic === "ptt-idle" && !micOff;
  return (
    <>
      {mic === "unavailable" || mic === "denied" ? (
        <MicProblem className={className} denied={mic === "denied"} />
      ) : (
        <button
          type="button"
          aria-pressed={micOff}
          aria-label="Stummschalten"
          aria-keyshortcuts={callShortcutKeys("mute")}
          title={`${micOff ? "Stummschaltung aufheben" : "Stummschalten"} (${callShortcutLabel("mute")})${pushToTalk && !micOff ? " · Push-to-Talk" : ""}`}
          onClick={() => toggleMute()}
          className={`${className}${micOff ? " is-off" : ""}${talking ? " is-talking" : ""}${idle ? " is-ptt-idle" : ""}`}
        >
          {micOff ? <MicOffIcon size={17} /> : <MicIcon size={17} />}
          {pushToTalk && !micOff ? (
            <span className="voice-control-badge" aria-hidden="true">
              PTT
            </span>
          ) : null}
        </button>
      )}
      <button
        type="button"
        aria-pressed={deafened}
        aria-label="Taubschalten"
        aria-keyshortcuts={callShortcutKeys("deafen")}
        title={`${deafened ? "Taubschaltung aufheben" : "Taubschalten"} (${callShortcutLabel("deafen")})`}
        onClick={() => toggleDeafen()}
        className={`${className}${deafened ? " is-off" : ""}`}
      >
        {deafened ? <HeadsetOffIcon size={17} /> : <HeadsetIcon size={17} />}
      </button>
    </>
  );
}

/** A microphone the call goes on without: says so, and pressing it says
 * why and offers to try again instead of muting nothing. */
function MicProblem({
  className,
  denied,
}: {
  className: string;
  denied: boolean;
}) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const openSettings = useMediaSettings((s) => s.openDialog);
  return (
    <>
      <button
        ref={anchor}
        type="button"
        aria-label="Kein Mikrofon"
        aria-haspopup="dialog"
        aria-expanded={open}
        title="Kein Mikrofon – du hörst nur zu"
        onClick={() => setOpen((value) => !value)}
        className={`${className} voice-control-mic-problem`}
      >
        <MicOffIcon size={17} />
        <span className="voice-control-dot" aria-hidden="true" />
        <span className="voice-control-text">Kein Mikrofon</span>
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label="Kein Mikrofon"
        placement="top-start"
        className="voice-mic-problem"
      >
        <p>
          {!denied
            ? "Es wurde kein Mikrofon gefunden, oder es ist gerade belegt."
            : isDesktopApp()
              ? "Gelabber darf das Mikrofon nicht nutzen. Erlaube den Zugriff in den Systemeinstellungen und versuch es dann erneut."
              : "Der Browser erlaubt den Mikrofonzugriff nicht. Erlaube ihn in den Website-Einstellungen und versuch es dann erneut."}{" "}
          Du hörst weiter zu.
        </p>
        <div className="voice-mic-problem-actions">
          <Button
            size="sm"
            onClick={() => {
              setOpen(false);
              retryMicrophone();
            }}
          >
            Erneut versuchen
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              setOpen(false);
              openSettings();
            }}
          >
            Mikrofon wählen …
          </Button>
        </div>
      </Popover>
    </>
  );
}

/** A capture the desktop app does not have: the control stays in its place,
 * dimmed, and says why on hover and on press. */
function Unavailable({
  label,
  reason,
  className,
  children,
}: {
  label: string;
  reason: string;
  className: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-disabled="true"
      aria-label={label}
      title={reason}
      onClick={() => notify(reason)}
      className={`${className} opacity-50`}
    >
      {children}
    </button>
  );
}

type LiveProps = {
  /** This server lets the user go live (`go_live`). */
  canLive?: boolean;
};

export function VoiceControls({
  canLive = false,
  streamSound = false,
}: LiveProps & {
  /** Someone's Live plays here, so its sound gets a control of its own. */
  streamSound?: boolean;
}) {
  const camera = useVoice((s) => s.camera);
  const capable = useCapabilities();

  return (
    <div className="voice-controls voice-controls-compact">
      <VoiceAudioToggles />
      {camera || capable.camera ? (
        <button
          type="button"
          aria-pressed={camera}
          aria-label="Kamera"
          title={camera ? "Kamera ausschalten" : "Kamera einschalten"}
          onClick={() => toggleCamera()}
          className={`${BUTTON}${camera ? " is-on" : ""}`}
        >
          {camera ? <CameraIcon size={17} /> : <CameraOffIcon size={17} />}
        </button>
      ) : capable.desktop ? (
        <Unavailable
          label="Kamera"
          reason="Diese Desktop-App kann keine Kamera nutzen."
          className={BUTTON}
        >
          <CameraOffIcon size={17} />
        </Unavailable>
      ) : null}
      <LiveControl canLive={canLive} />
      {streamSound ? <CallSoundButton className={BUTTON} /> : null}
      <QuickCallSettingsButton className={BUTTON} />
    </div>
  );
}

const LIVE_HINT = "Sehen alle im Sprachkanal";

/** Why going live is not on offer here, if it is not. */
function liveUnavailable(
  canLive: boolean,
  capable: Capabilities,
): string | null {
  if (!capable.screen) return "Diese Desktop-App kann nicht live gehen.";
  if (!canLive) return "In diesem Server darfst du nicht live gehen.";
  return null;
}

/** "Live gehen": a screen or window, seen by everyone in the voice channel,
 * several people at once. Where a Live can carry sound it is a menu, so
 * that choice is in sight before the picker opens; otherwise one toggle.
 * Ending a Live is always offered. */
function LiveControl({ canLive = false }: LiveProps) {
  const live = useVoice((s) => s.sharing);
  const capable = useCapabilities();
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  // A phone browser cannot capture a screen: nothing to offer, nothing to
  // explain. The desktop app explains what it lacks.
  if (!live && !capable.screen && !capable.desktop) return null;
  const reason = live ? null : liveUnavailable(canLive, capable);
  const className = `${BUTTON}${live ? " voice-control-live" : ""}`;
  const face = (
    <>
      <LiveIcon size={17} />
      <span className="voice-control-text">Live</span>
    </>
  );
  if (!capable.appAudio) {
    if (reason)
      return (
        <Unavailable label="Live gehen" reason={reason} className={BUTTON}>
          {face}
        </Unavailable>
      );
    return (
      <button
        type="button"
        aria-pressed={live}
        aria-label="Live gehen"
        title={live ? "Live beenden" : "Live gehen – sehen alle im Sprachkanal"}
        onClick={() => toggleShare()}
        className={className}
      >
        {face}
      </button>
    );
  }
  return (
    <>
      <button
        ref={anchor}
        type="button"
        aria-pressed={live}
        aria-label="Live gehen"
        aria-haspopup="menu"
        aria-expanded={open}
        title={live ? "Du bist live" : "Live gehen"}
        onClick={() => setOpen((value) => !value)}
        className={`${className}${reason ? " opacity-50" : ""}`}
      >
        {face}
        <ChevronIcon size={12} className="voice-control-caret" />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label="Live gehen"
        role="menu"
        placement="top-end"
        className="voice-share-menu"
      >
        <LiveMenuItems canLive={canLive} onDone={() => setOpen(false)} />
      </Popover>
    </>
  );
}

/** Up, down, Home and End walk the entries, as in any menu. */
function walkMenu(event: KeyboardEvent<HTMLElement>): void {
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  const items = [
    ...event.currentTarget.querySelectorAll<HTMLElement>(
      '[role="menuitem"], [role="menuitemcheckbox"]',
    ),
  ];
  if (items.length === 0) return;
  event.preventDefault();
  const at = items.indexOf(document.activeElement as HTMLElement);
  const next =
    event.key === "Home"
      ? 0
      : event.key === "End"
        ? items.length - 1
        : (at + (event.key === "ArrowDown" ? 1 : -1) + items.length) %
          items.length;
  items[next]?.focus();
}

function MenuEntry({
  icon,
  label,
  hint,
  disabled = false,
  danger = false,
  onSelect,
}: {
  icon: ReactNode;
  label: string;
  hint?: string;
  disabled?: boolean;
  danger?: boolean;
  onSelect: () => void;
}) {
  const hintId = useId();
  return (
    <button
      type="button"
      role="menuitem"
      aria-label={label}
      aria-disabled={disabled ? "true" : undefined}
      aria-describedby={hint ? hintId : undefined}
      onClick={disabled ? undefined : onSelect}
      className={`voice-menu-item${danger ? " is-danger" : ""}`}
    >
      {icon}
      <span>
        <strong>{label}</strong>
        {hint ? <small id={hintId}>{hint}</small> : null}
      </span>
    </button>
  );
}

/** The entries of the Live menu: going live or ending it, then whether a
 * Live carries sound. */
export function LiveMenuItems({
  canLive = false,
  onDone = () => {},
}: LiveProps & { onDone?: () => void }) {
  const live = useVoice((s) => s.sharing);
  const localScreen = useVoice((s) => s.localScreen);
  const sourceAudio = useVoice((s) => s.sourceAudio);
  const sourceAudioShare = useMediaSettings((s) => s.sourceAudioShare);
  const sourceAudioApp = useMediaSettings((s) => s.sourceAudioApp);
  const patch = useMediaSettings((s) => s.patch);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const capable = useCapabilities();
  const soundHintId = useId();
  const desktop = isDesktopApp();
  const sound = sharesSourceAudio({ sourceAudioShare });
  const reason = liveUnavailable(canLive, capable);
  // A browser hands sound over in its picker only: switched on during a
  // Live, it comes with the next one.
  const nextLive = sound && !desktop && localScreen && sourceAudio.s === "off";
  // The desktop app captures the sound without a picker, so the entry says
  // whose it is, before a Live and while one runs.
  const soundHint = [
    sourceAudioScope({ sourceAudioApp }) ??
      "Wähle den Ton im Freigabedialog des Browsers aus.",
    nextLive ? "Kommt mit deinem nächsten Live dazu." : null,
    sourceAudioCarriesCall({ sourceAudioShare, sourceAudioApp })
      ? SOURCE_AUDIO_CARRIES_CALL
      : null,
  ]
    .filter(Boolean)
    .join(" ");
  const done = (action: () => void) => () => {
    onDone();
    action();
  };

  return (
    <div role="none" className="voice-menu" onKeyDown={walkMenu}>
      {live ? (
        <MenuEntry
          icon={<LiveIcon size={17} />}
          label="Live beenden"
          danger
          onSelect={done(toggleShare)}
        />
      ) : (
        <MenuEntry
          icon={<LiveIcon size={17} />}
          label="Live gehen"
          hint={reason ?? LIVE_HINT}
          disabled={reason !== null}
          onSelect={done(toggleShare)}
        />
      )}
      {capable.appAudio ? (
        <>
          <div role="separator" className="voice-menu-separator" />
          <button
            type="button"
            role="menuitemcheckbox"
            aria-label="Ton mitsenden"
            aria-checked={sound}
            aria-describedby={soundHintId}
            onClick={() =>
              patch({ sourceAudioShare: sourceAudioChoice(!sound) })
            }
            className="voice-menu-item voice-menu-switch"
          >
            <span>
              <strong>Ton mitsenden</strong>
              <small id={soundHintId}>{soundHint}</small>
            </span>
            <span className="voice-menu-switch-track" aria-hidden="true" />
          </button>
          {desktop ? (
            <MenuEntry
              icon={<GearIcon size={17} />}
              label="Anwendung für den Ton wählen …"
              onSelect={done(() => openSettings())}
            />
          ) : null}
        </>
      ) : null}
    </div>
  );
}

/** Push-to-talk on a touch screen, which has no key to hold: a button to
 * hold instead. */
export function HoldToTalk() {
  const held = usePushToTalk((s) => s.held);
  const release = () => holdToTalk(false);
  return (
    <button
      type="button"
      className={`voice-hold-to-talk${held ? " is-held" : ""}`}
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture?.(event.pointerId);
        holdToTalk(true);
      }}
      onPointerUp={release}
      onPointerCancel={release}
      onLostPointerCapture={release}
      onKeyDown={(event) => {
        if ((event.key === " " || event.key === "Enter") && !event.repeat) {
          event.preventDefault();
          holdToTalk(true);
        }
      }}
      onKeyUp={(event) => {
        if (event.key === " " || event.key === "Enter") release();
      }}
      onBlur={release}
      onContextMenu={(event) => event.preventDefault()}
    >
      <MicIcon size={18} />
      {held ? "Du sprichst" : "Zum Sprechen gedrückt halten"}
    </button>
  );
}
