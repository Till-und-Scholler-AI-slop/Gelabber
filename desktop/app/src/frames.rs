//! In-page video: the native core's frames for a `<canvas>` in the server's
//! page, where the browser build has a `<video>`.
//!
//! WebKitGTK has no WebRTC and cannot take a native texture, so the page
//! pulls frames (`media_view_frame` in media.rs): one request is answered
//! with the next frame the page has not seen (long poll, raw response). The
//! page asks again only after it took a frame, so a slow page skips frames
//! here at no cost and nothing queues up.
//!
//! A consumer or source has one native sink slot. A [`Feed`] holds it and
//! hands every frame to its page views and, for a consumer, to the native
//! viewer window (viewer.rs). The core scales and paces frames to the largest
//! request among the feed's views before they get here.
//!
//! A packet is a 32-byte little-endian header and the tightly packed I420
//! planes (Y `w*h`, then U and V `ceil(w/2)*ceil(h/2)` each), limited range:
//!
//! | offset | type | field                                              |
//! |--------|------|----------------------------------------------------|
//! | 0      | [4]  | magic `GFR1`                                       |
//! | 4      | u16  | header length (32)                                 |
//! | 6      | u8   | pixel format: 0 = I420                             |
//! | 7      | u8   | flags: bit 0 BT.709 (else BT.601), bits 1-2 rotation / 90 the page applies, clockwise |
//! | 8      | u32  | width                                              |
//! | 12     | u32  | height                                             |
//! | 16     | u32  | sequence number of the view, from 1                |
//! | 20     | u32  | reserved, 0                                        |
//! | 24     | i64  | frame timestamp, microseconds                      |
use gelabber_media_core::{VideoFrame, VideoSink, VideoSinkLimits};
use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const HEADER_LEN: usize = 32;
const MAGIC: &[u8; 4] = b"GFR1";
const FORMAT_I420: u8 = 0;
const FLAG_BT709: u8 = 1;
/// What a view gets until the page said how large it draws.
pub const DEFAULT_REQUEST: VideoSinkLimits = VideoSinkLimits {
    max_width: 1280,
    max_height: 720,
    max_fps: 0,
};

