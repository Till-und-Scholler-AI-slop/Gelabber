//! Native video viewer: remote video in its own window, drawn on the GPU.
//! WebKitGTK cannot show the native core's video, so a stream the page
//! watches opens here. The windows run on their own thread and event loop
//! (winit), apart from the webview's GTK loop; decoded I420 frames go to the
//! GPU as three planes and are converted there.

use gelabber_media_core::VideoFrame;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, mpsc},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy, OwnedDisplayHandle},
    window::{Window, WindowAttributes, WindowId},
};

/// The latest decoded frame of a stream, tightly packed.
#[derive(Default)]
struct Frame {
    width: u32,
    height: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

#[derive(Default)]
struct Slot {
    frame: Frame,
    /// A frame arrived that the window has not drawn yet.
    fresh: bool,
}

fn copy_plane(dst: &mut Vec<u8>, src: &[u8], stride: usize, width: usize, rows: usize) {
    dst.clear();
    for row in 0..rows {
        let start = row * stride;
        dst.extend_from_slice(&src[start..start + width]);
    }
}

/// What a viewer window reports to the page.
#[derive(Debug, PartialEq)]
pub enum ViewerEvent {
    /// Height of the shown image in physical pixels (for layer choice).
    Height(u32),
    /// The window went away (or never opened); sent once, last.
    Closed,
}

pub type ViewerEvents = Box<dyn Fn(ViewerEvent) + Send>;

/// A window's listener; reports `Closed` when dropped.
struct Events(ViewerEvents);

impl Drop for Events {
    fn drop(&mut self) {
        (self.0)(ViewerEvent::Closed);
    }
}

enum Command {
    Open {
        id: u64,
        title: String,
        slot: Arc<Mutex<Slot>>,
        events: Events,
    },
    Close {
        id: u64,
    },
    CloseAll,
    Frame {
        id: u64,
    },
}

/// Handle to the viewer thread; started with the first window.
pub struct Viewer {
    proxy: Mutex<EventLoopProxy<Command>>,
}

static VIEWER: OnceLock<Result<Viewer, String>> = OnceLock::new();

impl Viewer {
    pub fn get() -> Result<&'static Viewer, String> {
        VIEWER
            .get_or_init(|| {
                let (ready, proxy) = mpsc::channel();
                std::thread::Builder::new()
                    .name("gelabber-viewer".into())
                    .spawn(move || run(ready))
                    .map_err(|e| e.to_string())?;
                let proxy = proxy.recv().map_err(|e| e.to_string())??;
                Ok(Viewer {
                    proxy: Mutex::new(proxy),
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Opens a window for stream `id` and returns the sink that feeds it.
    /// `events` gets the shown height and, once, `Closed` when the window
    /// goes away (the person closed it, or `close`).
    pub fn open(
        &self,
        id: u64,
        title: String,
        events: ViewerEvents,
    ) -> Result<gelabber_media_core::VideoSink, String> {
        let slot = Arc::new(Mutex::new(Slot::default()));
        let proxy = self.proxy.lock().unwrap().clone();
        proxy
            .send_event(Command::Open {
                id,
                title,
                slot: slot.clone(),
                events: Events(events),
            })
            .map_err(|_| "viewer stopped".to_string())?;
        Ok(Box::new(move |frame: &VideoFrame<'_>| {
            let (width, height) = (frame.width as usize, frame.height as usize);
            let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
            let wake = {
                let mut slot = slot.lock().unwrap();
                let out = &mut slot.frame;
                out.width = frame.width;
                out.height = frame.height;
                copy_plane(&mut out.y, frame.y, frame.stride_y, width, height);
                copy_plane(
                    &mut out.u,
                    frame.u,
                    frame.stride_u,
                    chroma_width,
                    chroma_height,
                );
                copy_plane(
                    &mut out.v,
                    frame.v,
                    frame.stride_v,
                    chroma_width,
                    chroma_height,
                );
                !std::mem::replace(&mut slot.fresh, true)
            };
            // One wake-up per drawn frame; a busy window skips frames.
            if wake {
                let _ = proxy.send_event(Command::Frame { id });
            }
        }))
    }

    pub fn close(&self, id: u64) {
        self.send(Command::Close { id });
    }

    pub fn close_all(&self) {
        self.send(Command::CloseAll);
    }

    /// The viewer if a window was ever opened.
    pub fn running() -> Option<&'static Viewer> {
        VIEWER.get().and_then(|viewer| viewer.as_ref().ok())
    }

    fn send(&self, command: Command) {
        let _ = self.proxy.lock().unwrap().send_event(command);
    }
}

fn run(ready: mpsc::Sender<Result<EventLoopProxy<Command>, String>>) {
    let mut builder = EventLoop::<Command>::with_user_event();
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use winit::platform::{wayland::EventLoopBuilderExtWayland, x11::EventLoopBuilderExtX11};
        EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
        EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
    }
    #[cfg(windows)]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    let event_loop = match builder.build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            let _ = ready.send(Err(format!("viewer event loop: {error}")));
            return;
        }
    };
    let _ = ready.send(Ok(event_loop.create_proxy()));
    let mut app = App {
        display: event_loop.owned_display_handle(),
        gpu: None,
        windows: HashMap::new(),
        ids: HashMap::new(),
    };
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("[gelabber] viewer stopped: {error}");
    }
}

