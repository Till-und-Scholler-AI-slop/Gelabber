//! How a room's pictures are laid out (web voice/stage.ts) and the stage's
//! choices for the call (voice/stageView.ts): Raster gives every picture a
//! tile sized to fill the stage, Fokus one picture over a row of the
//! others and the people, Kino one picture over the window with small
//! pictures in its corner.

use egui::{Context, Id};
use uuid::Uuid;

/// `StageKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Live,
    Camera,
}

/// A picture's id, `{user}-s` or `{user}-v` on the web.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PicId {
    pub user: Uuid,
    pub kind: Kind,
}

/// A tile's place on the stage, in px from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl StageRect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// The rect on screen for a stage whose top left is `origin`.
    pub fn on(self, origin: egui::Pos2) -> egui::Rect {
        egui::Rect::from_min_size(
            origin + egui::vec2(self.x as f32, self.y as f32),
            egui::vec2(self.w as f32, self.h as f32),
        )
    }
}

/// `GridFit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridFit {
    pub columns: i32,
    pub rows: i32,
    pub width: i32,
    pub height: i32,
}

/// `TILE_ASPECT`: never narrower than square nor wider than 16:9.
pub const TILE_ASPECT: (f64, f64) = (1.0, 16.0 / 9.0);
pub const WIDE: (f64, f64) = (16.0 / 9.0, 16.0 / 9.0);
pub const STAGE_GAP: f64 = 12.0;
pub const STRIP_HEIGHT: i32 = 96;
pub const STRIP_TILE: i32 = 171;
pub const STRIP_GAP: i32 = 10;
/// `KINO_THUMB`.
const KINO_PICTURE: f64 = 160.0;
const KINO_PERSON: f64 = 90.0;
const KINO_HEIGHT: f64 = 90.0;
const KINO_GAP: f64 = 8.0;
const KINO_RIGHT: f64 = 16.0;
const KINO_BOTTOM: f64 = 24.0;
/// Overlaid controls hide after this long without a pointer or key.
pub const CONTROLS_IDLE: f64 = 3.0;
/// No picture yet: a spinner after this, then the news that none arrives.
pub const FRAME_LOADING: f64 = 0.3;
pub const FRAME_STALLED: f64 = 8.0;

/// JavaScript's `Math.round` (halves towards +∞).
fn js_round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

/// `arrangeStage` (the overview): the pinned picture or the first Live in
/// the spotlight, the other Lives in the filmstrip.
pub fn arrange_stage<T: Clone>(
    shares: &[T],
    cameras: &[T],
    pinned: Option<PicId>,
    id: impl Fn(&T) -> PicId,
) -> (Option<T>, Vec<T>) {
    let spotlight = shares
        .iter()
        .chain(cameras)
        .find(|t| Some(id(t)) == pinned)
        .or(shares.first())
        .cloned();
    let spot = spotlight.as_ref().map(&id);
    let filmstrip = shares
        .iter()
        .filter(|t| Some(id(t)) != spot)
        .cloned()
        .collect();
    (spotlight, filmstrip)
}

/// `rasterOrder`: a pinned picture first, then the Lives, then the cameras.
pub fn raster_order<T: Clone>(
    shares: &[T],
    cameras: &[T],
    pinned: Option<PicId>,
    id: impl Fn(&T) -> PicId,
) -> Vec<T> {
    let all: Vec<T> = shares.iter().chain(cameras).cloned().collect();
    match all.iter().position(|t| Some(id(t)) == pinned) {
        Some(at) => {
            let mut out = vec![all[at].clone()];
            out.extend(
                all.iter()
                    .enumerate()
                    .filter(|(i, _)| *i != at)
                    .map(|(_, t)| t.clone()),
            );
            out
        }
        None => all,
    }
}

/// `focusOf`: the pinned picture, else someone else's Live, the own Live,
/// someone else's camera, the own camera.
pub fn focus_of<T: Clone>(
    shares: &[T],
    cameras: &[T],
    pinned: Option<PicId>,
    me: Option<Uuid>,
    id: impl Fn(&T) -> PicId,
) -> Option<T> {
    let user = |t: &T| id(t).user;
    shares
        .iter()
        .chain(cameras)
        .find(|t| Some(id(t)) == pinned)
        .or_else(|| shares.iter().find(|t| Some(user(t)) != me))
        .or(shares.first())
        .or_else(|| cameras.iter().find(|t| Some(user(t)) != me))
        .or(cameras.first())
        .cloned()
}