type Result<T> = std::result::Result<T, String>;
/// Gets the next frame's packet, or why there will be none.
pub type Waiter = Box<dyn FnOnce(std::result::Result<Vec<u8>, &'static str>) + Send>;

const CLOSED: &str = "view closed";

fn copy_plane(out: &mut Vec<u8>, src: &[u8], stride: usize, width: usize, rows: usize) {
    if stride == width {
        out.extend_from_slice(&src[..width * rows]);
    } else {
        for row in 0..rows {
            out.extend_from_slice(&src[row * stride..row * stride + width]);
        }
    }
}

/// Writes the frame's packet into `out`. False, with `out` untouched, for a
/// frame whose planes are shorter than its size says.
fn pack(out: &mut Vec<u8>, frame: &VideoFrame<'_>, seq: u32) -> bool {
    let (width, height) = (frame.width as usize, frame.height as usize);
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    let holds = |plane: &[u8], stride: usize, width: usize, rows: usize| {
        rows > 0 && stride >= width && plane.len() >= stride * (rows - 1) + width
    };
    if !holds(frame.y, frame.stride_y, width, height)
        || !holds(frame.u, frame.stride_u, chroma_width, chroma_height)
        || !holds(frame.v, frame.stride_v, chroma_width, chroma_height)
    {
        return false;
    }
    // Video from 720 lines up is HD and BT.709 by convention (as viewer.rs);
    // scaling it down for a small view does not change its colours.
    let mut flags = if frame.source_height >= 720 {
        FLAG_BT709
    } else {
        0
    };
    flags |= (((frame.rotation / 90) & 3) as u8) << 1;
    out.clear();
    out.reserve(HEADER_LEN + width * height + 2 * chroma_width * chroma_height);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    out.push(FORMAT_I420);
    out.push(flags);
    out.extend_from_slice(&frame.width.to_le_bytes());
    out.extend_from_slice(&frame.height.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&frame.timestamp_us.to_le_bytes());
    copy_plane(out, frame.y, frame.stride_y, width, height);
    copy_plane(out, frame.u, frame.stride_u, chroma_width, chroma_height);
    copy_plane(out, frame.v, frame.stride_v, chroma_width, chroma_height);
    true
}

/// The size the core gives a `width`x`height` picture under `limits`: shrunk
/// to fit, aspect kept, even dimensions, never enlarged (`FrameCounter::Fit`
/// in gelabber_media.cc).
pub fn fit(width: u32, height: u32, limits: VideoSinkLimits) -> (u32, u32) {
    let wide = limits.max_width > 0 && width > limits.max_width;
    let tall = limits.max_height > 0 && height > limits.max_height;
    if !wide && !tall {
        return (width, height);
    }
    let (w, h) = (u64::from(width), u64::from(height));
    let (max_w, max_h) = (u64::from(limits.max_width), u64::from(limits.max_height));
    // The side that has to shrink more decides.
    let (w, h) = if wide && (!tall || max_w * h <= max_h * w) {
        (max_w, h * max_w / w)
    } else {
        (w * max_h / h, max_h)
    };
    (((w & !1).max(2)) as u32, ((h & !1).max(2)) as u32)
}

/// Drops frames above a rate the way the core does (`FrameCounter::Due`):
/// evenly, and half an interval lenient, so a stream at about the limit
/// loses nothing to jitter.
#[derive(Default)]
struct Pace {
    next: Option<Instant>,
}

impl Pace {
    fn due(&mut self, max_fps: u32, now: Instant) -> bool {
        let interval = Duration::from_secs(1) / max_fps.max(1);
        if let Some(next) = self.next {
            let ahead = next.saturating_duration_since(now);
            let behind = now.saturating_duration_since(next);
            if ahead < 2 * interval && behind < 2 * interval {
                if !ahead.is_zero() {
                    return false;
                }
                self.next = Some(next + interval);
                return true;
            }
        }
        // The first frame, or one far off the schedule: start over.
        self.next = Some(now + interval / 2);
        true
    }
}

struct Slot {
    /// Sequence number of the newest frame; 0 before the first.
    seq: u32,
    /// The newest frame while no request waits for it.
    latest: Option<Vec<u8>>,
    waiter: Option<Waiter>,
    closed: bool,
    /// What the page asked for.
    request: VideoSinkLimits,
    pace: Pace,
}

/// One canvas's view of a stream: its newest frame and the request waiting
/// for the next.
pub struct View {
    slot: Mutex<Slot>,
}

impl View {
    fn new(request: VideoSinkLimits) -> Self {
        Self {
            slot: Mutex::new(Slot {
                seq: 0,
                latest: None,
                waiter: None,
                closed: false,
                request,
                pace: Pace::default(),
            }),
        }
    }

    /// A new frame: answers the waiting request, or replaces the frame the
    /// page has not asked for yet. `feed_fps` is the rate limit the feed's
    /// frames already kept (0: none).
    fn publish(&self, frame: &VideoFrame<'_>, feed_fps: u32, now: Instant) {
        let mut slot = self.slot.lock().unwrap();
        if slot.closed {
            return;
        }
        // Another view of the feed may ask for more frames than this one.
        let own = slot.request.max_fps;
        if own != 0 && (feed_fps == 0 || own < feed_fps) && !slot.pace.due(own, now) {
            return;
        }
        let seq = match slot.seq.wrapping_add(1) {
            0 => 1,
            seq => seq,
        };
        // The frame nobody took gives its buffer to the next one.
        let waiting = slot.latest.take();
        let kept = waiting.is_some();
        let mut packet = waiting.unwrap_or_default();
        if !pack(&mut packet, frame, seq) {
            slot.latest = kept.then_some(packet);
            return;
        }
        slot.seq = seq;
        match slot.waiter.take() {
            Some(waiter) => {
                drop(slot);
                waiter(Ok(packet));
            }
            None => slot.latest = Some(packet),
        }
    }

    /// Answers with the newest frame unless the page has it (`after` is its
    /// sequence number); then with the next one when it arrives. A frame
    /// goes to the page once. A page has one request per view: a second
    /// one takes the place of the first, which fails.
    pub fn pull(&self, after: Option<u32>, waiter: Waiter) {
        let mut slot = self.slot.lock().unwrap();
        if slot.closed {
            drop(slot);
            return waiter(Err(CLOSED));
        }
        if after != Some(slot.seq)
            && let Some(packet) = slot.latest.take()
        {
            drop(slot);
            return waiter(Ok(packet));
        }
        let replaced = slot.waiter.replace(waiter);
        drop(slot);
        if let Some(replaced) = replaced {
            replaced(Err("replaced by a newer request for the view"));
        }
    }

    fn configure(&self, request: VideoSinkLimits) {
        let mut slot = self.slot.lock().unwrap();
        slot.request = request;
        slot.pace = Pace::default();
    }

    fn request(&self) -> VideoSinkLimits {
        self.slot.lock().unwrap().request
    }

    /// Ends the view: the waiting request and every later one fail.
    fn close(&self) {
        let waiter = {
            let mut slot = self.slot.lock().unwrap();
            slot.closed = true;
            slot.latest = None;
            slot.waiter.take()
        };
        if let Some(waiter) = waiter {
            waiter(Err(CLOSED));
        }
    }
}

/// The native end of a feed: the one sink slot of a consumer or source and
/// its limits. The calls wait for a frame in delivery to finish.
pub trait Tap: Send {
    fn set_sink(&mut self, sink: Option<VideoSink>) -> Result<()>;
    fn set_limits(&mut self, limits: VideoSinkLimits) -> Result<()>;
}

/// What a feed shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// A remote video consumer, by its handle in media.rs.
    Consumer(u64),
    /// A local video source (camera, screen), by its handle in media.rs.
    Source(u64),
    /// The synthetic pattern of that width, height and rate.
    Pattern(u32, u32, u32),
}

/// Where a feed's frames go. The frame thread holds this for one frame at a
/// time; nothing that waits for the frame thread may be called under it.
#[derive(Default)]
struct Targets {
    views: Vec<(u64, Arc<View>)>,
    /// The native viewer window of a consumer.
    window: Option<VideoSink>,
    /// The rate limit the core applies to the feed (0: none).
    fps: u32,
}

/// The larger of two limits, of which 0 is none.
fn larger(a: u32, b: u32) -> u32 {
    if a == 0 || b == 0 { 0 } else { a.max(b) }
}

impl Targets {
    /// The largest request among the targets.
    fn wanted(&self) -> VideoSinkLimits {
        // A viewer window shows the stream as it is.
        if self.window.is_some() {
            return VideoSinkLimits::default();
        }
        let mut requests = self.views.iter().map(|(_, view)| view.request());
        let first = requests.next().unwrap_or_default();
        requests.fold(first, |wanted, request| VideoSinkLimits {
            max_width: larger(wanted.max_width, request.max_width),
            max_height: larger(wanted.max_height, request.max_height),
            max_fps: larger(wanted.max_fps, request.max_fps),
        })
    }

    fn is_empty(&self) -> bool {
        self.views.is_empty() && self.window.is_none()
    }
}

fn deliver(targets: &Mutex<Targets>, frame: &VideoFrame<'_>) {
    let mut targets = targets.lock().unwrap();
    let now = Instant::now();
    for (_, view) in &targets.views {
        view.publish(frame, targets.fps, now);
    }
    if let Some(window) = targets.window.as_mut() {
        window(frame);
    }
}

/// One consumer's or source's frames, fanned out to its views.
struct Feed {
    targets: Arc<Mutex<Targets>>,
    tap: Box<dyn Tap>,
    /// The limits the tap has, once its sink is set.
    applied: Option<VideoSinkLimits>,
}

