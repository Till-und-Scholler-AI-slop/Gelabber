// The call's controls: microphone, headset, camera, "Live gehen", the
// volumes while a Live plays, as icon buttons. Each toggle keeps one name and says its state
// through aria-pressed; the icon shows the state, and media that is on
// sits on a pill: the camera accent, a Live red. A closed microphone or
// headset also says "Stumm" or "Taub". The tooltips (ui/Tooltip.tsx) say
// what a press does, with its shortcut. The click flips local state
// first. Leaving is the session's own button.
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

import { translate, useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { voice } from "../i18n/catalogs/voice.ts";
import {
  CameraIcon,
  CameraOffIcon,
  ChevronIcon,
  EyeIcon,
  HeadsetIcon,
  HeadsetOffIcon,
  LiveIcon,
  MicIcon,
  MicOffIcon,
} from "./Icons.tsx";
import { Button } from "./ui/Button.tsx";
import { Popover } from "./ui/Popover.tsx";
import { useTooltip } from "./ui/Tooltip.tsx";
import {
  retryMicrophone,
  toggleCamera,
  toggleDeafen,
  toggleMute,
  toggleShare,
  useVoice,
} from "../voice/session.ts";
import { useCapabilities } from "../voice/capabilities.ts";
import { LiveWatchersPopover } from "../voice/LiveWatchers.tsx";
import { useLiveWatchers, watchersTitle } from "../voice/useLiveWatchers.ts";
import { formatNumber } from "../i18n/format.ts";
import { useUserId } from "../auth/scope.ts";
import type { ServerDetail } from "../servers/types.ts";
import { CallSoundButton } from "../voice/QuickCallSettings.tsx";
import {
  callShortcutKeys,
  callTip,
  holdToTalk,
  usePushToTalk,
} from "../voice/pushToTalk.ts";
import {
  sharesSourceAudio,
  sourceAudioChoice,
  useMediaSettings,
  usePushToTalkKeyName,
} from "../voice/settings.ts";
import "../voice/room.css";

const BUTTON = "voice-control voice-control-compact";

/** Mute and deafen, for the call bar and wherever the call is watched. */
export function VoiceAudioToggles({
  className = BUTTON,
  words = false,
}: {
  className?: string;
  /** "Stumm" or "Taub" beside a closed one's icon: the call bar's. The
   * fullscreen strip keeps to its icons, as the desktop app's does. */
  words?: boolean;
}) {
  const t = useT(voice);
  const muted = useVoice((s) => s.muted);
  const deafened = useVoice((s) => s.deafened);
  const mic = useVoice((s) => s.mic);
  const micOff = muted || deafened;
  const pushToTalk = usePushToTalk((s) => s.active);
  // With push-to-talk the button shows the key at work: lit while it opens
  // the microphone, dimmed while it holds it closed.
  const talking = usePushToTalk((s) => s.active && s.open) && !micOff;
  const idle = mic === "ptt-idle" && !micOff;
  const pttKey = usePushToTalkKeyName();
  const muteTip = useTooltip(
    callTip("mute", micOff, pushToTalk ? pttKey : null),
  );
  const deafenTip = useTooltip(callTip("deafen", deafened));
  // Closed, each says so in a word beside its icon, as "Kein Mikrofon"
  // does: "Stumm", or "Taub" for both (Participant's order). The name
  // stays; aria-pressed carries the state.
  return (
    <>
      {mic === "unavailable" || mic === "denied" ? (
        <MicProblem className={className} denied={mic === "denied"} />
      ) : (
        <>
          <button
            {...muteTip.trigger}
            type="button"
            aria-pressed={micOff}
            aria-label={t("controls.mute")}
            aria-keyshortcuts={callShortcutKeys("mute")}
            onClick={() => toggleMute()}
            className={`${className}${micOff ? " is-off" : ""}${talking ? " is-talking" : ""}${idle ? " is-ptt-idle" : ""}`}
          >
            {micOff ? <MicOffIcon size={17} /> : <MicIcon size={17} />}
            {words && muted && !deafened ? (
              <span className="voice-control-text" aria-hidden="true">
                {t("controls.muted")}
              </span>
            ) : null}
            {pushToTalk && !micOff ? (
              <span className="voice-control-badge" aria-hidden="true">
                {t("controls.pttBadge")}
              </span>
            ) : null}
          </button>
          {muteTip.tooltip}
        </>
      )}
      <button
        {...deafenTip.trigger}
        type="button"
        aria-pressed={deafened}
        aria-label={t("controls.deafen")}
        aria-keyshortcuts={callShortcutKeys("deafen")}
        onClick={() => toggleDeafen()}
        className={`${className}${deafened ? " is-off" : ""}`}
      >
        {deafened ? <HeadsetOffIcon size={17} /> : <HeadsetIcon size={17} />}
        {words && deafened ? (
          <span className="voice-control-text" aria-hidden="true">
            {t("controls.deafened")}
          </span>
        ) : null}
      </button>
      {deafenTip.tooltip}
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
  const t = useT(voice);
  const tc = useT(common);
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const tip = useTooltip({ label: t("mic.none"), disabled: open });
  return (
    <>
      <button
        {...tip.trigger}
        ref={anchor}
        type="button"
        aria-label={t("mic.none")}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className={`${className} voice-control-mic-problem`}
      >
        <MicOffIcon size={17} />
        <span className="voice-control-dot" aria-hidden="true" />
        <span className="voice-control-text">{t("mic.none")}</span>
      </button>
      {tip.tooltip}
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label={t("mic.none")}
        placement="top-start"
        className="voice-mic-problem"
      >
        <p>{!denied ? t("mic.notFound") : t("mic.blocked")}</p>
        <div className="voice-mic-problem-actions">
          <Button
            size="sm"
            onClick={() => {
              setOpen(false);
              retryMicrophone();
            }}
          >
            {tc("tryAgain")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              setOpen(false);
              openSettings();
            }}
          >
            {t("mic.choose")}
          </Button>
        </div>
      </Popover>
    </>
  );
}

type LiveProps = {
  /** This server lets the user go live (`go_live`). */
  canLive?: boolean;
};

export function VoiceControls({
  canLive = false,
  streamSound = false,
  server,
}: LiveProps & {
  /** Someone's Live plays here, so its sound gets a control of its own. */
  streamSound?: boolean;
  /** The call's server: the own Live's watchers by name. */
  server?: ServerDetail;
}) {
  const t = useT(voice);
  const camera = useVoice((s) => s.camera);
  const capable = useCapabilities();
  const cameraTip = useTooltip({
    label: camera ? t("controls.cameraOff") : t("controls.cameraOn"),
  });

  return (
    <div className="voice-controls voice-controls-compact">
      <VoiceAudioToggles words />
      {camera || capable.camera ? (
        <>
          <button
            {...cameraTip.trigger}
            type="button"
            aria-pressed={camera}
            aria-label={t("controls.camera")}
            onClick={() => toggleCamera()}
            className={`${BUTTON}${camera ? " is-on" : ""}`}
          >
            {camera ? <CameraIcon size={17} /> : <CameraOffIcon size={17} />}
          </button>
          {cameraTip.tooltip}
        </>
      ) : null}
      <LiveControl canLive={canLive} server={server} />
      {streamSound ? <CallSoundButton className={BUTTON} /> : null}
    </div>
  );
}

/** Why going live is not on offer here, if it is not. */
function liveUnavailable(canLive: boolean): string | null {
  return canLive ? null : translate(voice, "live.notAllowed");
}

/** "Live gehen": a screen or window, seen by everyone in the voice channel,
 * several people at once. Where a Live can carry sound it is a menu, so
 * that choice is in sight before the picker opens; otherwise one toggle.
 * Ending a Live is always offered. */
function LiveControl({
  canLive = false,
  server,
}: LiveProps & { server?: ServerDetail }) {
  const t = useT(voice);
  const live = useVoice((s) => s.sharing);
  const capable = useCapabilities();
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  // Where a Live can carry sound the button opens a menu; otherwise it
  // toggles, and says what everyone sees.
  const tip = useTooltip(
    capable.appAudio
      ? {
          label: live ? t("live.youAreLive") : t("live.goLive"),
          disabled: open,
        }
      : {
          label: live ? t("live.end") : t("live.goLive"),
          detail: live ? null : t("live.hint"),
        },
  );
  // While live a press lists who watches, with the way to end it.
  if (live) return <OwnLiveControl server={server} />;
  // A phone browser cannot capture a screen: nothing to offer, nothing to
  // explain.
  if (!live && !capable.screen) return null;
  const reason = live ? null : liveUnavailable(canLive);
  const className = `${BUTTON}${live ? " voice-control-live" : ""}`;
  const face = (
    <>
      <LiveIcon size={17} />
      <span className="voice-control-text">{t("live.badge")}</span>
    </>
  );
  // Only a Live that runs gets here: it can always be ended.
  if (!capable.appAudio) {
    return (
      <>
        <button
          {...tip.trigger}
          type="button"
          aria-pressed={live}
          aria-label={t("live.goLive")}
          onClick={() => toggleShare()}
          className={className}
        >
          {face}
        </button>
        {tip.tooltip}
      </>
    );
  }
  return (
    <>
      <button
        {...tip.trigger}
        ref={anchor}
        type="button"
        aria-pressed={live}
        aria-label={t("live.goLive")}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className={`${className}${reason ? " opacity-50" : ""}`}
      >
        {face}
        <ChevronIcon size={12} className="voice-control-caret" />
      </button>
      {tip.tooltip}
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label={t("live.goLive")}
        role="menu"
        placement="top-end"
        className="voice-share-menu"
      >
        <LiveMenuItems canLive={canLive} onDone={() => setOpen(false)} />
      </Popover>
    </>
  );
}

/** The own Live on the call bar: how many watch it, and a press lists them
 * with "Live beenden" and, where a Live can carry sound, that switch. */
function OwnLiveControl({ server }: { server?: ServerDetail }) {
  const t = useT(voice);
  const serverId = useVoice((s) => s.serverId) ?? "";
  const channelId = useVoice((s) => s.channelId) ?? "";
  const channelName = useVoice((s) => s.channelName) ?? t("call.voiceChannel");
  const me = useUserId();
  const capable = useCapabilities();
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const list = useLiveWatchers({
    serverId,
    channelId,
    owner: me,
    members: server?.members,
  });
  const count = list.watching.length;
  const tip = useTooltip({
    label: t("live.youAreLiveWatchers", { watchers: watchersTitle(t, count) }),
    disabled: open,
  });
  return (
    <>
      <button
        {...tip.trigger}
        ref={anchor}
        type="button"
        aria-pressed
        aria-label={t("live.goLive")}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className={`${BUTTON} voice-control-live`}
      >
        <LiveIcon size={17} />
        <span className="voice-control-text">{t("live.badge")}</span>
        <span className="voice-control-watchers">
          <EyeIcon size={15} />
          {formatNumber(count)}
        </span>
        <ChevronIcon size={12} className="voice-control-caret" />
      </button>
      {tip.tooltip}
      <LiveWatchersPopover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        placement="top-end"
        own
        ownerName=""
        channelName={channelName}
        list={list}
        extra={capable.appAudio ? <ShareSoundSwitch role="switch" /> : null}
      />
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
  const t = useT(voice);
  const live = useVoice((s) => s.sharing);
  const capable = useCapabilities();
  const reason = liveUnavailable(canLive);
  const done = (action: () => void) => () => {
    onDone();
    action();
  };

  return (
    <div role="none" className="voice-menu" onKeyDown={walkMenu}>
      {live ? (
        <MenuEntry
          icon={<LiveIcon size={17} />}
          label={t("live.end")}
          danger
          onSelect={done(toggleShare)}
        />
      ) : (
        <MenuEntry
          icon={<LiveIcon size={17} />}
          label={t("live.goLive")}
          hint={reason ?? t("live.hint")}
          disabled={reason !== null}
          onSelect={done(toggleShare)}
        />
      )}
      {capable.appAudio ? (
        <>
          <div role="separator" className="voice-menu-separator" />
          <ShareSoundSwitch role="menuitemcheckbox" />
        </>
      ) : null}
    </div>
  );
}

/** "Ton mitsenden": whether a Live carries sound. An entry of the Live menu,
 * or a switch in the own Live's list of watchers. */
export function ShareSoundSwitch({
  role,
}: {
  role: "menuitemcheckbox" | "switch";
}) {
  const t = useT(voice);
  const localScreen = useVoice((s) => s.localScreen);
  const sourceAudio = useVoice((s) => s.sourceAudio);
  const sourceAudioShare = useMediaSettings((s) => s.sourceAudioShare);
  const patch = useMediaSettings((s) => s.patch);
  const soundHintId = useId();
  const sound = sharesSourceAudio({ sourceAudioShare });
  // A browser hands sound over in its picker only: switched on during a
  // Live, it comes with the next one.
  const nextLive = sound && localScreen && sourceAudio.s === "off";
  const soundHint = nextLive
    ? t("live.soundHintNextLive")
    : t("live.soundHint");
  return (
    <button
      type="button"
      role={role}
      aria-label={t("live.shareSound")}
      aria-checked={sound}
      aria-describedby={soundHintId}
      onClick={() => patch({ sourceAudioShare: sourceAudioChoice(!sound) })}
      className="voice-menu-item voice-menu-switch"
    >
      <span>
        <strong>{t("live.shareSound")}</strong>
        <small id={soundHintId}>{soundHint}</small>
      </span>
      <span className="voice-menu-switch-track" aria-hidden="true" />
    </button>
  );
}

/** Push-to-talk on a touch screen, which has no key to hold: a button to
 * hold instead. */
export function HoldToTalk() {
  const t = useT(voice);
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
      {held ? t("holdToTalk.talking") : t("holdToTalk.hold")}
    </button>
  );
}
