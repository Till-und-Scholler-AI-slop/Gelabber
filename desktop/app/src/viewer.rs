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

impl Gpu {
    fn new(instance: wgpu::Instance, surface: &wgpu::Surface<'_>) -> Result<Self, String> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
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
        let mut attributes = WindowAttributes::default()
            .with_title(title)
            .with_inner_size(LogicalSize::new(1280.0, 720.0));
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            // Window rules in tiling compositors (Hyprland) match the class.
            use winit::platform::wayland::WindowAttributesExtWayland;
            attributes = attributes.with_name("gelabber-viewer", "gelabber-viewer");
        }
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
    use std::{process::Command as Process, time::Duration};

    fn pixel(x: u32, y: u32) -> String {
        let out = Process::new("import")
            .args(["-window", "root", "-crop", &format!("1x1+{x}+{y}"), "txt:-"])
            .output()
            .expect("ImageMagick import");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Draws a frame in a real window: run under Xvfb with a GPU or a
    /// software Vulkan/GL driver, and ImageMagick for the screenshot:
    /// `xvfb-run -a cargo test -p gelabber-desktop -- --ignored viewer`.
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
        let (left, right) = (pixel(320, 360), pixel(960, 360));
        eprintln!("left {left}right {right}");
        // "0,0: (...)  #RRRRGGGGBBBB  ..." with 16-bit channels.
        let rgb = |text: &str| -> (u32, u32, u32) {
            let hex = &text[text.rfind('#').expect(text) + 1..][..12];
            let channel = |i: usize| u32::from_str_radix(&hex[i * 4..i * 4 + 4], 16).unwrap() >> 8;
            (channel(0), channel(1), channel(2))
        };
        let (r, g, b) = rgb(&left);
        assert!(r > 180 && g < 60 && b < 60, "left half red: {left}");
        let (r, g, b) = rgb(&right);
        assert!(r < 60 && g < 60 && b > 180, "right half blue: {right}");
        viewer.close(7);
        let mut events = Vec::new();
        while let Ok(event) = events_rx.recv_timeout(Duration::from_secs(5)) {
            let closed = event == ViewerEvent::Closed;
            events.push(event);
            if closed {
                break;
            }
        }
        // 1280x720 window, 16:9 frame: the image fills it.
        assert_eq!(events, [ViewerEvent::Height(720), ViewerEvent::Closed]);
    }
}