impl Feed {
    /// Brings the tap in line with the targets: the sink with the first
    /// target, the limits whenever the largest request changed.
    fn sync(&mut self) -> Result<()> {
        let wanted = self.targets.lock().unwrap().wanted();
        if self.applied == Some(wanted) {
            return Ok(());
        }
        // Limits first, so that the sink's first frame already keeps them.
        self.tap.set_limits(wanted)?;
        self.targets.lock().unwrap().fps = wanted.max_fps;
        if self.applied.is_none() {
            let targets = self.targets.clone();
            self.tap
                .set_sink(Some(Box::new(move |frame: &VideoFrame<'_>| {
                    deliver(&targets, frame)
                })))?;
        }
        self.applied = Some(wanted);
        Ok(())
    }
}

#[derive(Default)]
struct Registry {
    next: AtomicU64,
    /// Held across a feed's tap calls, which wait for the frame thread, so
    /// that one origin never has two sinks coming and going at once. The
    /// frame thread itself only takes a feed's targets.
    feeds: Mutex<HashMap<Origin, Feed>>,
    /// For the frame requests; never held across a tap call.
    views: Mutex<HashMap<u64, (Origin, Arc<View>)>>,
}

type Feeds = HashMap<Origin, Feed>;

impl Registry {
    /// The feed of `origin`, started on `tap` when there is none yet.
    fn feed(
        feeds: &mut Feeds,
        origin: Origin,
        tap: impl FnOnce() -> Result<Box<dyn Tap>>,
    ) -> Result<&mut Feed> {
        Ok(match feeds.entry(origin) {
            Entry::Occupied(feed) => feed.into_mut(),
            Entry::Vacant(free) => free.insert(Feed {
                targets: Arc::default(),
                tap: tap()?,
                applied: None,
            }),
        })
    }

    /// After a feed's targets changed: it follows their largest request, or
    /// ends when none is left.
    fn settle(&self, feeds: &mut Feeds, origin: Origin) -> Result<()> {
        let Some(feed) = feeds.get_mut(&origin) else {
            return Ok(());
        };
        if !feed.targets.lock().unwrap().is_empty() {
            return feed.sync();
        }
        if let Some(feed) = feeds.remove(&origin) {
            self.end(feed);
        }
        Ok(())
    }

    /// Closes a feed's views and takes its sink off the tap; no frame
    /// arrives once this returns.
    fn end(&self, mut feed: Feed) {
        let (views, window) = {
            let mut targets = feed.targets.lock().unwrap();
            (std::mem::take(&mut targets.views), targets.window.take())
        };
        drop(window);
        {
            let mut known = self.views.lock().unwrap();
            for (handle, _) in &views {
                known.remove(handle);
            }
        }
        for (_, view) in views {
            view.close();
        }
        if feed.applied.is_some() {
            let _ = feed.tap.set_sink(None);
        }
    }
}

/// The app's feeds and page views. Everything but [`Frames::view`] may wait
/// for a frame in delivery, so commands call it off the async runtime.
#[derive(Clone, Default)]
pub struct Frames(Arc<Registry>);

impl Frames {
    /// Opens a page view of `origin`; `tap` makes the native end when this
    /// is the origin's first target.
    pub fn open(
        &self,
        origin: Origin,
        request: VideoSinkLimits,
        tap: impl FnOnce() -> Result<Box<dyn Tap>>,
    ) -> Result<u64> {
        let handle = self.0.next.fetch_add(1, Ordering::Relaxed) + 1;
        let view = Arc::new(View::new(request));
        let mut feeds = self.0.feeds.lock().unwrap();
        let targets = Registry::feed(&mut feeds, origin, tap)?.targets.clone();
        targets.lock().unwrap().views.push((handle, view.clone()));
        if let Err(error) = self.0.settle(&mut feeds, origin) {
            targets.lock().unwrap().views.retain(|(h, _)| *h != handle);
            let _ = self.0.settle(&mut feeds, origin);
            return Err(error);
        }
        self.0.views.lock().unwrap().insert(handle, (origin, view));
        Ok(handle)
    }

    /// The view for a frame request.
    pub fn view(&self, handle: u64) -> Result<Arc<View>> {
        self.entry(handle).map(|(_, view)| view)
    }

    fn entry(&self, handle: u64) -> Result<(Origin, Arc<View>)> {
        let views = self.0.views.lock().unwrap();
        let entry = views.get(&handle).cloned();
        entry.ok_or_else(|| format!("unknown view {handle}"))
    }

    /// What the page can use now; the feed follows its largest request.
    pub fn configure(&self, handle: u64, request: VideoSinkLimits) -> Result<()> {
        let (origin, view) = self.entry(handle)?;
        view.configure(request);
        self.0.settle(&mut self.0.feeds.lock().unwrap(), origin)
    }

    /// Closes a view; the last target of a feed takes the native sink with it.
    pub fn close(&self, handle: u64) {
        let Some((origin, view)) = self.0.views.lock().unwrap().remove(&handle) else {
            return;
        };
        view.close();
        let mut feeds = self.0.feeds.lock().unwrap();
        if let Some(feed) = feeds.get(&origin) {
            let mut targets = feed.targets.lock().unwrap();
            targets.views.retain(|(view, _)| *view != handle);
        }
        let _ = self.0.settle(&mut feeds, origin);
    }

    /// Feeds the native viewer window of `origin`, alone or next to its
    /// page views.
    pub fn set_window(
        &self,
        origin: Origin,
        sink: VideoSink,
        tap: impl FnOnce() -> Result<Box<dyn Tap>>,
    ) -> Result<()> {
        let mut feeds = self.0.feeds.lock().unwrap();
        let targets = Registry::feed(&mut feeds, origin, tap)?.targets.clone();
        let previous = targets.lock().unwrap().window.replace(sink);
        drop(previous);
        let settled = self.0.settle(&mut feeds, origin);
        if settled.is_err() {
            let failed = targets.lock().unwrap().window.take();
            drop(failed);
            let _ = self.0.settle(&mut feeds, origin);
        }
        settled
    }