const SHADER: &str = r#"
@group(0) @binding(0) var plane_y: texture_2d<f32>;
@group(0) @binding(1) var plane_u: texture_2d<f32>;
@group(0) @binding(2) var plane_v: texture_2d<f32>;
@group(0) @binding(3) var bilinear: sampler;
// x: 1 for BT.709, 0 for BT.601 (both limited range).
@group(0) @binding(4) var<uniform> colors: vec4<f32>;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) index: u32) -> Out {
    // One triangle covering the viewport.
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Out;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs(input: Out) -> @location(0) vec4<f32> {
    let y = (textureSample(plane_y, bilinear, input.uv).r - 16.0 / 255.0) * (255.0 / 219.0);
    let u = (textureSample(plane_u, bilinear, input.uv).r - 128.0 / 255.0) * (255.0 / 224.0);
    let v = (textureSample(plane_v, bilinear, input.uv).r - 128.0 / 255.0) * (255.0 / 224.0);
    var rgb: vec3<f32>;
    if (colors.x > 0.5) {
        rgb = vec3<f32>(y + 1.5748 * v, y - 0.1873 * u - 0.4681 * v, y + 1.8556 * u);
    } else {
        rgb = vec3<f32>(y + 1.402 * v, y - 0.3441 * u - 0.7141 * v, y + 1.772 * u);
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
"#;

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
}

/// Which of `adapters` draws. They can all present to the window and come in
/// the system's order: Vulkan before GL, and Vulkan lists the GPU that drives
/// the screen first (Mesa's device-select layer asks the compositor, the
/// loader prefers a discrete GPU). The first one wins, not the low-power one:
/// a second GPU's images reach a compositor on the NVIDIA driver as a black
/// window (AMD iGPU next to an RTX card: v0.5.2). A software renderer only
/// wins when nothing else is there; a GPU without a Vulkan driver is listed
/// after lavapipe, through GL.
/// `named` (WGPU_ADAPTER_NAME, part of the name) and `power`
/// (WGPU_POWER_PREF: low, high) override the choice.
fn choose_adapter(
    adapters: &[wgpu::AdapterInfo],
    named: Option<&str>,
    power: wgpu::PowerPreference,
) -> Result<usize, String> {
    use wgpu::{DeviceType, PowerPreference};
    if let Some(named) = named {
        let wanted = named.to_lowercase();
        return adapters
            .iter()
            .position(|info| info.name.to_lowercase().contains(&wanted))
            .ok_or_else(|| {
                let names: Vec<_> = adapters.iter().map(|info| &info.name).collect();
                format!("no GPU adapter named {named:?} (WGPU_ADAPTER_NAME) among {names:?}")
            });
    }
    // With a power preference wgpu's own order, otherwise the system's.
    let rank = |info: &wgpu::AdapterInfo| match (power, info.device_type) {
        (_, DeviceType::Cpu) => 5,
        (PowerPreference::None, _) => 0,
        (PowerPreference::LowPower, DeviceType::IntegratedGpu)
        | (PowerPreference::HighPerformance, DeviceType::DiscreteGpu) => 1,
        (_, DeviceType::IntegratedGpu | DeviceType::DiscreteGpu) => 2,
        (_, DeviceType::Other) => 3,
        (_, DeviceType::VirtualGpu) => 4,
    };
    // The first of the best: min_by_key keeps the earliest of equals.
    (0..adapters.len())
        .min_by_key(|&index| rank(&adapters[index]))
        .ok_or_else(|| "no GPU adapter".to_string())
}

impl Gpu {
    fn new(instance: wgpu::Instance, surface: &wgpu::Surface<'_>) -> Result<Self, String> {
        let mut adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let listed = adapters.len();
        adapters.retain(|adapter| adapter.is_surface_supported(surface));
        if adapters.is_empty() {
            return Err(format!(
                "no GPU adapter can draw to the window ({listed} listed)"
            ));
        }
        let infos: Vec<_> = adapters.iter().map(wgpu::Adapter::get_info).collect();
        let named = std::env::var("WGPU_ADAPTER_NAME")
            .ok()
            .filter(|name| !name.is_empty());
        let power = wgpu::PowerPreference::from_env().unwrap_or_default();
        let chosen = choose_adapter(&infos, named.as_deref(), power)?;
        let adapter = adapters.swap_remove(chosen);
        // Once per run: the GPU is kept for every later window.
        let info = &infos[chosen];
        eprintln!(
            "[gelabber] viewer GPU: {} ({:?}, {:?})",
            info.name, info.backend, info.device_type
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("viewer"),
            required_limits:
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU device: {e}"))?;
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
            label: Some("viewer planes"),
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
            label: Some("viewer"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewer"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewer"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            layout,
            pipelines: HashMap::new(),
            shader,
            pipeline_layout,
            sampler,
        })
    }

    fn pipeline(&mut self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        self.pipelines.entry(format).or_insert_with(|| {
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("viewer"),
                    layout: Some(&self.pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &self.shader,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(format.into())],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })
    }
}

/// GPU copies of one frame's planes.
struct Planes {
    width: u32,
    height: u32,
    textures: [wgpu::Texture; 3],
    bind_group: wgpu::BindGroup,
}

impl Planes {
    fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let plane = |label, w, h| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
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
            plane("y", width, height),
            plane("u", cw, ch),
            plane("v", cw, ch),
        ];
        // Video from 720 lines up is HD and BT.709 by convention.
        let bt709: f32 = if height >= 720 { 1.0 } else { 0.0 };
        let colors = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer colors"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut bytes = [0u8; 16];
        bytes[..4].copy_from_slice(&bt709.to_le_bytes());
        gpu.queue.write_buffer(&colors, 0, &bytes);
        let views: Vec<_> = textures
            .iter()
            .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect();
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewer planes"),
            layout: &gpu.layout,
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
                    resource: wgpu::BindingResource::Sampler(&gpu.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: colors.as_entire_binding(),
                },
            ],
        });
        Self {
            width,
            height,
            textures,
            bind_group,
        }
    }

    fn upload(&self, gpu: &Gpu, frame: &Frame) {
        let (cw, ch) = (frame.width.div_ceil(2), frame.height.div_ceil(2));
        for (texture, data, w, h) in [
            (&self.textures[0], &frame.y, frame.width, frame.height),
            (&self.textures[1], &frame.u, cw, ch),
            (&self.textures[2], &frame.v, cw, ch),
        ] {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

struct ViewerWindow {
    id: u64,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: Option<wgpu::SurfaceConfiguration>,
    slot: Arc<Mutex<Slot>>,
    planes: Option<Planes>,
    reported_height: u32,
    // Declared last: the window and surface are gone when it reports Closed.
    events: Events,
}

struct App {
    display: OwnedDisplayHandle,
    gpu: Option<Gpu>,
    windows: HashMap<WindowId, ViewerWindow>,
    ids: HashMap<u64, WindowId>,
}

impl App {
    fn instance(&self) -> wgpu::Instance {
        wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle(Box::new(self.display.clone()))
                .with_env(),
        )
    }

    fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: u64,
        title: String,
        slot: Arc<Mutex<Slot>>,
        events: Events,
    ) -> Result<(), String> {
        self.close(id);
        let attributes = WindowAttributes::default()
            .with_title(title)
            .with_inner_size(LogicalSize::new(1280.0, 720.0));
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let attributes = {
            // Window rules in tiling compositors (Hyprland) match the class.
            use winit::platform::wayland::WindowAttributesExtWayland;
            attributes.with_name("gelabber-viewer", "gelabber-viewer")
        };
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| format!("viewer window: {e}"))?,
        );
        let instance = match &self.gpu {
            Some(gpu) => gpu.instance.clone(),
            None => self.instance(),
        };
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("viewer surface: {e}"))?;
        if self.gpu.is_none() {
            self.gpu = Some(Gpu::new(instance, &surface)?);
        }
        let window_id = window.id();
        self.ids.insert(id, window_id);
        self.windows.insert(
            window_id,
            ViewerWindow {
                id,
                window,
                surface,
                config: None,
                slot,
                planes: None,
                reported_height: 0,
                events,
            },
        );
        self.configure(window_id);
        Ok(())
    }

    fn close(&mut self, id: u64) {
        if let Some(window_id) = self.ids.remove(&id) {
            self.windows.remove(&window_id);
        }
    }

    fn configure(&mut self, window_id: WindowId) {
        let (Some(gpu), Some(view)) = (self.gpu.as_ref(), self.windows.get_mut(&window_id)) else {
            return;
        };
        let size = view.window.inner_size();
        if size.width == 0 || size.height == 0 {
            view.config = None;
            return;
        }
        let capabilities = view.surface.get_capabilities(&gpu.adapter);
        let Some(&format) = capabilities
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .or(capabilities.formats.first())
        else {
            return;
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        view.surface.configure(&gpu.device, &config);
        view.config = Some(config);
        view.window.request_redraw();
    }

    fn draw(&mut self, window_id: WindowId) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let Some(view) = self.windows.get_mut(&window_id) else {
            return;
        };
        let Some(config) = view.config.clone() else {
            return;
        };
        {
            let mut slot = view.slot.lock().unwrap();
            let frame = &slot.frame;
            if slot.fresh && frame.width > 0 && frame.height > 0 {
                let matches = view
                    .planes
                    .as_ref()
                    .is_some_and(|p| p.width == frame.width && p.height == frame.height);
                if !matches {
                    view.planes = Some(Planes::new(gpu, frame.width, frame.height));
                }
                if let Some(planes) = &view.planes {
                    planes.upload(gpu, frame);
                }
            }
            slot.fresh = false;
        }
        let target = match view.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                view.surface.configure(&gpu.device, &config);
                view.window.request_redraw();
                return;
            }
            _ => return,
        };
        let format = config.format;
        let (surface_width, surface_height) = (config.width as f32, config.height as f32);
        let pipeline = gpu.pipeline(format).clone();
        // Letterbox: the whole frame, centered, aspect kept.
        let shown = view.planes.as_ref().map(|planes| {
            let scale =
                (surface_width / planes.width as f32).min(surface_height / planes.height as f32);
            (planes.width as f32 * scale, planes.height as f32 * scale)
        });
        if let Some((_, h)) = shown {
            let height = h.round() as u32;
            if height != view.reported_height {
                view.reported_height = height;
                (view.events.0)(ViewerEvent::Height(height));
            }
        }
        let output = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("viewer"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewer"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if let (Some(planes), Some((w, h))) = (&view.planes, shown) {
                pass.set_viewport(
                    (surface_width - w) / 2.0,
                    (surface_height - h) / 2.0,
                    w,
                    h,
                    0.0,
                    1.0,
                );
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &planes.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        gpu.queue.submit([encoder.finish()]);
        view.window.pre_present_notify();
        gpu.queue.present(target);
    }
}

