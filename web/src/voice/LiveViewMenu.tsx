// GEL-32: "Bildqualität" on someone else's Live. A viewer on a weak
// download chooses for itself whether the Live keeps sharp text (the frame
// rate drops first) or stays smooth (the resolution drops first); "Wie
// gesendet" follows the owner's "Text scharf halten". Holds for this call.

import { useRef, useState, type KeyboardEvent } from "react";

import { CheckIcon, GearIcon } from "../components/Icons.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { Popover } from "../components/ui/Popover.tsx";
import { useT } from "../i18n/catalog.ts";
import { voice } from "../i18n/catalogs/voice.ts";
import { setLiveViewMode, useVoice, type LiveViewMode } from "./session.ts";
import "./room.css";

const CHOICES = [
  { mode: null, label: "view.auto", hint: "view.autoHint" },
  { mode: "sharp", label: "view.sharp", hint: "view.sharpHint" },
  { mode: "smooth", label: "view.smooth", hint: "view.smoothHint" },
] as const satisfies readonly {
  mode: LiveViewMode | null;
  label: string;
  hint: string;
}[];

/** The quality button on a watched Live: an icon on the tile, a labelled
 * button in the fullscreen strip. */
export function LiveViewMenu({
  userId,
  name,
  strip = false,
}: {
  userId: string;
  name: string;
  strip?: boolean;
}) {
  const t = useT(voice);
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const current = useVoice((s) => s.liveViewModes[userId] ?? null);
  const toggle = () => setOpen((was) => !was);
  return (
    <>
      {strip ? (
        <button
          ref={anchor}
          type="button"
          aria-haspopup="menu"
          aria-expanded={open}
          className="voice-watch-button"
          onClick={toggle}
        >
          <GearIcon size={16} />
          <span>{t("view.label")}</span>
        </button>
      ) : (
        <IconButton
          ref={anchor}
          size="sm"
          label={t("view.label")}
          aria-haspopup="menu"
          aria-expanded={open}
          icon={<GearIcon size={16} />}
          onClick={toggle}
          className="voice-video-action"
        />
      )}
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label={t("view.menuLabel", { name })}
        role="menu"
        placement={strip ? "top-end" : "bottom-end"}
        className="voice-share-menu"
      >
        <div role="none" className="voice-menu" onKeyDown={walkRadios}>
          {CHOICES.map((choice) => (
            <button
              key={choice.label}
              type="button"
              role="menuitemradio"
              aria-checked={current === choice.mode}
              data-view-mode={choice.mode ?? "auto"}
              className="voice-menu-item"
              onClick={() => {
                setOpen(false);
                setLiveViewMode(userId, choice.mode);
              }}
            >
              {current === choice.mode ? (
                <CheckIcon size={17} />
              ) : (
                <span className="voice-menu-check" aria-hidden="true" />
              )}
              <span>
                <strong>{t(choice.label)}</strong>
                <small>{t(choice.hint)}</small>
              </span>
            </button>
          ))}
        </div>
      </Popover>
    </>
  );
}

/** Up, down, Home and End walk the choices, as in any menu. */
function walkRadios(event: KeyboardEvent<HTMLElement>): void {
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  const items = [
    ...event.currentTarget.querySelectorAll<HTMLElement>(
      '[role="menuitemradio"]',
    ),
  ];
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