/// `fitGrid`: of every number of columns, the one whose tiles come out
/// largest; ties keep the fewer columns.
pub fn fit_grid(
    count: usize,
    width: f64,
    height: f64,
    gap: f64,
    aspect: (f64, f64),
) -> Option<GridFit> {
    if count < 1 || width <= 0.0 || height <= 0.0 || width.is_nan() || height.is_nan() {
        return None;
    }
    let mut best = None;
    let mut best_area = 0.0;
    for columns in 1..=count {
        let rows = count.div_ceil(columns);
        let cell_w = (width - gap * (columns as f64 - 1.0)) / columns as f64;
        let cell_h = (height - gap * (rows as f64 - 1.0)) / rows as f64;
        if cell_w <= 0.0 || cell_h <= 0.0 {
            continue;
        }
        let tile_w = cell_w.min(cell_h * aspect.1);
        let tile_h = cell_h.min(tile_w / aspect.0);
        let area = tile_w * tile_h;
        if area > best_area + 0.5 {
            best_area = area;
            best = Some(GridFit {
                columns: columns as i32,
                rows: rows as i32,
                width: tile_w.floor() as i32,
                height: tile_h.floor() as i32,
            });
        }
    }
    best
}

/// `rasterRects`: fitGrid's tiles centred on the stage, a short last row
/// centred under the others.
pub fn raster_rects(count: usize, width: i32, height: i32) -> Vec<StageRect> {
    let gap = STAGE_GAP as i32;
    let Some(fit) = fit_grid(count, width as f64, height as f64, STAGE_GAP, TILE_ASPECT) else {
        return Vec::new();
    };
    let span = |n: i32, size: i32| n * size + (n - 1) * gap;
    let top = js_round(f64::from(height - span(fit.rows, fit.height)) / 2.0);
    (0..count as i32)
        .map(|index| {
            let row = index / fit.columns;
            let column = index - row * fit.columns;
            let in_row = fit.columns.min(count as i32 - row * fit.columns);
            let left = js_round(f64::from(width - span(in_row, fit.width)) / 2.0);
            StageRect::new(
                left + column * (fit.width + gap),
                top + row * (fit.height + gap),
                fit.width,
                fit.height,
            )
        })
        .collect()
}

/// `focusRects`: the largest 16:9 picture above a row of `strip_count`
/// small tiles, both centred.
pub fn focus_rects(
    strip_count: usize,
    width: i32,
    height: i32,
) -> Option<(StageRect, Vec<StageRect>)> {
    let gap = STAGE_GAP as i32;
    let strip = if strip_count > 0 {
        STRIP_HEIGHT + gap
    } else {
        0
    };
    let fit = fit_grid(1, width as f64, f64::from(height - strip), 0.0, WIDE)?;
    let n = strip_count as i32;
    let tile = if n == 0 {
        // JavaScript: floor(x / 0) is Infinity, min(171, ∞) = 171.
        STRIP_TILE
    } else {
        STRIP_TILE.min((f64::from(width - STRIP_GAP * (n - 1)) / f64::from(n)).floor() as i32)
    }
    .max(0);
    let row = n * tile + (n - 1) * STRIP_GAP;
    let left = js_round(f64::from(width - row) / 2.0);
    let focus = StageRect::new(
        js_round(f64::from(width - fit.width) / 2.0),
        js_round(f64::from(height - strip - fit.height) / 2.0),
        fit.width,
        fit.height,
    );
    let tiles = (0..n)
        .map(|i| {
            StageRect::new(
                left + i * (tile + STRIP_GAP),
                height - STRIP_HEIGHT,
                tile,
                STRIP_HEIGHT,
            )
        })
        .collect();
    Some((focus, tiles))
}

/// A small picture in Kino: a camera or Live, or a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thumb {
    Picture,
    Person,
}

