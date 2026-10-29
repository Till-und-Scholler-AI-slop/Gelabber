// How a room's pictures are laid out. The channel's page gives every
// picture a tile of its own, sized to fill the stage (Raster, fitGrid), or
// one picture the stage over a row of the others and the people (Fokus,
// focusRects). The overview keeps one spotlight, a filmstrip of the other
// Lives, and cameras in their owners' places among the participants.

export type StageKind = "live" | "camera";

/** The first Live takes the spotlight unless someone pinned another picture
 * there, a camera included. The filmstrip holds the Lives that are not in
 * it; cameras stay with their owners. */
export function arrangeStage<T extends { id: string; kind: StageKind }>(
  shares: readonly T[],
  cameras: readonly T[],
  pinned: string | null,
): { spotlight: T | null; filmstrip: T[] } {
  const spotlight =
    [...shares, ...cameras].find((tile) => tile.id === pinned) ??
    shares[0] ??
    null;
  return {
    spotlight,
    filmstrip: shares.filter((tile) => tile !== spotlight),
  };
}

/** The stage's pictures in their order: a pinned one first, then the
 * Lives, then the cameras. */
export function rasterOrder<T extends { id: string }>(
  shares: readonly T[],
  cameras: readonly T[],
  pinned: string | null,
): T[] {
  const all = [...shares, ...cameras];
  const first = all.find((tile) => tile.id === pinned);
  return first ? [first, ...all.filter((tile) => tile !== first)] : all;
}

/** Tiles in rows and columns, each `width` × `height` pixels. */
export type GridFit = {
  columns: number;
  rows: number;
  width: number;
  height: number;
};

/** A camera is cut to fit its tile, but never narrower than square nor
 * wider than 16:9. */
export const TILE_ASPECT = { min: 1, max: 16 / 9 } as const;

/** How `count` tiles fill a `width` × `height` stage with `gap` between
 * them: of every number of columns, the one whose tiles come out largest.
 * A tile keeps its width to height between `aspect.min` and `aspect.max`,
 * so a lone camera on a tall stage is not a sliver. Null while there is
 * no room to measure. */
export function fitGrid(
  count: number,
  width: number,
  height: number,
  gap: number,
  aspect: { min: number; max: number } = TILE_ASPECT,
): GridFit | null {
  if (count < 1 || !(width > 0) || !(height > 0)) return null;
  let best: GridFit | null = null;
  let bestArea = 0;
  for (let columns = 1; columns <= count; columns++) {
    const rows = Math.ceil(count / columns);
    const cellWidth = (width - gap * (columns - 1)) / columns;
    const cellHeight = (height - gap * (rows - 1)) / rows;
    if (cellWidth <= 0 || cellHeight <= 0) continue;
    const tileWidth = Math.min(cellWidth, cellHeight * aspect.max);
    const tileHeight = Math.min(cellHeight, tileWidth / aspect.min);
    const area = tileWidth * tileHeight;
    // Ties keep the fewer columns.
    if (area > bestArea + 0.5) {
      bestArea = area;
      best = {
        columns,
        rows,
        width: Math.floor(tileWidth),
        height: Math.floor(tileHeight),
      };
    }
  }
  return best;
}

/** A tile's place on the stage, in CSS pixels from its top left corner. */
export type Rect = { x: number; y: number; width: number; height: number };

/** The space between tiles on the stage. */
export const STAGE_GAP = 12;

/** Every picture as a tile of fitGrid's size, the grid centred on the
 * stage and a short last row centred under the others. In the order of
 * the pictures; empty while the stage has no size. */
export function rasterRects(
  count: number,
  width: number,
  height: number,
  gap = STAGE_GAP,
): Rect[] {
  const fit = fitGrid(count, width, height, gap);
  if (!fit) return [];
  const span = (n: number, size: number) => n * size + (n - 1) * gap;
  const top = Math.round((height - span(fit.rows, fit.height)) / 2);
  return Array.from({ length: count }, (_, index) => {
    const row = Math.floor(index / fit.columns);
    const column = index - row * fit.columns;
    const inRow = Math.min(fit.columns, count - row * fit.columns);
    const left = Math.round((width - span(inRow, fit.width)) / 2);
    return {
      x: left + column * (fit.width + gap),
      y: top + row * (fit.height + gap),
      width: fit.width,
      height: fit.height,
    };
  });
}

/** The picture that fills the stage in Fokus: the pinned one, else the
 * first Live of someone else, else the own Live, else someone else's
 * camera, else the own. */