    pub fn clear_window(&self, origin: Origin) {
        let mut feeds = self.0.feeds.lock().unwrap();
        if let Some(feed) = feeds.get(&origin) {
            let window = feed.targets.lock().unwrap().window.take();
            drop(window);
        }
        let _ = self.0.settle(&mut feeds, origin);
    }

    /// The consumer or source went away: its views end.
    pub fn close_origin(&self, origin: Origin) {
        let mut feeds = self.0.feeds.lock().unwrap();
        if let Some(feed) = feeds.remove(&origin) {
            self.0.end(feed);
        }
    }

    /// Ends every view and feed of a page that went away.
    pub fn reset(&self) {
        let mut feeds = self.0.feeds.lock().unwrap();
        for (_, feed) in feeds.drain() {
            self.0.end(feed);
        }
    }
}

/// Smoke tests and benches: colour bars with a moving box and the frame
/// number as a row of black and white squares, timestamped with the wall
/// clock so a page can measure how old a frame is when it draws it. Keeps a
/// view's limits like the core does: the pattern is drawn at the size they
/// leave, at most at the rate they allow.
pub struct TestPattern {
    width: u32,
    height: u32,
    fps: u32,
    limits: Arc<Mutex<VideoSinkLimits>>,
    worker: Option<(Arc<AtomicBool>, JoinHandle<()>)>,
}

impl TestPattern {
    pub fn new(width: u32, height: u32, fps: u32) -> Result<Self> {
        if !(16..=3840).contains(&width)
            || !(16..=2160).contains(&height)
            || !(1..=120).contains(&fps)
        {
            return Err("test pattern size or rate out of range".into());
        }
        Ok(Self {
            width: width & !1,
            height: height & !1,
            fps,
            limits: Arc::default(),
            worker: None,
        })
    }

    fn stop(&mut self) {
        if let Some((stop, worker)) = self.worker.take() {
            stop.store(true, Ordering::Relaxed);
            let _ = worker.join();
        }
    }
}

impl Drop for TestPattern {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Tap for TestPattern {
    fn set_sink(&mut self, sink: Option<VideoSink>) -> Result<()> {
        self.stop();
        let Some(sink) = sink else {
            return Ok(());
        };
        let stop = Arc::new(AtomicBool::new(false));
        let (stopped, limits) = (stop.clone(), self.limits.clone());
        let (width, height, fps) = (self.width, self.height, self.fps);
        let worker = std::thread::Builder::new()
            .name("gelabber-test-pattern".into())
            .spawn(move || draw_pattern(width, height, fps, &limits, &stopped, sink))
            .map_err(|e| e.to_string())?;
        self.worker = Some((stop, worker));
        Ok(())
    }