/// `kinoRects`: the largest 16:9 picture, the others as small pictures
/// over its bottom right corner, smaller when they would not fit.
pub fn kino_rects(
    thumbs: &[Thumb],
    width: i32,
    height: i32,
) -> Option<(StageRect, Vec<StageRect>)> {
    let fit = fit_grid(1, width as f64, height as f64, 0.0, WIDE)?;
    let widths: Vec<f64> = thumbs
        .iter()
        .map(|t| match t {
            Thumb::Picture => KINO_PICTURE,
            Thumb::Person => KINO_PERSON,
        })
        .collect();
    let row = widths.iter().sum::<f64>() + KINO_GAP * (widths.len() as f64 - 1.0);
    let scale = if row > 0.0 {
        1.0_f64.min((f64::from(width) - 2.0 * KINO_RIGHT) / row)
    } else {
        1.0
    };
    let mut x = f64::from(width) - KINO_RIGHT - row * scale;
    let thumb_h = (KINO_HEIGHT * scale).floor() as i32;
    let focus = StageRect::new(
        js_round(f64::from(width - fit.width) / 2.0),
        js_round(f64::from(height - fit.height) / 2.0),
        fit.width,
        fit.height,
    );
    let strip = widths
        .iter()
        .map(|size| {
            let rect = StageRect::new(
                js_round(x),
                height - KINO_BOTTOM as i32 - thumb_h,
                (size * scale).floor() as i32,
                thumb_h,
            );
            x += (size + KINO_GAP) * scale;
            rect
        })
        .collect();
    Some((focus, strip))
}

/// `StageView`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Raster,
    Fokus,
}

/// `stageViewFor`: Fokus while someone else is live, unless picked.
pub fn view_for(picked: Option<View>, others_live: bool) -> View {
    picked.unwrap_or(if others_live {
        View::Fokus
    } else {
        View::Raster
    })
}

/// What a key does to Kino (`kinoKey`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KinoKey {
    Toggle,
    Leave,
}

/// `kinoKey`: K turns Kino on and off, Esc leaves it; keys with Strg,
/// Alt, ⌘ or Umschalt stay the shell's, and so does the push-to-talk key.
pub fn kino_key(
    key: egui::Key,
    modifiers: egui::Modifiers,
    repeat: bool,
    kino: bool,
    ptt: Option<egui::Key>,
) -> Option<KinoKey> {
    if modifiers.ctrl || modifiers.command || modifiers.mac_cmd || modifiers.alt || modifiers.shift
    {
        return None;
    }
    if key == egui::Key::Escape {
        return kino.then_some(KinoKey::Leave);
    }
    if repeat || Some(key) == ptt {
        return None;
    }
    (key == egui::Key::K).then_some(KinoKey::Toggle)
}

/// The stage's choices for one call (`useStageChoices`): they last for
/// the call, also when its page is left and opened again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Choices {
    /// `serverId/channelId` of the call the choices belong to.
    pub call: Option<(Uuid, Uuid)>,
    pub view: Option<View>,
    pub pinned: Option<PicId>,
    /// The call seen last frame: a call that ends takes its choices along.
    joined: Option<(Uuid, Uuid)>,
    pub kino: bool,
    /// Kino's controls stay awake until this time.
    pub awake_until: f64,
}

fn choices_id() -> Id {
    Id::unique("gelabber-stage-choices")
}

fn load(ctx: &Context) -> Choices {
    ctx.data(|d| d.get_temp(choices_id())).unwrap_or_default()
}

fn store(ctx: &Context, choices: Choices) {
    ctx.data_mut(|d| d.insert_temp(choices_id(), choices));
}

/// This call's choices (`choicesFor`), after dropping those of a call that
/// ended or moved since the last frame.
pub fn choices(ctx: &Context, call: (Uuid, Uuid), joined: Option<(Uuid, Uuid)>) -> Choices {
    let mut all = sync(ctx, joined);
    if all.call != Some(call) {
        // Kino is the page's, not the call's: it stays.
        all = Choices {
            joined: all.joined,
            kino: all.kino,
            awake_until: all.awake_until,
            ..Choices::default()
        };
    }
    all
}

