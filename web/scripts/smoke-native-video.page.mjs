// Page side of smoke-native-video.mjs, which bundles it with Vite: the
// client's own tile, feed and renderer. The desktop app around them is the
// runner's fake `window.__TAURI_INTERNALS__`.
/* global window, document */
import { createElement } from "react";
import { createRoot } from "react-dom/client";

import "../src/index.css";
import { NativeStream, NativeTrack } from "../src/voice/native/tracks.ts";
import { nativeVideoLimit } from "../src/voice/native/videoFeed.ts";
import { renderedVideoHeight } from "../src/voice/viewerLayers.ts";
import { VoiceTile } from "../src/voice/VoiceTile.tsx";

const root = createRoot(document.querySelector("#root"));
/** Streams by tile name, so a tile keeps its track from render to render. */
const streams = new Map();

function streamOf({ name, consumer, source }) {
  let stream = streams.get(name);
  if (!stream) {
    const handle = consumer === undefined ? { source } : { consumer };
    stream = new NativeStream([new NativeTrack("video", name, handle)]);
    streams.set(name, stream);
  }
  return stream;
}

const row = { display: "flex", flexWrap: "wrap", gap: 8, padding: 8 };

window.smoke = {
  /** Renders one tile per entry, `width` CSS pixels wide:
   * `{ name, consumer | source, width, focus?, mirror?, screen?, live? }`.
   * Tiles with `focus` sit in a container of their own, so setting it
   * remounts the tile like the room's focus does. Answers the track ids by
   * name. */
  show(tiles) {
    const ids = {};
    const tile = ({ name, consumer, source, width, focus, ...props }) => {
      void focus;
      const stream = streamOf({ name, consumer, source });
      ids[name] = stream.getVideoTracks()[0].id;
      return createElement(
        "div",
        { key: name, "data-tile": name, style: { width } },
        createElement(VoiceTile, { stream, label: name, ...props }),
      );
    };
    root.render(
      createElement(
        "div",
        null,
        createElement(
          "div",
          { style: row },
          tiles.filter((entry) => entry.focus).map(tile),
        ),
        createElement(
          "div",
          { style: row },
          tiles.filter((entry) => !entry.focus).map(tile),
        ),
      ),
    );
    return ids;
  },
  renderedVideoHeight,
  nativeVideoLimit,
};
