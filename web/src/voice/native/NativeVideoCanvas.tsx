// A native video track drawn in the page. The canvas takes the place of the
// browser's <video>: same box, same object-fit, its bitmap is the picture.
import { useEffect, useLayoutEffect, useRef } from "react";

import type { NativeTrack } from "./tracks.ts";
import { attachNativeVideo } from "./videoFeed.ts";
import "./video.css";

/** Attribute the canvas of a track carries, with the track id as value. */
export const NATIVE_TRACK_ATTRIBUTE = "data-native-track";

export function NativeVideoCanvas({
  track,
  className,
  onDoubleClick,
  onPainted,
}: {
  track: NativeTrack;
  className?: string;
  onDoubleClick?: () => void;
  /** True once a frame of this track is on the canvas. */
  onPainted?: (painted: boolean) => void;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const painted = useRef(onPainted);
  useEffect(() => {
    painted.current = onPainted;
  });
  // Before the browser paints: a tile that moved shows the stream's current
  // picture at once instead of its placeholder.
  useLayoutEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const detach = attachNativeVideo(track, canvas, () =>
      painted.current?.(true),
    );
    return () => {
      detach();
      painted.current?.(false);
    };
  }, [track]);
  return (
    <canvas
      ref={ref}
      {...{ [NATIVE_TRACK_ATTRIBUTE]: track.id }}
      // Empty until the first frame sets the bitmap to the picture's size.
      width={0}
      height={0}
      onDoubleClick={onDoubleClick}
      className={className}
    />
  );
}
