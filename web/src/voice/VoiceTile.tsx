// Local or remote video tile. srcObject is set in an effect so the
// preview can appear in the same frame as the stream, without SDP.

import { useEffect, useRef } from "react";

export function VoiceTile({
  stream,
  label,
  mirror,
  screen,
}: {
  stream: MediaStream | null;
  label: string;
  mirror?: boolean;
  screen?: boolean;
}) {
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.srcObject = stream;
    return () => {
      el.srcObject = null;
    };
  }, [stream]);

  return (
    <figure
      className={[
        "relative overflow-hidden rounded-lg bg-neutral-900 text-white",
        screen ? "aspect-video w-full" : "aspect-video",
      ].join(" ")}
    >
      <video
        ref={ref}
        autoPlay
        playsInline
        muted
        className={[
          "size-full object-cover",
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
    </figure>
  );
}
