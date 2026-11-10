//! Video tiles on the GPU: an I420 [`Frame`] of the call is uploaded as
//! three R8 planes and turned into RGB by the shader of the Tauri app's
//! viewer (viewer.rs: BT.601/709 limited range, clockwise
//! quarter turns), drawn by an egui_wgpu paint callback inside the egui
//! pass. A camera covers its tile, a Live is contained in it; the tile's
//! rounded corners are cut in the shader.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor, wgpu};
use egui::{Rect, Ui, vec2};
use gelabber_call::Frame;

/// How a picture fills its tile (`object-fit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Cameras: cut to the tile.
    Cover,
    /// Lives, the focus and fullscreen: all of it, letterboxed.
    Contain,
}

const SHADER: &str = r#"
@group(0) @binding(0) var plane_y: texture_2d<f32>;
@group(0) @binding(1) var plane_u: texture_2d<f32>;
@group(0) @binding(2) var plane_v: texture_2d<f32>;
@group(0) @binding(3) var bilinear: sampler;
// a: BT.709 (1) or BT.601 (0), clockwise quarter turns, mirror, sRGB target.
// b: the part of the shown picture that fills the drawn rect (offset, scale).
// c: the tile in target pixels (min, size); d: corner radius, _, drawn size;
// e: the drawn rect's min in target pixels. Target pixels, because egui
// clamps the viewport to the window when a tile reaches past its edge.
struct Params { a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>, e: vec4<f32> };
@group(0) @binding(4) var<uniform> params: Params;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the viewport.
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv_rect = (position.xy - params.e.xy) / params.d.zw;
    // Rounded corners of the tile.
    let px = position.xy - params.c.xy;
    let half = params.c.zw * 0.5;
    let r = params.d.x;
    let q = abs(px - half) - (half - vec2<f32>(r));
    let dist = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
    let alpha = clamp(0.5 - dist, 0.0, 1.0);
    // The point of the shown picture, then of the frame once it is turned.
    var shown = params.b.xy + uv_rect * params.b.zw;
    if (params.a.z > 0.5) {
        shown.x = 1.0 - shown.x;
    }
    var uv = shown;
    if (params.a.y > 2.5) {
        uv = vec2<f32>(1.0 - shown.y, shown.x);
    } else if (params.a.y > 1.5) {
        uv = vec2<f32>(1.0) - shown;
    } else if (params.a.y > 0.5) {
        uv = vec2<f32>(shown.y, 1.0 - shown.x);
    }
    let y = (textureSample(plane_y, bilinear, uv).r - 16.0 / 255.0) * (255.0 / 219.0);
    let u = (textureSample(plane_u, bilinear, uv).r - 128.0 / 255.0) * (255.0 / 224.0);
    let v = (textureSample(plane_v, bilinear, uv).r - 128.0 / 255.0) * (255.0 / 224.0);
    var rgb: vec3<f32>;
    if (params.a.x > 0.5) {
        rgb = vec3<f32>(y + 1.5748 * v, y - 0.1873 * u - 0.4681 * v, y + 1.8556 * u);
    } else {
        rgb = vec3<f32>(y + 1.402 * v, y - 0.3441 * u - 0.7141 * v, y + 1.772 * u);
    }
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if (params.a.w > 0.5) {
        rgb = srgb_to_linear(rgb);
    }
    return vec4<f32>(rgb * alpha, alpha);
}
"#;

/// The pipeline, kept in egui_wgpu's callback resources.
struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    srgb: bool,
    tiles: HashMap<egui::Id, Planes>,
}

impl Resources {
    fn drop_unused(&mut self, now: Instant) {
        self.tiles
            .retain(|_, p| now.saturating_duration_since(p.used) <= KEEP);
    }
}

/// Once a frame of the app: frees the GPU copies of tiles no window draws
/// any more, also after the last one went (no draw runs then).
pub fn sweep_gpu(render_state: &egui_wgpu::RenderState) {
    sweep_gpu_at(render_state, Instant::now());
}