/// Follows the running call: its end or a move resets the choices.
pub fn sync(ctx: &Context, joined: Option<(Uuid, Uuid)>) -> Choices {
    let mut all = load(ctx);
    if all.joined.is_some() && all.joined != joined {
        all = Choices::default();
    }
    all.joined = joined;
    store(ctx, all);
    all
}

/// `chooseView`.
pub fn choose_view(ctx: &Context, call: (Uuid, Uuid), view: View) {
    let mut all = load(ctx);
    if all.call != Some(call) {
        all = Choices {
            joined: all.joined,
            kino: all.kino,
            awake_until: all.awake_until,
            ..Choices::default()
        };
    }
    all.call = Some(call);
    all.view = Some(view);
    store(ctx, all);
}

/// `pinPicture`: pins a picture to the front, which is Fokus; `None` unpins
/// and keeps the view.
pub fn pin(ctx: &Context, call: (Uuid, Uuid), id: Option<PicId>) {
    let mut all = load(ctx);
    if all.call != Some(call) {
        all = Choices {
            joined: all.joined,
            kino: all.kino,
            awake_until: all.awake_until,
            ..Choices::default()
        };
    }
    all.call = Some(call);
    all.pinned = id;
    if id.is_some() {
        all.view = Some(View::Fokus);
    }
    store(ctx, all);
}

/// Kino on or off for the call's page; on wakes the controls.
pub fn set_kino(ctx: &Context, on: bool) {
    let mut all = load(ctx);
    all.kino = on;
    if on {
        all.awake_until = ctx.input(|i| i.time) + CONTROLS_IDLE;
    }
    store(ctx, all);
}

/// Pointer moves, presses and keys keep Kino's controls in sight.
pub fn wake(ctx: &Context) {
    let mut all = load(ctx);
    all.awake_until = ctx.input(|i| i.time) + CONTROLS_IDLE;
    store(ctx, all);
}