    fn set_limits(&mut self, limits: VideoSinkLimits) -> Result<()> {
        *self.limits.lock().unwrap() = limits;
        Ok(())
    }
}

/// Limited-range Y, Cb, Cr of an RGB colour (0..1 per channel).
fn yuv(rgb: [f64; 3], bt709: bool) -> [u8; 3] {
    let (kr, kb) = if bt709 {
        (0.2126, 0.0722)
    } else {
        (0.299, 0.114)
    };
    let luma = kr * rgb[0] + (1.0 - kr - kb) * rgb[1] + kb * rgb[2];
    let cb = (rgb[2] - luma) / (2.0 * (1.0 - kb));
    let cr = (rgb[0] - luma) / (2.0 * (1.0 - kr));
    [
        (16.0 + 219.0 * luma).round() as u8,
        (128.0 + 224.0 * cb).round() as u8,
        (128.0 + 224.0 * cr).round() as u8,
    ]
}

/// White, yellow, cyan, green, magenta, red, blue, black at full level.
const BARS: [[f64; 3]; 8] = [
    [1.0, 1.0, 1.0],
    [1.0, 1.0, 0.0],
    [0.0, 1.0, 1.0],
    [0.0, 1.0, 0.0],
    [1.0, 0.0, 1.0],
    [1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, 0.0],
];

struct Planes {
    width: usize,
    height: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Planes {
    /// Bars over the top three quarters, mid grey below.
    fn bars(width: usize, height: usize, bt709: bool) -> Self {
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        let mut planes = Self {
            width,
            height,
            y: vec![0; width * height],
            u: vec![0; cw * ch],
            v: vec![0; cw * ch],
        };
        let grey = yuv([0.5, 0.5, 0.5], bt709);
        let split = height * 3 / 4;
        for (bar, rgb) in BARS.iter().enumerate() {
            let color = yuv(*rgb, bt709);
            planes.fill(bar * width / 8, 0, (bar + 1) * width / 8, split, color);
        }
        planes.fill(0, split, width, height, grey);
        planes
    }

    /// Fills a rectangle (even coordinates keep chroma exact).
    fn fill(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, color: [u8; 3]) {
        let (x1, y1) = (x1.min(self.width), y1.min(self.height));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for row in y0..y1 {
            self.y[row * self.width + x0..row * self.width + x1].fill(color[0]);
        }
        let cw = self.width.div_ceil(2);
        for row in y0 / 2..y1.div_ceil(2) {
            self.u[row * cw + x0 / 2..row * cw + x1.div_ceil(2)].fill(color[1]);
            self.v[row * cw + x0 / 2..row * cw + x1.div_ceil(2)].fill(color[2]);
        }
    }
}

fn draw_pattern(
    width: u32,
    height: u32,
    fps: u32,
    limits: &Mutex<VideoSinkLimits>,
    stop: &AtomicBool,
    mut sink: VideoSink,
) {
    // The colours of the full-size picture, whatever size is drawn.
    let bt709 = height >= 720;
    let (white, black) = (yuv([1.0; 3], bt709), yuv([0.0; 3], bt709));
    let mut template = Planes::bars(0, 0, bt709);
    let mut frame = Planes::bars(0, 0, bt709);
    let mut next = Instant::now();
    let mut number = 0u32;
    while !stop.load(Ordering::Relaxed) {
        let limits = *limits.lock().unwrap();
        let (w, h) = fit(width, height, limits);
        let (w, h) = (w as usize, h as usize);
        if (template.width, template.height) != (w, h) {
            template = Planes::bars(w, h, bt709);
            frame = Planes::bars(w, h, bt709);
        }
        let rate = match limits.max_fps {
            0 => fps,
            max => fps.min(max),
        };
        number = number.wrapping_add(1);
        // The lower quarter changes: a box crossing once every two seconds
        // and 16 squares showing the frame number in binary.
        let band = (h * 3 / 4) & !1;
        let cell = ((h - band) / 2) & !1;
        let digit = (w / 16) & !1;
        frame.y[band * w..].copy_from_slice(&template.y[band * w..]);
        let chroma = (band / 2) * (w / 2);
        frame.u[chroma..].copy_from_slice(&template.u[chroma..]);
        frame.v[chroma..].copy_from_slice(&template.v[chroma..]);
        if cell > 0 && digit > 2 && w > cell {
            let travel = (w - cell) as u64;
            let x = ((u64::from(number) * travel / (2 * u64::from(rate))) % travel) as usize & !1;
            frame.fill(x, band, x + cell, band + cell, white);
            for bit in 0..16 {
                let color = if number >> (15 - bit) & 1 == 1 {
                    white
                } else {
                    black
                };
                frame.fill(bit * digit, band + cell, (bit + 1) * digit - 2, h, color);
            }
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0);
        sink(&VideoFrame {
            width: w as u32,
            height: h as u32,
            y: &frame.y,
            u: &frame.u,
            v: &frame.v,
            stride_y: w,
            stride_u: w / 2,
            stride_v: w / 2,
            rotation: 0,
            timestamp_us: now,
            source_width: width,
            source_height: height,
        });
        next += Duration::from_secs(1) / rate;
        match next.checked_duration_since(Instant::now()) {
            Some(wait) => std::thread::sleep(wait),
            // Fell behind: do not burst to catch up.
            None => next = Instant::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn limits(max_width: u32, max_height: u32, max_fps: u32) -> VideoSinkLimits {
        VideoSinkLimits {
            max_width,
            max_height,
            max_fps,
        }
    }

    fn frame<'a>(planes: &'a Planes, timestamp_us: i64) -> VideoFrame<'a> {
        VideoFrame {
            width: planes.width as u32,
            height: planes.height as u32,
            y: &planes.y,
            u: &planes.u,
            v: &planes.v,
            stride_y: planes.width,
            stride_u: planes.width.div_ceil(2),
            stride_v: planes.width.div_ceil(2),
            rotation: 0,
            timestamp_us,
            source_width: planes.width as u32,
            source_height: planes.height as u32,
        }
    }

    fn seq(packet: &[u8]) -> u32 {
        u32::from_le_bytes(packet[16..20].try_into().unwrap())
    }

    fn size(packet: &[u8]) -> (u32, u32) {
        (
            u32::from_le_bytes(packet[8..12].try_into().unwrap()),
            u32::from_le_bytes(packet[12..16].try_into().unwrap()),
        )
    }

    type Pulled = mpsc::Receiver<std::result::Result<Vec<u8>, &'static str>>;

    /// One frame request; its answer arrives on the channel.
    fn pull(view: &View, after: Option<u32>) -> Pulled {
        let (tx, rx) = mpsc::channel();
        view.pull(after, Box::new(move |answer| tx.send(answer).unwrap()));
        rx
    }

    #[test]
    fn packet_is_header_and_packed_planes() {
        let planes = Planes::bars(64, 36, false);
        // Padded strides must not reach the packet.
        let mut y = Vec::new();
        for row in planes.y.chunks(64) {
            y.extend_from_slice(row);
            y.extend_from_slice(&[0xee; 16]);
        }
        let mut padded = frame(&planes, 123_456_789);
        padded.y = &y;
        padded.stride_y = 80;
        let mut packet = vec![0xaa; 7];
        assert!(pack(&mut packet, &padded, 7));
        assert_eq!(packet.len(), HEADER_LEN + 64 * 36 * 3 / 2);
        assert_eq!(&packet[..4], b"GFR1");
        assert_eq!(u16::from_le_bytes([packet[4], packet[5]]), 32);
        assert_eq!((packet[6], packet[7]), (FORMAT_I420, 0));
        assert_eq!(size(&packet), (64, 36));
        assert_eq!(seq(&packet), 7);
        assert_eq!(&packet[20..24], &[0; 4]);
        assert_eq!(
            i64::from_le_bytes(packet[24..32].try_into().unwrap()),
            123_456_789
        );
        assert_eq!(&packet[HEADER_LEN..HEADER_LEN + 64 * 36], &planes.y[..]);
        assert_eq!(&packet[HEADER_LEN + 64 * 36..][..32 * 18], &planes.u[..]);
        assert_eq!(&packet[HEADER_LEN + 64 * 36 + 32 * 18..], &planes.v[..]);
        assert!(!packet.contains(&0xee));

        // Odd sizes round the chroma planes up.
        let odd = Planes::bars(33, 19, false);
        assert!(pack(&mut packet, &frame(&odd, 0), 1));
        assert_eq!(packet.len(), HEADER_LEN + 33 * 19 + 2 * 17 * 10);

        // 720 lines and up are BT.709, also when scaled down for a small
        // view; the rotation is in bits 1 and 2.
        assert!(pack(
            &mut packet,
            &frame(&Planes::bars(1280, 720, true), 0),
            1
        ));
        assert_eq!(packet[7], FLAG_BT709);
        let mut scaled = frame(&planes, 0);
        scaled.source_height = 1080;
        scaled.rotation = 270;
        assert!(pack(&mut packet, &scaled, 1));
        assert_eq!(packet[7], FLAG_BT709 | 3 << 1);
        scaled.source_height = 480;
        scaled.rotation = 90;
        assert!(pack(&mut packet, &scaled, 1));
        assert_eq!(packet[7], 1 << 1);

        // A plane shorter than its size says is dropped, not read past.
        let before = packet.clone();
        let mut short = frame(&planes, 0);
        short.v = &planes.v[..10];
        assert!(!pack(&mut packet, &short, 2));
        assert_eq!(packet, before);
    }

    #[test]
    fn a_request_gets_the_newest_frame_once_then_waits_for_the_next() {
        let view = View::new(DEFAULT_REQUEST);
        let planes = Planes::bars(32, 18, false);
        let now = Instant::now();
        let publish = |timestamp| view.publish(&frame(&planes, timestamp), 0, now);
        // Nothing yet: the request waits, and gets the first frame as number 1.
        let waiting = pull(&view, None);
        assert!(waiting.try_recv().is_err());
        publish(1);
        assert_eq!(seq(&waiting.try_recv().unwrap().unwrap()), 1);
        // Frames the page was too slow for are skipped, not queued.
        publish(2);
        publish(3);
        let newest = pull(&view, Some(1)).try_recv().unwrap().unwrap();
        assert_eq!(seq(&newest), 3);
        assert_eq!(i64::from_le_bytes(newest[24..32].try_into().unwrap()), 3);
        // The page has frame 3: the request waits for the next one.
        let waiting = pull(&view, Some(3));
        assert!(waiting.try_recv().is_err());
        // A second request takes its place.
        let second = pull(&view, Some(3));
        assert!(waiting.try_recv().unwrap().is_err());
        publish(4);
        assert_eq!(seq(&second.try_recv().unwrap().unwrap()), 4);
        // A frame the page did not ask for yet is there at once, whatever
        // the page says it has.
        publish(5);
        assert_eq!(seq(&pull(&view, None).try_recv().unwrap().unwrap()), 5);
        // A frame with planes too short to copy changes nothing.
        publish(6);
        let mut short = frame(&planes, 7);
        short.u = &planes.u[..3];
        view.publish(&short, 0, now);
        assert_eq!(seq(&pull(&view, Some(5)).try_recv().unwrap().unwrap()), 6);
        // Closing fails the waiting request and every later one.
        let waiting = pull(&view, Some(6));
        view.close();
        assert_eq!(waiting.try_recv().unwrap(), Err(CLOSED));
        assert_eq!(pull(&view, None).try_recv().unwrap(), Err(CLOSED));
        publish(8);
    }

    #[test]
    fn a_view_keeps_its_own_rate_below_the_feeds() {
        let planes = Planes::bars(32, 18, false);
        let start = Instant::now();
        // 60 frames a second for one second.
        let count = |request: VideoSinkLimits, feed_fps: u32| {
            let view = View::new(request);
            for tick in 0..60u64 {
                let now = start + Duration::from_micros(tick * 16_667);
                view.publish(&frame(&planes, 0), feed_fps, now);
            }
            view.slot.lock().unwrap().seq
        };
        assert_eq!(count(limits(0, 0, 0), 0), 60);
        // The first frame and every fourth after it.
        assert_eq!(count(limits(0, 0, 15), 0), 16);
        assert_eq!(count(limits(0, 0, 15), 60), 16);
        // The core already kept this rate: nothing is dropped twice.
        assert_eq!(count(limits(0, 0, 60), 60), 60);
        assert_eq!(count(limits(0, 0, 30), 30), 60);
        // A stream at the limit keeps every frame despite jitter.
        let view = View::new(limits(0, 0, 30));
        for tick in 0..30u64 {
            let jitter = if tick % 2 == 0 { 0 } else { 9_000 };
            let now = start + Duration::from_micros(tick * 33_333 + jitter);
            view.publish(&frame(&planes, 0), 0, now);
        }
        assert_eq!(view.slot.lock().unwrap().seq, 30);
    }

    #[test]
    fn fit_shrinks_to_even_sizes_and_never_enlarges() {
        for (source, wanted, expected) in [
            ((640, 360), limits(320, 180, 0), (320, 180)),
            ((640, 360), limits(300, 300, 0), (300, 168)),
            ((640, 360), limits(0, 101, 0), (178, 100)),
            ((640, 360), limits(333, 0, 0), (332, 186)),
            ((640, 360), limits(640, 360, 0), (640, 360)),
            ((640, 360), limits(4000, 4000, 0), (640, 360)),
            ((640, 360), limits(1, 1, 0), (2, 2)),
            ((641, 361), limits(0, 0, 0), (641, 361)),
            ((3840, 2160), limits(1280, 720, 0), (1280, 720)),
            ((3840, 1600), limits(1280, 720, 0), (1280, 532)),
            ((1080, 1920), limits(1280, 720, 0), (404, 720)),
        ] {
            assert_eq!(
                fit(source.0, source.1, wanted),
                expected,
                "{source:?} {wanted:?}"
            );
        }
    }

    /// A tap that records what the feed asks of it and lets the test play
    /// the frame thread.
    #[derive(Clone, Default)]
    struct Probe(Arc<Mutex<ProbeState>>);

    #[derive(Default)]
    struct ProbeState {
        sink: Option<VideoSink>,
        limits: Vec<VideoSinkLimits>,
        installed: u32,
        removed: u32,
        fail_sink: bool,
    }

    impl Tap for Probe {
        fn set_sink(&mut self, sink: Option<VideoSink>) -> Result<()> {
            let mut state = self.0.lock().unwrap();
            if state.fail_sink && sink.is_some() {
                return Err("no video here".into());
            }
            match sink {
                Some(_) => state.installed += 1,
                None => state.removed += 1,
            }
            state.sink = sink;
            Ok(())
        }

        fn set_limits(&mut self, limits: VideoSinkLimits) -> Result<()> {
            self.0.lock().unwrap().limits.push(limits);
            Ok(())
        }
    }

    impl Probe {
        fn tap(&self) -> impl FnOnce() -> Result<Box<dyn Tap>> + use<> {
            let tap = self.clone();
            move || Ok(Box::new(tap) as Box<dyn Tap>)
        }

        /// A frame from the core; false without a sink.
        fn push(&self, planes: &Planes) -> bool {
            let mut state = self.0.lock().unwrap();
            let Some(sink) = state.sink.as_mut() else {
                return false;
            };
            sink(&frame(planes, 0));
            true
        }

        fn limits(&self) -> VideoSinkLimits {
            *self.0.lock().unwrap().limits.last().unwrap()
        }

        fn counts(&self) -> (u32, u32) {
            let state = self.0.lock().unwrap();
            (state.installed, state.removed)
        }
    }

    fn unused_tap() -> Result<Box<dyn Tap>> {
        panic!("the feed exists")
    }

    #[test]
    fn a_feed_serves_its_views_from_one_sink_at_the_largest_request() {
        let frames = Frames::default();
        let probe = Probe::default();
        let planes = Planes::bars(32, 18, false);
        let origin = Origin::Consumer(5);

        // Until the page says more, a view gets at most 1280x720.
        let first = frames.open(origin, DEFAULT_REQUEST, probe.tap()).unwrap();
        assert_eq!(probe.counts(), (1, 0));
        assert_eq!(probe.limits(), limits(1280, 720, 0));
        // More views share the sink; each has its own numbering.
        assert!(probe.push(&planes));
        let second = frames
            .open(origin, limits(1920, 400, 30), unused_tap)
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(probe.counts(), (1, 0));
        // The largest request per side; one view without a rate limit
        // leaves the feed without one.
        assert_eq!(probe.limits(), limits(1920, 720, 0));
        assert!(probe.push(&planes));
        let (a, b) = (frames.view(first).unwrap(), frames.view(second).unwrap());
        assert_eq!(seq(&pull(&a, None).try_recv().unwrap().unwrap()), 2);
        assert_eq!(seq(&pull(&b, None).try_recv().unwrap().unwrap()), 1);

        // A view's new size moves the feed's limits only when it changes
        // the largest request.
        frames.configure(first, limits(640, 360, 15)).unwrap();
        assert_eq!(probe.limits(), limits(1920, 400, 30));
        let changes = probe.0.lock().unwrap().limits.len();
        frames.configure(first, limits(320, 180, 15)).unwrap();
        assert_eq!(probe.0.lock().unwrap().limits.len(), changes);
        assert!(frames.configure(99, limits(1, 1, 0)).is_err());

        // Closing one view leaves the other running at its own request.
        let waiting = pull(&b, Some(1));
        frames.close(second);
        assert_eq!(waiting.try_recv().unwrap(), Err(CLOSED));
        assert!(frames.view(second).is_err());
        assert_eq!(probe.limits(), limits(320, 180, 15));
        assert_eq!(probe.counts(), (1, 0));
        assert!(probe.push(&planes));
        // The last view takes the sink with it; closing twice is fine.
        frames.close(first);
        frames.close(first);
        assert_eq!(probe.counts(), (1, 1));
        assert!(!probe.push(&planes));
        assert!(frames.0.feeds.lock().unwrap().is_empty());
        assert!(frames.0.views.lock().unwrap().is_empty());

        // The next view starts a new feed.
        let again = frames.open(origin, DEFAULT_REQUEST, probe.tap()).unwrap();
        assert_eq!(probe.counts(), (2, 1));
        assert!(again > second);
        frames.close(again);
    }

    #[test]
    fn a_viewer_window_shares_the_feed_with_page_views() {
        let frames = Frames::default();
        let probe = Probe::default();
        let planes = Planes::bars(32, 18, false);
        let origin = Origin::Consumer(8);
        let (shown_tx, shown) = mpsc::channel();
        let window = |tx: mpsc::Sender<(u32, u32)>| -> VideoSink {
            Box::new(move |frame: &VideoFrame<'_>| tx.send((frame.width, frame.height)).unwrap())
        };

        // The window alone, as with a web client that knows no page views:
        // the stream as it is.
        frames
            .set_window(origin, window(shown_tx.clone()), probe.tap())
            .unwrap();
        assert_eq!(probe.counts(), (1, 0));
        assert_eq!(probe.limits(), limits(0, 0, 0));
        assert!(probe.push(&planes));
        assert_eq!(shown.try_recv().unwrap(), (32, 18));

        // A page view next to it gets the same frames, unscaled.
        let view = frames
            .open(origin, limits(320, 180, 15), unused_tap)
            .unwrap();
        assert_eq!(probe.counts(), (1, 0));
        assert_eq!(probe.limits(), limits(0, 0, 0));
        assert!(probe.push(&planes));
        assert_eq!(shown.try_recv().unwrap(), (32, 18));
        let packet = pull(&frames.view(view).unwrap(), None);
        assert_eq!(size(&packet.try_recv().unwrap().unwrap()), (32, 18));

        // Opening the window again replaces its sink, not the feed's.
        let (again_tx, again) = mpsc::channel();
        frames
            .set_window(origin, window(again_tx), unused_tap)
            .unwrap();
        assert_eq!(probe.counts(), (1, 0));
        assert!(probe.push(&planes));
        assert!(shown.try_recv().is_err());
        assert_eq!(again.try_recv().unwrap(), (32, 18));

        // Without the window the page view's request counts again.
        frames.clear_window(origin);
        assert_eq!(probe.counts(), (1, 0));
        assert_eq!(probe.limits(), limits(320, 180, 15));
        assert!(probe.push(&planes));
        assert!(again.try_recv().is_err());

        // And the other way round: the view goes, the window stays.
        frames
            .set_window(origin, window(shown_tx), unused_tap)
            .unwrap();
        frames.close(view);
        assert_eq!(probe.counts(), (1, 0));
        assert!(probe.push(&planes));
        assert_eq!(shown.try_recv().unwrap(), (32, 18));
        frames.clear_window(origin);
        assert_eq!(probe.counts(), (1, 1));
        // Closing a window that is not open is fine.
        frames.clear_window(origin);
        frames.clear_window(Origin::Consumer(9));
        assert_eq!(probe.counts(), (1, 1));
    }

    #[test]
    fn views_end_with_their_origin_and_with_the_page() {
        let frames = Frames::default();
        let (consumer, source) = (Probe::default(), Probe::default());
        let planes = Planes::bars(32, 18, false);
        let remote = frames
            .open(Origin::Consumer(1), DEFAULT_REQUEST, consumer.tap())
            .unwrap();
        let own = frames
            .open(Origin::Source(1), DEFAULT_REQUEST, source.tap())
            .unwrap();
        let mirror = frames
            .open(Origin::Source(1), DEFAULT_REQUEST, unused_tap)
            .unwrap();
        frames
            .set_window(Origin::Consumer(1), Box::new(|_| {}), unused_tap)
            .unwrap();

        // The source is closed: its views end, the consumer's stay.
        let waiting = [own, mirror].map(|view| pull(&frames.view(view).unwrap(), None));
        frames.close_origin(Origin::Source(1));
        for request in waiting {
            assert_eq!(request.try_recv().unwrap(), Err(CLOSED));
        }
        assert_eq!(source.counts(), (1, 1));
        assert!(!source.push(&planes));
        assert!(frames.view(own).is_err() && frames.view(mirror).is_err());
        assert!(frames.view(remote).is_ok());
        assert_eq!(consumer.counts(), (1, 0));
        frames.close_origin(Origin::Source(1));

        // The page reloads: everything ends, requests fail, no sink is left.
        let waiting = pull(&frames.view(remote).unwrap(), None);
        frames.reset();
        assert_eq!(waiting.try_recv().unwrap(), Err(CLOSED));
        assert_eq!(consumer.counts(), (1, 1));
        assert!(!consumer.push(&planes));
        assert!(frames.view(remote).is_err());
        assert!(frames.0.feeds.lock().unwrap().is_empty());
        assert!(frames.0.views.lock().unwrap().is_empty());
        frames.reset();
    }

    #[test]
    fn a_feed_that_cannot_start_leaves_nothing_behind() {
        let frames = Frames::default();
        // No consumer or source behind the handle.
        let error = frames
            .open(Origin::Source(3), DEFAULT_REQUEST, || {
                Err("unknown source 3".into())
            })
            .err();
        assert_eq!(error.as_deref(), Some("unknown source 3"));
        // An audio source: the core refuses the sink.
        let probe = Probe::default();
        probe.0.lock().unwrap().fail_sink = true;
        assert!(
            frames
                .open(Origin::Source(4), DEFAULT_REQUEST, probe.tap())
                .is_err()
        );
        assert!(
            frames
                .set_window(Origin::Consumer(4), Box::new(|_| {}), probe.tap())
                .is_err()
        );
        assert_eq!(probe.counts(), (0, 0));
        assert!(frames.0.feeds.lock().unwrap().is_empty());
        assert!(frames.0.views.lock().unwrap().is_empty());
    }

    fn pixel(packet: &[u8], x: usize, y: usize) -> [u8; 3] {
        let (width, height) = size(packet);
        let (width, height) = (width as usize, height as usize);
        let chroma = HEADER_LEN + width * height + (y / 2) * (width / 2) + x / 2;
        [
            packet[HEADER_LEN + y * width + x],
            packet[chroma],
            packet[chroma + (width / 2) * (height / 2)],
        ]
    }

    #[test]
    fn the_test_pattern_feeds_views_and_stops_with_the_last() {
        let frames = Frames::default();
        let origin = Origin::Pattern(1280, 720, 60);
        let pattern = || Ok(Box::new(TestPattern::new(1280, 720, 60)?) as Box<dyn Tap>);
        let threads = || {
            std::fs::read_dir("/proc/self/task")
                .map(|tasks| {
                    tasks
                        .filter_map(|task| {
                            std::fs::read_to_string(task.ok()?.path().join("comm")).ok()
                        })
                        .filter(|name| name.trim() == "gelabber-test-p")
                        .count()
                })
                .unwrap_or(0)
        };
        let view = frames.open(origin, limits(640, 360, 0), pattern).unwrap();
        let next = |after| {
            pull(&frames.view(view).unwrap(), after)
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap()
        };
        // Drawn at the size the view asked for, in the full picture's
        // colours (BT.709 from 720 lines).
        let packet = next(None);
        assert_eq!(size(&packet), (640, 360));
        assert_eq!(packet[7], FLAG_BT709);
        assert_eq!(pixel(&packet, 40, 100), yuv([1.0, 1.0, 1.0], true));
        assert_eq!(pixel(&packet, 440, 100), yuv([1.0, 0.0, 0.0], true));
        assert_eq!(pixel(&packet, 600, 100), [16, 128, 128]);
        // The view grows: the next frames follow.
        frames.configure(view, limits(4000, 4000, 0)).unwrap();
        let mut last = seq(&packet);
        let grown = loop {
            let packet = next(Some(last));
            assert!(seq(&packet) > last);
            last = seq(&packet);
            if size(&packet) == (1280, 720) {
                break packet;
            }
        };
        assert_eq!(pixel(&grown, 880, 200), yuv([1.0, 0.0, 0.0], true));
        if cfg!(target_os = "linux") {
            assert_eq!(threads(), 1);
        }
        // The last view ends the pattern's thread.
        frames.close(view);
        assert_eq!(threads(), 0);
        assert!(TestPattern::new(8, 8, 30).is_err());
        assert!(TestPattern::new(1280, 720, 0).is_err());
    }

    #[test]
    fn bars_have_the_nominal_colours() {
        assert_eq!(yuv([1.0, 1.0, 1.0], true), [235, 128, 128]);
        assert_eq!(yuv([0.0, 0.0, 0.0], false), [16, 128, 128]);
        assert_eq!(yuv([1.0, 0.0, 0.0], false), [81, 90, 240]);
        assert_eq!(yuv([0.0, 0.0, 1.0], true), [32, 240, 118]);
    }
}