fn sweep_gpu_at(render_state: &egui_wgpu::RenderState, now: Instant) {
    let mut renderer = render_state.renderer.write();
    if let Some(res) = renderer.callback_resources.get_mut::<Resources>()
        && !res.tiles.is_empty()
    {
        res.drop_unused(now);
    }
}

/// One tile's GPU copy of its picture.
struct Planes {
    width: u32,
    height: u32,
    serial: Option<u64>,
    textures: [wgpu::Texture; 3],
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// When a tile drew it last; tiles gone for [`KEEP`] are freed. A
    /// clock, not egui's frame number, which each window counts for itself.
    used: Instant,
}

/// How long the GPU copy of a tile that is no longer drawn stays.
const KEEP: Duration = Duration::from_secs(2);

/// The pipeline is in a renderer: pictures draw at their size.
static INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Why pictures stay small here, if they do: without the GPU pipeline
/// (another renderer) tiles have nothing to draw them with.
pub fn limit() -> Option<super::tile::VideoLimit> {
    (!INSTALLED.load(std::sync::atomic::Ordering::Acquire))
        .then_some(super::tile::VideoLimit::Renderer)
}

/// Builds the pipeline once for the renderer's target (no MSAA, no depth:
/// eframe's defaults, which the app keeps).
pub fn install(render_state: &egui_wgpu::RenderState) {
    INSTALLED.store(true, std::sync::atomic::Ordering::Release);
    let mut renderer = render_state.renderer.write();
    if renderer.callback_resources.contains::<Resources>() {
        return;
    }
    let device = &render_state.device;
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("video planes"),
        entries: &[
            texture(0),
            texture(1),
            texture(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("video"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("video"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let format = render_state.target_format;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("video"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("video"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    renderer.callback_resources.insert(Resources {
        pipeline,
        layout,
        sampler,
        srgb: format.is_srgb(),
        tiles: HashMap::new(),
    });
}

impl Planes {
    fn new(device: &wgpu::Device, res: &Resources, width: u32, height: u32) -> Self {
        let plane = |label, w, h| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        let textures = [
            plane("video y", width, height),
            plane("video u", cw, ch),
            plane("video v", cw, ch),
        ];
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video params"),
            size: PARAMS_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let views: Vec<_> = textures
            .iter()
            .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video planes"),
            layout: &res.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&res.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        Self {
            width,
            height,
            serial: None,
            textures,
            params,
            bind_group,
            used: Instant::now(),
        }
    }

    fn upload(&mut self, queue: &wgpu::Queue, frame: &Frame) {
        let (cw, ch) = (frame.width.div_ceil(2), frame.height.div_ceil(2));
        for (texture, data, stride, w, h) in [
            (
                &self.textures[0],
                &frame.y,
                frame.stride_y,
                frame.width,
                frame.height,
            ),
            (&self.textures[1], &frame.u, frame.stride_u, cw, ch),
            (&self.textures[2], &frame.v, frame.stride_v, cw, ch),
        ] {
            let stride = stride.max(w);
            // A short plane would make wgpu panic; such a frame is skipped.
            if (data.len() as u64) < u64::from(stride) * u64::from(h - 1) + u64::from(w) {
                return;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.serial = Some(frame.serial);
    }
}

/// The shader's parameters for one draw.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Params {
    bt709: bool,
    turns: u32,
    mirror: bool,
    /// The part of the shown picture in the drawn rect: offset and scale.
    crop: [f32; 4],
    /// The tile in target pixels: min and size.
    tile: [f32; 4],
    radius: f32,
    /// The drawn rect in target pixels: min and size.
    drawn: [f32; 4],
}

const PARAMS_SIZE: usize = 80;

impl Params {
    fn bytes(&self, srgb: bool) -> [u8; PARAMS_SIZE] {
        let values = [
            f32::from(u8::from(self.bt709)),
            self.turns as f32,
            f32::from(u8::from(self.mirror)),
            f32::from(u8::from(srgb)),
            self.crop[0],
            self.crop[1],
            self.crop[2],
            self.crop[3],
            self.tile[0],
            self.tile[1],
            self.tile[2],
            self.tile[3],
            self.radius,
            0.0,
            self.drawn[2],
            self.drawn[3],
            self.drawn[0],
            self.drawn[1],
            0.0,
            0.0,
        ];
        let mut out = [0u8; PARAMS_SIZE];
        for (i, v) in values.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

struct Draw {
    id: egui::Id,
    frame: Arc<Frame>,
    params: Params,
    at: Instant,
}

impl CallbackTrait for Draw {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get_mut::<Resources>() else {
            return Vec::new();
        };
        let (w, h) = (self.frame.width, self.frame.height);
        if w == 0 || h == 0 || w > 16384 || h > 16384 {
            return Vec::new();
        }
        let fits = res
            .tiles
            .get(&self.id)
            .is_some_and(|p| p.width == w && p.height == h);
        if !fits {
            let planes = Planes::new(device, res, w, h);
            res.tiles.insert(self.id, planes);
        }
        let srgb = res.srgb;
        if let Some(planes) = res.tiles.get_mut(&self.id) {
            if planes.serial != Some(self.frame.serial) {
                planes.upload(queue, &self.frame);
            }
            planes.used = planes.used.max(self.at);
            queue.write_buffer(&planes.params, 0, &self.params.bytes(srgb));
        }
        res.drop_unused(self.at);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(res) = resources.get::<Resources>() else {
            return;
        };
        let Some(planes) = res.tiles.get(&self.id) else {
            return;
        };
        if planes.serial.is_none() {
            return;
        }
        pass.set_pipeline(&res.pipeline);
        pass.set_bind_group(0, &planes.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The picture's size as shown, turned by its rotation.
pub fn shown_size(frame: &Frame) -> (f32, f32) {
    if (frame.rotation / 90) % 2 == 1 {
        (frame.height as f32, frame.width as f32)
    } else {
        (frame.width as f32, frame.height as f32)
    }
}

/// Where the picture goes in `tile` and which part of it shows there:
/// the drawn rect and the crop (offset, scale) of the shown picture.
pub fn placement(tile: Rect, shown: (f32, f32), fit: Fit) -> (Rect, [f32; 4]) {
    let (w, h) = shown;
    if w <= 0.0 || h <= 0.0 || tile.width() <= 0.0 || tile.height() <= 0.0 {
        return (tile, [0.0, 0.0, 1.0, 1.0]);
    }
    let picture = w / h;
    let room = tile.width() / tile.height();
    match fit {
        Fit::Cover => {
            if picture > room {
                let scale = room / picture;
                (tile, [(1.0 - scale) / 2.0, 0.0, scale, 1.0])
            } else {
                let scale = picture / room;
                (tile, [0.0, (1.0 - scale) / 2.0, 1.0, scale])
            }
        }
        Fit::Contain => {
            let size = if picture > room {
                vec2(tile.width(), tile.width() / picture)
            } else {
                vec2(tile.height() * picture, tile.height())
            };
            (
                Rect::from_center_size(tile.center(), size),
                [0.0, 0.0, 1.0, 1.0],
            )
        }
    }
}

/// Paints `frame` into `tile` (clipped to the ui's clip rect), with the
/// tile's corner `radius`. `id` names the tile's GPU copy.
pub fn paint(
    ui: &Ui,
    id: egui::Id,
    tile: Rect,
    frame: Arc<Frame>,
    fit: Fit,
    mirror: bool,
    radius: f32,
) {
    let ppp = ui.ctx().pixels_per_point();
    let (rect, crop) = placement(tile, shown_size(&frame), fit);
    let params = Params {
        bt709: false,
        turns: (frame.rotation / 90) & 3,
        mirror,
        crop,
        tile: [
            tile.left() * ppp,
            tile.top() * ppp,
            tile.width() * ppp,
            tile.height() * ppp,
        ],
        radius: radius * ppp,
        drawn: [
            rect.left() * ppp,
            rect.top() * ppp,
            rect.width() * ppp,
            rect.height() * ppp,
        ],
    };
    let draw = Draw {
        id,
        frame,
        params,
        at: Instant::now(),
    };
    ui.painter()
        .add(egui_wgpu::Callback::new_paint_callback(rect, draw));
}

/// The size each key was last asked for, when (`input.time`), and for a
/// cover tile whether the picture counted as wider than it.
#[derive(Clone, Default)]
struct Asked(HashMap<gelabber_call::VideoKey, ((u32, u32, u32), f64, Option<bool>)>);

fn asked_id() -> egui::Id {
    egui::Id::unique("gelabber-video-requests")
}

/// A key no tile asked for this long has no tile any more.
const GONE_AFTER: f64 = 0.5;

/// How far a picture's shape may stray from the tile's before a cover tile
/// switches the side it fills by: more than the core's rounding moves it.
const COVER_SLACK: f32 = 1.03;

/// What to ask the core for so the picture fills a `tile` (physical
/// pixels) drawn with `fit`: the core shrinks the picture to fit inside,
/// aspect kept, sides rounded down to even, and 0 leaves a side open.
/// Only the tile, the picture's turn and (for cover) which side is longer
/// decide, never the delivered size, which carries the core's rounding.
/// `wide_before` is the last cover decision; returns the size and this one.
pub fn ask(
    tile: (u32, u32),
    shown: Option<(f32, f32)>,
    turned: bool,
    fit: Fit,
    wide_before: Option<bool>,
) -> ((u32, u32), Option<bool>) {
    let (tw, th) = tile;
    let (size, wide) = match (fit, shown) {
        (Fit::Cover, Some((w, h))) if w > 0.0 && h > 0.0 => {
            let (picture, room) = (w / h, tw as f32 / th as f32);
            let wide = match wide_before {
                Some(true) => picture * COVER_SLACK > room,
                Some(false) => picture > room * COVER_SLACK,
                None => picture > room,
            };
            // The tile's height (wide) or width, the other side open.
            ((if wide { (0, th) } else { (tw, 0) }), Some(wide))
        }
        _ => ((tw, th), None),
    };
    // The core scales the picture before it is turned.
    let size = if turned { (size.1, size.0) } else { size };
    (size, wide)
}

/// Asks the call for the size a tile shows `key` at (see [`ask`]), once
/// per change: the core scales and drops frames before they reach Rust.
/// `frame` is the picture the tile shows, when it has one.
pub fn request_fit(
    ui: &Ui,
    call: &gelabber_call::Call,
    key: gelabber_call::VideoKey,
    tile: Rect,
    frame: Option<&Frame>,
    fit: Fit,
    fps: u32,
) {
    let ppp = ui.ctx().pixels_per_point();
    let tile_px = (
        (tile.width() * ppp).round().max(1.0) as u32,
        (tile.height() * ppp).round().max(1.0) as u32,
    );
    let now = ui.ctx().input(|i| i.time);
    let before = ui.ctx().data(|d| {
        d.get_temp::<Asked>(asked_id())
            .and_then(|a| a.0.get(&key).copied())
    });
    let (wh, wide) = ask(
        tile_px,
        frame.map(shown_size),
        frame.is_some_and(|f| (f.rotation / 90) % 2 == 1),
        fit,
        before.and_then(|(_, _, wide)| wide),
    );
    let size = (wh.0, wh.1, fps);
    ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<Asked>(asked_id())
            .0
            .insert(key, (size, now, wide))
    });
    if before.map(|(size, _, _)| size) != Some(size) {
        call.request_video(key, size.0, size.1, size.2);
    }
}

/// [`request_fit`] for the whole of `tile`, before a picture shows.
pub fn request(
    ui: &Ui,
    call: &gelabber_call::Call,
    key: gelabber_call::VideoKey,
    tile: Rect,
    fps: u32,
) {
    request_fit(ui, call, key, tile, None, Fit::Contain, fps);
}

/// Forgets the last size asked for `key`, here and in the call, so the
/// next tile asks again.
pub fn forget(ctx: &egui::Context, call: &gelabber_call::Call, key: gelabber_call::VideoKey) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Asked>(asked_id())
            .0
            .remove(&key)
    });
    call.release_video(key);
}

/// Once a frame: a tile does not say it went, so a key no tile asked for
/// a while is let go of, and its next tile asks for its size again.
pub fn sweep(ctx: &egui::Context, call: &gelabber_call::Call) {
    let now = ctx.input(|i| i.time);
    let gone: Vec<gelabber_call::VideoKey> = ctx.data_mut(|d| {
        let asked = &mut d.get_temp_mut_or_default::<Asked>(asked_id()).0;
        let gone = asked
            .iter()
            .filter(|(_, (_, at, _))| now - at > GONE_AFTER)
            .map(|(k, _)| *k)
            .collect();
        asked.retain(|_, (_, at, _)| now - *at <= GONE_AFTER);
        gone
    });
    for key in gone {
        call.release_video(key);
    }
}

/// The rect a [`placement`] gives, for callers that draw around it.
pub fn drawn_rect(tile: Rect, frame: &Frame, fit: Fit) -> Rect {
    placement(tile, shown_size(frame), fit).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    /// A frame of one colour, BT.601 limited range.
    pub fn solid(width: u32, height: u32, y: u8, u: u8, v: u8, serial: u64) -> Frame {
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        Frame {
            width,
            height,
            y: vec![y; (width * height) as usize],
            u: vec![u; (cw * ch) as usize],
            v: vec![v; (cw * ch) as usize],
            stride_y: width,
            stride_u: cw,
            stride_v: cw,
            rotation: 0,
            serial,
        }
    }

    /// A tile that went stops holding its size: drawn again at the same
    /// size, it asks the call again.
    #[test]
    fn a_tile_that_went_asks_again() {
        let call = gelabber_call::Call::new(
            gelabber_client::Client::new("http://127.0.0.1:9").expect("client"),
        );
        let key = gelabber_call::VideoKey(8);
        let ctx = egui::Context::default();
        let tile = Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 180.0));
        let frame = |time: f64, drawn: bool| {
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                sweep(ui.ctx(), &call);
                if drawn {
                    request(ui, &call, key, tile, 30);
                }
            });
            out.textures_delta.clear();
        };
        frame(0.0, true);
        assert_eq!(call.video_request(key), Some((320, 180, 30)));
        frame(0.3, false);
        assert!(
            call.video_request(key).is_some(),
            "a frame away is not gone"
        );
        frame(1.0, false);
        assert_eq!(call.video_request(key), None);
        frame(1.1, true);
        assert_eq!(call.video_request(key), Some((320, 180, 30)));
    }

    /// The core's `Fit` (gelabber_media.cc): shrink into the limits (0:
    /// none), aspect kept, to even sides; a picture that fits stays.
    fn core_fit(max: (u32, u32), source: (u32, u32)) -> (u32, u32) {
        let (mw, mh) = (i64::from(max.0), i64::from(max.1));
        let (w, h) = (i64::from(source.0), i64::from(source.1));
        let wide = mw > 0 && w > mw;
        let tall = mh > 0 && h > mh;
        if !wide && !tall {
            return source;
        }
        let (w, h) = if wide && (!tall || mw * h <= mh * w) {
            (mw, h * mw / w)
        } else {
            (w * mh / h, mh)
        };
        ((w & !1).max(2) as u32, (h & !1).max(2) as u32)
    }

    /// Asked, delivered, asked again: the ask settles at once for every
    /// tile size (the stage's 944×612 used to flip every frame), a cover
    /// picture covers its tile and a contained one fits.
    #[test]
    fn the_ask_settles_and_fills_the_tile() {
        for source in [(1920, 1080), (1280, 720), (640, 480), (1080, 1920)] {
            for fit in [Fit::Contain, Fit::Cover] {
                for tw in (120..1400).step_by(37) {
                    for th in (90..900).step_by(29) {
                        let mut frame = None::<(u32, u32)>;
                        let mut wide = None;
                        let mut asks = Vec::new();
                        for _ in 0..4 {
                            let shown = frame.map(|(w, h)| (w as f32, h as f32));
                            let (size, w) = ask((tw, th), shown, false, fit, wide);
                            wide = w;
                            asks.push(size);
                            frame = Some(core_fit(size, source));
                        }
                        assert_eq!(asks[1], asks[3], "{source:?} {fit:?} {tw}×{th}: {asks:?}");
                        assert_eq!(asks[2], asks[3], "{source:?} {fit:?} {tw}×{th}: {asks:?}");
                        let (w, h) = frame.unwrap();
                        // No upscaling beyond the source itself.
                        let shrunk = w < source.0;
                        match fit {
                            Fit::Contain => assert!(w <= tw && h <= th),
                            Fit::Cover if shrunk => assert!(
                                // 3 % of slack near the tile's own shape.
                                w as f32 * 1.04 >= tw as f32 && h as f32 * 1.04 >= th as f32,
                                "{source:?} {tw}×{th}: {w}×{h}"
                            ),
                            Fit::Cover => {}
                        }
                    }
                }
            }
        }
        // A turned picture is scaled before the turn: the sides swap.
        assert_eq!(
            ask((400, 300), Some((300.0, 400.0)), true, Fit::Contain, None).0,
            (300, 400)
        );
    }

    #[test]
    fn cover_cuts_and_contain_letterboxes() {
        let tile = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0));
        let (rect, crop) = placement(tile, (400.0, 200.0), Fit::Cover);
        assert_eq!(rect, tile);
        assert_eq!(crop, [0.25, 0.0, 0.5, 1.0]);
        let (rect, crop) = placement(tile, (400.0, 200.0), Fit::Contain);
        assert_eq!(
            rect,
            Rect::from_min_size(pos2(0.0, 50.0), vec2(200.0, 100.0))
        );
        assert_eq!(crop, [0.0, 0.0, 1.0, 1.0]);
        let mut frame = solid(4, 2, 0, 0, 0, 0);
        frame.rotation = 90;
        assert_eq!(shown_size(&frame), (2.0, 4.0));
    }

    /// Draws synthetic frames through the real wgpu path and reads the
    /// pixels back: red as BT.601 limited range, a camera cut to its tile,
    /// a Live letterboxed on the tile's dark background.
    #[test]
    fn draws_frames_on_the_gpu() {
        let red = Arc::new(solid(64, 36, 81, 90, 240, 1));
        let tall = Arc::new(solid(36, 64, 145, 54, 34, 1));
        let render_state = egui_kittest::wgpu::create_render_state(
            egui_kittest::wgpu::default_wgpu_setup(),
            egui_wgpu::RendererOptions::PREDICTABLE,
        );
        install(&render_state);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(300.0, 120.0))
            .with_pixels_per_point(1.0)
            .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(
                render_state,
            ))
            .build_ui(move |ui| {
                let bg = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 120.0));
                ui.painter()
                    .rect_filled(bg, 0.0, egui::Color32::from_rgb(11, 13, 14));
                let cover = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 100.0));
                paint(
                    ui,
                    egui::Id::unique("video-test-a"),
                    cover,
                    red.clone(),
                    Fit::Cover,
                    false,
                    10.0,
                );
                let contain = Rect::from_min_size(pos2(150.0, 10.0), vec2(140.0, 100.0));
                paint(
                    ui,
                    egui::Id::unique("video-test-b"),
                    contain,
                    tall.clone(),
                    Fit::Contain,
                    false,
                    0.0,
                );
            });
        harness.run();
        let image = harness.render().expect("render");
        let px = |x: u32, y: u32| image.get_pixel(x, y).0;
        // Red from Y 81, U 90, V 240.
        let [r, g, b, _] = px(60, 60);
        assert!(r > 230 && g < 25 && b < 25, "red: {r} {g} {b}");
        // The rounded corner leaves the background.
        assert_eq!(&px(10, 10)[..3], &[11, 13, 14]);
        // Green from Y 145, U 54, V 34, only in the middle of its tile.
        let [r, g, b, _] = px(220, 60);
        assert!(r < 25 && g > 230 && b < 25, "green: {r} {g} {b}");
        assert_eq!(&px(160, 60)[..3], &[11, 13, 14]);
    }

    /// The GPU copies of tiles go a while after the last draw, through the
    /// app's sweep: no tile's draw is left to free them.
    #[test]
    fn copies_of_tiles_no_one_draws_are_freed() {
        let red = Arc::new(solid(64, 36, 81, 90, 240, 1));
        let render_state = egui_kittest::wgpu::create_render_state(
            egui_kittest::wgpu::default_wgpu_setup(),
            egui_wgpu::RendererOptions::PREDICTABLE,
        );
        install(&render_state);
        let shown = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let on = shown.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(120.0, 120.0))
            .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(
                render_state.clone(),
            ))
            .build_ui(move |ui| {
                if on.load(std::sync::atomic::Ordering::Relaxed) {
                    let tile = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 100.0));
                    paint(
                        ui,
                        egui::Id::unique("video-test-sweep"),
                        tile,
                        red.clone(),
                        Fit::Cover,
                        false,
                        0.0,
                    );
                }
            });
        harness.run();
        harness.render().expect("render");
        let tiles = |rs: &egui_wgpu::RenderState| {
            rs.renderer
                .read()
                .callback_resources
                .get::<Resources>()
                .map_or(0, |r| r.tiles.len())
        };
        assert_eq!(tiles(&render_state), 1);
        shown.store(false, std::sync::atomic::Ordering::Relaxed);
        harness.run();
        harness.render().expect("render");
        sweep_gpu(&render_state);
        assert_eq!(tiles(&render_state), 1, "kept for a moment");
        sweep_gpu_at(&render_state, Instant::now() + KEEP * 2);
        assert_eq!(tiles(&render_state), 0);
    }

    /// A tile reaching past the window's edge keeps its scale: egui clamps
    /// the viewport, the shader maps from target pixels.
    #[test]
    fn a_tile_past_the_edge_is_cut_not_squeezed() {
        // Top half red, bottom half green.
        let mut halves = solid(64, 64, 81, 90, 240, 1);
        halves.y[64 * 32..].fill(145);
        halves.u[32 * 16..].fill(54);
        halves.v[32 * 16..].fill(34);
        let halves = Arc::new(halves);
        let render_state = egui_kittest::wgpu::create_render_state(
            egui_kittest::wgpu::default_wgpu_setup(),
            egui_wgpu::RendererOptions::PREDICTABLE,
        );
        install(&render_state);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(120.0, 120.0))
            .with_pixels_per_point(1.0)
            .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(
                render_state,
            ))
            .build_ui(move |ui| {
                // 100 high from y 70: only its top half is in the window.
                let tile = Rect::from_min_size(pos2(10.0, 70.0), vec2(100.0, 100.0));
                paint(
                    ui,
                    egui::Id::unique("video-test-edge"),
                    tile,
                    halves.clone(),
                    Fit::Cover,
                    false,
                    0.0,
                );
            });
        harness.run();
        let image = harness.render().expect("render");
        let [r, g, b, _] = image.get_pixel(60, 115).0;
        assert!(
            r > 230 && g < 25 && b < 25,
            "still the top half: {r} {g} {b}"
        );
    }
}