export function focusOf<
  T extends { id: string; kind: StageKind; userId: string },
>(
  shares: readonly T[],
  cameras: readonly T[],
  pinned: string | null,
  me: string | undefined,
): T | null {
  return (
    [...shares, ...cameras].find((tile) => tile.id === pinned) ??
    shares.find((tile) => tile.userId !== me) ??
    shares[0] ??
    cameras.find((tile) => tile.userId !== me) ??
    cameras[0] ??
    null
  );
}

/** Fokus's row under the picture: 96 pixels high, its tiles 16:9. */
export const STRIP_HEIGHT = 96;
export const STRIP_TILE = 171;
export const STRIP_GAP = 10;

/** Fokus: the largest 16:9 picture the stage holds above a row of
 * `stripCount` small tiles, both centred. The row's tiles narrow when
 * there are more than fit. Null while the stage has no size. */
export function focusRects(
  stripCount: number,
  width: number,
  height: number,
  gap = STAGE_GAP,
): { focus: Rect; strip: Rect[] } | null {
  const strip = stripCount > 0 ? STRIP_HEIGHT + gap : 0;
  const fit = fitGrid(1, width, height - strip, 0, {
    min: 16 / 9,
    max: 16 / 9,
  });
  if (!fit) return null;
  const tile = Math.max(
    0,
    Math.min(
      STRIP_TILE,
      Math.floor((width - STRIP_GAP * (stripCount - 1)) / stripCount),
    ),
  );
  const row = stripCount * tile + (stripCount - 1) * STRIP_GAP;
  const left = Math.round((width - row) / 2);
  return {
    focus: {
      x: Math.round((width - fit.width) / 2),
      y: Math.round((height - strip - fit.height) / 2),
      width: fit.width,
      height: fit.height,
    },
    strip: Array.from({ length: stripCount }, (_, index) => ({
      x: left + index * (tile + STRIP_GAP),
      y: height - STRIP_HEIGHT,
      width: tile,
      height: STRIP_HEIGHT,
    })),
  };
}

/** How a tile gets the whole screen: the element itself where the page may
 * go fullscreen, the video's own player on an iPhone, the large view
 * otherwise. */
export function fullscreenRoute(
  fullscreenEnabled: boolean | undefined,
  video: { webkitEnterFullscreen?: unknown } | null,
): "element" | "video" | "viewer" {
  if (fullscreenEnabled) return "element";
  if (typeof video?.webkitEnterFullscreen === "function") return "video";
  return "viewer";
}

/** No picture yet: nothing for a moment, then a spinner, then the news
 * that none arrives. */
export const FRAME_LOADING_MS = 300;
export const FRAME_STALLED_MS = 8_000;
/** Overlaid controls hide after this long without a pointer or key. */
export const CONTROLS_IDLE_MS = 3_000;
/** A press this long on a participant opens their menu. */
export const LONG_PRESS_MS = 500;
/** A finger that moves this far scrolls instead of pressing. */
const LONG_PRESS_SLOP = 10;

/** Press-and-hold on a touch screen, where no contextmenu event can be
 * relied on (iOS Safari fires none). Mouse presses are left alone. */
export function createLongPress(
  onLongPress: () => void,
  timers: {
    set: (run: () => void, ms: number) => unknown;
    clear: (handle: unknown) => void;
  } = {
    set: (run, ms) => setTimeout(run, ms),
    clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
  },
) {
  let timer: unknown = null;
  let origin: { x: number; y: number } | null = null;
  let fired = false;
  const cancel = () => {
    if (timer !== null) timers.clear(timer);
    timer = null;
    origin = null;
  };
  return {
    down(event: { pointerType: string; clientX: number; clientY: number }) {
      cancel();
      fired = false;
      if (event.pointerType === "mouse") return;
      origin = { x: event.clientX, y: event.clientY };
      timer = timers.set(() => {
        timer = null;
        fired = true;
        onLongPress();
      }, LONG_PRESS_MS);
    },
    move(event: { clientX: number; clientY: number }) {
      if (
        origin &&
        Math.hypot(event.clientX - origin.x, event.clientY - origin.y) >
          LONG_PRESS_SLOP
      )
        cancel();
    },
    up: cancel,
    /** Whether the click that ends this press already did its work. */
    consumeClick(): boolean {
      const was = fired;
      fired = false;
      return was;
    },
  };
}
