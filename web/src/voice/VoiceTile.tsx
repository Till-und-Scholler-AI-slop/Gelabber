// Local or remote video tile. srcObject is set in an effect so the
// preview can appear in the same frame as the stream, without SDP.

import { useEffect, useRef } from "react";

import { CollapseIcon, ExpandIcon } from "../components/Icons.tsx";

export function VoiceTile({
  stream,
  label,
  mirror,
  screen,
  live,
  expanded,
  onToggleExpand,
}: {
  stream: MediaStream | null;
  label: string;
  mirror?: boolean;
  screen?: boolean;
  live?: boolean;
  expanded?: boolean;
  onToggleExpand?: () => void;
}) {
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.srcObject = stream;
    if (stream) {
      const play = () => {
        void el.play()?.catch(() => undefined);
      };
      play();
      const onUnmute = () => play();
      stream.getVideoTracks().forEach((track) => {
        track.addEventListener("unmute", onUnmute);
      });
      return () => {
        stream.getVideoTracks().forEach((track) => {
          track.removeEventListener("unmute", onUnmute);
        });
        el.srcObject = null;
      };
    }
    return () => {
      el.srcObject = null;
    };
  }, [stream]);

  return (
    <figure
      className={[
        "relative overflow-hidden rounded-lg bg-neutral-900 text-white",
        screen || expanded ? "aspect-video w-full" : "aspect-video",
        expanded ? "min-h-[40vh] sm:min-h-[56vh]" : "",
      ].join(" ")}
    >
      <video
        ref={ref}
        autoPlay
        playsInline
        muted
        className={[
          "size-full",
          screen || expanded ? "object-contain" : "object-cover",
          mirror ? "-scale-x-100" : "",
          stream ? "" : "opacity-0",
        ].join(" ")}
      />
      {!stream ? (
        <div className="absolute inset-0 flex items-center justify-center text-sm text-neutral-400">
          {label}
        </div>
      ) : null}
      <figcaption className="absolute inset-x-0 bottom-0 truncate bg-black/50 px-2 py-1 text-left text-xs">
        {label}
      </figcaption>
      {live ? (
        <span className="absolute top-2 left-2 rounded bg-red-600 px-1.5 py-0.5 text-[10px] font-semibold tracking-wide uppercase">
          Live
        </span>
      ) : null}
      {onToggleExpand ? (
        <button
          type="button"
          onClick={onToggleExpand}
          aria-pressed={expanded ?? false}
          aria-label={expanded ? "Rasteransicht" : "Maximieren"}
          title={expanded ? "Rasteransicht" : "Maximieren"}
          className="absolute top-2 right-2 rounded-md bg-black/55 p-1.5 text-white hover:bg-black/75"
        >
          {expanded ? <CollapseIcon size={16} /> : <ExpandIcon size={16} />}
        </button>
      ) : null}
    </figure>
  );
}