/// Whether Kino runs (the shell hides its parts for it).
pub fn kino(ctx: &Context) -> bool {
    load(ctx).kino
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> StageRect {
        StageRect::new(x, y, w, h)
    }

    #[test]
    fn raster_matches_the_web() {
        // 1280×800, stage 944×612 / 944×556 (ui-voice.md §3.3).
        assert_eq!(raster_rects(1, 944, 612), vec![r(0, 0, 944, 612)]);
        assert_eq!(raster_rects(1, 944, 556), vec![r(0, 0, 944, 556)]);
        let two = raster_rects(2, 944, 612);
        assert_eq!((two[0].w, two[0].h, two[0].y), (466, 466, 73));
        assert_eq!(raster_rects(2, 944, 556)[0].y, 45);
        let three = raster_rects(3, 944, 612);
        assert_eq!((three[0].w, three[0].h), (466, 300));
        assert_eq!((three[2].x, three[2].y), (239, 312));
        let five = raster_rects(5, 944, 612);
        assert_eq!((five[0].w, five[0].h), (306, 300));
        assert_eq!(
            five.iter().map(|r| r.x).collect::<Vec<_>>(),
            vec![1, 319, 637, 160, 478]
        );
        // 1600×900.
        assert_eq!(raster_rects(1, 1264, 656), vec![r(49, 0, 1166, 656)]);
        let three = raster_rects(3, 1264, 712);
        assert_eq!(
            three,
            vec![
                r(4, 0, 622, 350),
                r(638, 0, 622, 350),
                r(321, 362, 622, 350)
            ]
        );
        assert!(raster_rects(2, 0, 100).is_empty());
    }

    #[test]
    fn focus_matches_the_web() {
        let (focus, strip) = focus_rects(0, 944, 612).unwrap();
        assert_eq!(focus, r(0, 41, 944, 531));
        assert!(strip.is_empty());
        let (focus, strip) = focus_rects(1, 944, 612).unwrap();
        assert_eq!(focus, r(24, 0, 896, 504));
        assert_eq!(strip, vec![r(387, 516, 171, 96)]);
        let (_, strip) = focus_rects(3, 944, 612).unwrap();
        assert_eq!(
            strip.iter().map(|r| r.x).collect::<Vec<_>>(),
            vec![206, 387, 568]
        );
        let (_, strip) = focus_rects(6, 944, 612).unwrap();
        assert_eq!((strip[0].x, strip[0].w), (0, 149));
        let (_, strip) = focus_rects(8, 944, 612).unwrap();
        assert_eq!((strip[0].x, strip[0].w), (1, 109));
        let (focus, strip) = focus_rects(1, 1264, 712).unwrap();
        assert_eq!(focus, r(96, 0, 1073, 604));
        assert_eq!(strip[0], r(547, 616, 171, 96));
    }

    #[test]
    fn kino_matches_the_web() {
        let (focus, strip) = kino_rects(&[Thumb::Picture], 1280, 744).unwrap();
        assert_eq!(focus, r(0, 12, 1280, 720));
        assert_eq!(strip, vec![r(1104, 630, 160, 90)]);
        let (_, strip) = kino_rects(&[Thumb::Picture, Thumb::Person], 1280, 744).unwrap();
        assert_eq!(strip, vec![r(1006, 630, 160, 90), r(1174, 630, 90, 90)]);
        let (focus, strip) = kino_rects(&[Thumb::Picture], 1600, 844).unwrap();
        assert_eq!(focus, r(50, 0, 1500, 844));
        assert_eq!(strip[0], r(1424, 730, 160, 90));
    }

    #[test]
    fn orders_and_focus() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let id = |p: &PicId| *p;
        let shares = [PicId {
            user: a,
            kind: Kind::Live,
        }];
        let cameras = [
            PicId {
                user: a,
                kind: Kind::Camera,
            },
            PicId {
                user: b,
                kind: Kind::Camera,
            },
        ];
        let pinned = Some(cameras[1]);
        assert_eq!(
            raster_order(&shares, &cameras, pinned, id),
            vec![cameras[1], shares[0], cameras[0]]
        );
        // Someone else's Live wins over the own; the own wins over cameras.
        assert_eq!(
            focus_of(&shares, &cameras, None, Some(b), id),
            Some(shares[0])
        );
        assert_eq!(
            focus_of(&shares, &cameras, None, Some(a), id),
            Some(shares[0])
        );
        assert_eq!(focus_of(&[], &cameras, None, Some(a), id), Some(cameras[1]));
        assert_eq!(focus_of(&shares, &cameras, pinned, Some(a), id), pinned);
        let (spot, film) = arrange_stage(&shares, &cameras, None, id);
        assert_eq!(spot, Some(shares[0]));
        assert!(film.is_empty());
        assert_eq!(view_for(None, true), View::Fokus);
        assert_eq!(view_for(Some(View::Raster), true), View::Raster);
    }

    #[test]
    fn kino_keys() {
        use egui::{Key, Modifiers};
        let none = Modifiers::NONE;
        assert_eq!(
            kino_key(Key::K, none, false, false, None),
            Some(KinoKey::Toggle)
        );
        assert_eq!(
            kino_key(Key::Escape, none, false, true, None),
            Some(KinoKey::Leave)
        );
        assert_eq!(kino_key(Key::Escape, none, false, false, None), None);
        assert_eq!(kino_key(Key::K, Modifiers::CTRL, false, false, None), None);
        assert_eq!(kino_key(Key::K, none, true, false, None), None);
        assert_eq!(kino_key(Key::K, none, false, false, Some(Key::K)), None);
    }

    #[test]
    fn choices_end_with_the_call() {
        let ctx = Context::default();
        let call = (Uuid::from_u128(1), Uuid::from_u128(2));
        sync(&ctx, Some(call));
        choose_view(&ctx, call, View::Fokus);
        assert_eq!(choices(&ctx, call, Some(call)).view, Some(View::Fokus));
        let pic = PicId {
            user: Uuid::from_u128(3),
            kind: Kind::Camera,
        };
        pin(&ctx, call, Some(pic));
        pin(&ctx, call, None);
        assert_eq!(choices(&ctx, call, Some(call)).view, Some(View::Fokus));
        assert_eq!(choices(&ctx, call, None).view, None);
        // Kino belongs to the page and holds before any view is picked.
        sync(&ctx, Some(call));
        set_kino(&ctx, true);
        assert!(choices(&ctx, call, Some(call)).kino);
        assert!(kino(&ctx));
    }
}