impl ApplicationHandler<Command> for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, command: Command) {
        match command {
            Command::Open {
                id,
                title,
                slot,
                events,
            } => {
                // On failure `events` reports Closed as it drops.
                if let Err(error) = self.open(event_loop, id, title, slot, events) {
                    eprintln!("[gelabber] {error}");
                }
            }
            Command::Close { id } => self.close(id),
            Command::CloseAll => {
                self.ids.clear();
                self.windows.clear();
            }
            Command::Frame { id } => {
                if let Some(view) = self.ids.get(&id).and_then(|w| self.windows.get(w)) {
                    view.window.request_redraw();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                if let Some(view) = self.windows.remove(&window_id) {
                    self.ids.remove(&view.id);
                }
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.configure(window_id)
            }
            WindowEvent::RedrawRequested => self.draw(window_id),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::Command as Process,
        time::{Duration, Instant},
    };

    fn adapter(name: &str, device_type: wgpu::DeviceType) -> wgpu::AdapterInfo {
        let mut info = wgpu::AdapterInfo::new(device_type, wgpu::Backend::Vulkan);
        info.name = name.into();
        info
    }

    #[test]
    fn adapter_choice_follows_the_system_order() {
        use wgpu::{DeviceType::*, PowerPreference as Power};
        // v0.5.2's black window: an RTX card drives the screens, next to an
        // AMD iGPU.
        let two = [
            adapter("NVIDIA GeForce RTX 5080", DiscreteGpu),
            adapter("AMD Radeon Graphics (RADV RAPHAEL)", IntegratedGpu),
            adapter("llvmpipe (LLVM 21.1.2, 256 bits)", Cpu),
        ];
        assert_eq!(choose_adapter(&two, None, Power::None), Ok(0));
        assert_eq!(choose_adapter(&two, None, Power::HighPerformance), Ok(0));
        assert_eq!(choose_adapter(&two, None, Power::LowPower), Ok(1));
        assert_eq!(choose_adapter(&two, Some("radv"), Power::None), Ok(1));
        assert_eq!(choose_adapter(&two, Some("LLVMpipe"), Power::None), Ok(2));
        // A laptop whose iGPU drives the panel lists it first.
        let laptop = [
            adapter("Intel(R) Graphics (ADL GT2)", IntegratedGpu),
            adapter("NVIDIA GeForce RTX 4060 Laptop GPU", DiscreteGpu),
        ];
        assert_eq!(choose_adapter(&laptop, None, Power::None), Ok(0));
        assert_eq!(choose_adapter(&laptop, None, Power::HighPerformance), Ok(1));
        // No Vulkan driver for the GPU: lavapipe is listed ahead of its GL
        // driver, which does not tell what kind of device it is.
        let no_vulkan = [
            adapter("llvmpipe (LLVM 21.1.2, 256 bits)", Cpu),
            adapter("Mesa Intel(R) HD Graphics 4000 (IVB GT2)", Other),
        ];
        assert_eq!(choose_adapter(&no_vulkan, None, Power::None), Ok(1));
        assert_eq!(choose_adapter(&no_vulkan, None, Power::LowPower), Ok(1));
        // Software only (CI): better than no picture.
        assert_eq!(choose_adapter(&no_vulkan[..1], None, Power::None), Ok(0));
    }

    /// wgpu's own helper panics here, which would end the viewer thread.
    #[test]
    fn adapter_choice_reports_a_name_nothing_matches() {
        use wgpu::{DeviceType::DiscreteGpu, PowerPreference as Power};
        let adapters = [adapter("NVIDIA GeForce RTX 5080", DiscreteGpu)];
        assert_eq!(
            choose_adapter(&adapters, Some("nvdia"), Power::None).unwrap_err(),
            r#"no GPU adapter named "nvdia" (WGPU_ADAPTER_NAME) among ["NVIDIA GeForce RTX 5080"]"#
        );
        assert!(choose_adapter(&[], None, Power::None).is_err());
    }

    /// winit's rule: a Wayland session wins over `DISPLAY`.
    fn on_wayland() -> bool {
        ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
    }

    /// X11: one pixel of the screen.
    fn pixel(x: u32, y: u32) -> String {
        let out = Process::new("import")
            .args(["-window", "root", "-crop", &format!("1x1+{x}+{y}"), "txt:-"])
            .output()
            .expect("ImageMagick import");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Wayland: how much of the screen is the test frame's red and its blue,
    /// as `(pixels, mean column)` each. `GELABBER_TEST_OUTPUT` names the
    /// output to look at; without it grim scales all of them into one image.
    fn red_and_blue() -> [(f64, f64); 2] {
        let mut grim = Process::new("grim");
        grim.args(["-t", "ppm"]);
        if let Some(output) = std::env::var_os("GELABBER_TEST_OUTPUT") {
            grim.arg("-o").arg(output);
        }
        let out = grim.arg("-").output().expect("grim");
        assert!(
            out.status.success(),
            "grim: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // "P6\n<width> <height>\n255\n", then RGB rows.
        let mut parts = out.stdout.splitn(5, u8::is_ascii_whitespace);
        let header: Vec<_> = parts
            .by_ref()
            .take(4)
            .map(String::from_utf8_lossy)
            .collect();
        let rgb = parts.next().expect("PPM from grim");
        assert_eq!((&*header[0], &*header[3]), ("P6", "255"));
        let width: usize = header[1].parse().unwrap();
        // Pixels and the sum of their columns, red then blue.
        let mut areas = [(0u64, 0u64); 2];
        for (index, pixel) in rgb.as_chunks::<3>().0.iter().enumerate() {
            let area = match *pixel {
                [r, g, b] if r > 180 && g < 60 && b < 60 => &mut areas[0],
                [r, g, b] if r < 60 && g < 60 && b > 180 => &mut areas[1],
                _ => continue,
            };
            area.0 += 1;
            area.1 += (index % width) as u64;
        }
        areas.map(|(pixels, columns)| (pixels as f64, columns as f64 / pixels.max(1) as f64))
    }

    /// Draws a frame in a real window and checks the colors on a screenshot.
    /// Needs a GPU or a software Vulkan/GL driver.
    /// X11, without a window manager (ImageMagick takes the screenshot):
    /// `xvfb-run -a cargo test -p gelabber-desktop -- --ignored viewer`.
    /// Wayland (grim takes the screenshot): the window opens in the running
    /// session, also under xvfb-run as long as `WAYLAND_DISPLAY` is set:
    /// `GELABBER_TEST_OUTPUT=<output> cargo test -p gelabber-desktop -- --ignored viewer`.
    #[test]
    #[ignore = "needs a display"]
    fn viewer_draws_frames() {
        let viewer = Viewer::get().unwrap();
        let (events_tx, events_rx) = mpsc::channel();
        let mut sink = viewer
            .open(
                7,
                "viewer test".into(),
                Box::new(move |event| events_tx.send(event).unwrap()),
            )
            .unwrap();
        // 640x360: left half red, right half blue (BT.601 limited range).
        let (w, h) = (640usize, 360usize);
        let mut y = vec![0u8; w * h];
        let mut u = vec![0u8; w / 2 * h / 2];
        let mut v = vec![0u8; w / 2 * h / 2];
        for row in 0..h {
            for col in 0..w {
                y[row * w + col] = if col < w / 2 { 81 } else { 41 };
            }
        }
        for row in 0..h / 2 {
            for col in 0..w / 2 {
                let left = col < w / 4;
                u[row * w / 2 + col] = if left { 90 } else { 240 };
                v[row * w / 2 + col] = if left { 240 } else { 110 };
            }
        }
        for _ in 0..30 {
            sink(&VideoFrame {
                width: w as u32,
                height: h as u32,
                y: &y,
                u: &u,
                v: &v,
                stride_y: w,
                stride_u: w / 2,
                stride_v: w / 2,
                rotation: 0,
                timestamp_us: 0,
                source_width: w as u32,
                source_height: h as u32,
            });
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut events = Vec::new();
        if on_wayland() {
            // The compositor places and sizes the window (and animates it
            // in): look for the halves by color until both have the size
            // the viewer reports, red left of blue.
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                events.extend(events_rx.try_iter());
                let shown = match events.last() {
                    Some(ViewerEvent::Height(height)) => f64::from(*height),
                    _ => 0.0,
                };
                let half = shown * (shown * w as f64 / h as f64) / 2.0;
                let [red, blue] = red_and_blue();
                let fits = |pixels: f64| pixels > half * 0.9 && pixels < half * 1.1;
                if half > 0.0 && fits(red.0) && fits(blue.0) && red.1 < blue.1 {
                    eprintln!("{half} pixels each: red {red:?}, blue {blue:?}");
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "red left, blue right, {half} pixels each: red {red:?}, blue {blue:?}"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        } else {
            let (left, right) = (pixel(320, 360), pixel(960, 360));
            eprintln!("left {left}right {right}");
            // "0,0: (...)  #RRRRGGGGBBBB  ..." with 16-bit channels.
            let rgb = |text: &str| -> (u32, u32, u32) {
                let hex = &text[text.rfind('#').expect(text) + 1..][..12];
                let channel =
                    |i: usize| u32::from_str_radix(&hex[i * 4..i * 4 + 4], 16).unwrap() >> 8;
                (channel(0), channel(1), channel(2))
            };
            let (r, g, b) = rgb(&left);
            assert!(r > 180 && g < 60 && b < 60, "left half red: {left}");
            let (r, g, b) = rgb(&right);
            assert!(r < 60 && g < 60 && b > 180, "right half blue: {right}");
        }
        viewer.close(7);
        while let Ok(event) = events_rx.recv_timeout(Duration::from_secs(5)) {
            let closed = event == ViewerEvent::Closed;
            events.push(event);
            if closed {
                break;
            }
        }
        if on_wayland() {
            // One height per size the compositor gave the window.
            let (closed, heights) = events.split_last().unwrap();
            assert_eq!(closed, &ViewerEvent::Closed);
            assert!(
                !heights.is_empty()
                    && heights
                        .iter()
                        .all(|event| matches!(event, ViewerEvent::Height(1..))),
                "heights, then closed: {events:?}"
            );
        } else {
            // 1280x720 window, 16:9 frame: the image fills it.
            assert_eq!(events, [ViewerEvent::Height(720), ViewerEvent::Closed]);
        }
    }
}
