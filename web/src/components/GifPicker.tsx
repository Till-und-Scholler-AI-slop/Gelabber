import { useId, useState } from "react";

import {
  gifBox,
  gifFeedStatus,
  gifLabel,
  gifMediaUrl,
  type GifPick,
} from "../gifs/api.ts";
import { useGifFeed } from "../gifs/queries.ts";
import { useT } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import { common } from "../i18n/catalogs/common.ts";
import { useReducedMotion } from "../interface/useReducedMotion.ts";
import type { Gif } from "../messages/types.ts";
import { useDebouncedValue } from "./useDebouncedValue.ts";

/** One GIF rendition through the server relay. With `playing` off it shows
 *  the first frame; without one, a labelled placeholder. */
export function GifImage({
  gif,
  playing,
  alt,
  width,
  height,
  fit = "contain",
}: {
  gif: Gif;
  playing: boolean;
  alt: string;
  width?: number;
  height?: number;
  fit?: "contain" | "cover";
}) {
  const src = playing ? gif.url : gif.still;
  if (!src)
    return (
      <span
        className="lr-gif-placeholder"
        role="img"
        aria-label={alt}
        style={{ width, height }}
      >
        GIF{/* i18n-ok */}
      </span>
    );
  return (
    <img
      src={gifMediaUrl(src)}
      alt={alt}
      width={width}
      height={height}
      decoding="async"
      loading="lazy"
      draggable={false}
      style={{ objectFit: fit }}
    />
  );
}

/** A GIF in a message. Plays in place unless `reduced`; then the first
 *  frame, animated while hovered. It is always a pause/play toggle, so
 *  animation that starts on its own can be stopped (WCAG 2.2.2). */
export function GifMessageButton({
  gif,
  reduced,
}: {
  gif: Gif;
  reduced: boolean;
}) {
  const t = useT(chat);
  const [chosen, setChosen] = useState<boolean | null>(null);
  const [hover, setHover] = useState(false);
  const playing = chosen ?? (!reduced || hover);
  const box = gifBox(gif);
  const label = gifLabel(gif);
  return (
    <button
      type="button"
      className="lr-message-gif"
      style={{ width: box.width, height: box.height }}
      // A neutral name: the pressed state already says whether it plays.
      aria-label={t("gif.animationLabel", { label })}
      aria-pressed={playing}
      title={gif.title || undefined}
      onClick={() => setChosen(!playing)}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
    >
      <GifImage
        gif={gif}
        playing={playing}
        alt={label}
        width={box.width}
        height={box.height}
      />
      {playing ? null : (
        <span className="lr-message-gif-badge" aria-hidden="true">
          GIF{/* i18n-ok */}
        </span>
      )}
    </button>
  );
}

/** Typing pauses this long before a search goes out. Search, trending and
 *  "more" share the server's per-user GIF budget. */
const SEARCH_DEBOUNCE_MS = 550;

export default function GifPicker({
  onSelect,
}: {
  onSelect: (pick: GifPick) => void;
}) {
  const t = useT(chat);
  const tc = useT(common);
  const id = useId();
  const [input, setInput] = useState("");
  const query = useDebouncedValue(input.trim(), SEARCH_DEBOUNCE_MS);
  const feed = useGifFeed(query, true);
  const reduced = useReducedMotion();
  const [active, setActive] = useState<string | null>(null);
  const gifs = feed.data?.pages.flatMap((page) => page.gifs) ?? [];
  const status = gifFeedStatus({
    pending: feed.isPending,
    error: feed.isError ? feed.error : null,
    count: gifs.length,
    query,
  });
  return (
    <div className="lr-gif-picker flex flex-col gap-3">
      <label htmlFor={`${id}-search`} className="text-sm font-medium">
        {t("gif.search")}
      </label>
      <input
        id={`${id}-search`}
        type="search"
        value={input}
        onChange={(event) => setInput(event.target.value)}
        // Klipy's attribution rule asks for exactly this placeholder.
        placeholder="Search KLIPY" // i18n-ok
        maxLength={100}
        autoFocus
        enterKeyHint="search"
        className="rounded border border-neutral-300 p-2 dark:border-neutral-600"
      />
      <p role="status" className="text-xs text-neutral-500">
        {status}
      </p>
      {feed.isError && gifs.length === 0 ? (
        <button
          type="button"
          onClick={() => void feed.refetch()}
          className="rounded border px-3 py-2 text-sm"
        >
          {tc("tryAgain")}
        </button>
      ) : null}
      <div className="lr-gif-grid" role="group" aria-label={t("gif.gridLabel")}>
        {gifs.map((gif, index) => (
          <button
            key={`${gif.slug}:${index}`}
            type="button"
            title={gif.title || undefined}
            aria-label={gifLabel(gif)}
            onClick={() => onSelect({ slug: gif.slug, query, gif })}
            onPointerEnter={() => setActive(gif.slug)}
            onPointerLeave={() => setActive(null)}
            onFocus={() => setActive(gif.slug)}
            onBlur={() => setActive(null)}
            className="lr-gif-tile"
          >
            <GifImage
              gif={gif}
              playing={!reduced || active === gif.slug}
              alt=""
              fit="cover"
            />
          </button>
        ))}
      </div>
      {feed.hasNextPage ? (
        <button
          type="button"
          disabled={feed.isFetchingNextPage}
          onClick={() => void feed.fetchNextPage()}
          className="rounded border px-3 py-2 text-sm"
        >
          {t(feed.isFetchingNextPage ? "gif.loadingMore" : "gif.more")}
        </button>
      ) : null}
      {/* Klipy's attribution, in English as it asks. */}
      <p className="lr-gif-attribution">Powered by KLIPY{/* i18n-ok */}</p>
    </div>
  );
}
