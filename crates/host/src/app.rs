use std::cell::Cell;
use std::ffi::{c_char, CStr};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;

use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::prelude::{GlSurface, NotCurrentGlContext};
use glutin::surface::{Surface, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use kaplay_native_audio::{Audio, AudioEvent};
use kaplay_native_canvas2d::Canvas2d;
use kaplay_native_gfx::GlState;
use kaplay_native_net::{Network, NetworkEvent};
use kaplay_native_pak::{normalize, AssetSource};
use kaplay_native_steam::SteamApi;
use serde_json::json;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::raw_window_handle::HasWindowHandle;
use winit::window::Window;

use crate::config::HostConfig;
use crate::js::{JsArg, JsRuntime, Watchdog};
use crate::keys::KeyState;
use crate::state::{HostState, PendingMessage};

const SHIM: &str = include_str!("../../../shim/src/shim.js");
const DEFAULT_FONT: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf");
const FRAME_SAMPLES: usize = 10_000;

fn percentile(sorted: &[f32], fraction: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f32 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn sample_stats(samples: &std::collections::VecDeque<f32>) -> serde_json::Value {
    if samples.is_empty() {
        return json!({
            "avg": 0.0,
            "min": 0.0,
            "p50": 0.0,
            "p90": 0.0,
            "p95": 0.0,
            "p99": 0.0,
            "p999": 0.0,
            "max": 0.0,
            "sd": 0.0,
            "mad": 0.0,
            "samples": 0
        });
    }
    let mut sorted: Vec<f32> = samples.iter().copied().collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let average = samples.iter().sum::<f32>() / samples.len() as f32;
    let variance = samples
        .iter()
        .map(|value| (value - average) * (value - average))
        .sum::<f32>()
        / samples.len() as f32;
    let median = percentile(&sorted, 0.5);
    let mut deviations: Vec<f32> = samples.iter().map(|value| (value - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    json!({
        "avg": average,
        "min": percentile(&sorted, 0.0),
        "p50": median,
        "p90": percentile(&sorted, 0.9),
        "p95": percentile(&sorted, 0.95),
        "p99": percentile(&sorted, 0.99),
        "p999": percentile(&sorted, 0.999),
        "max": percentile(&sorted, 1.0),
        "sd": variance.sqrt(),
        "mad": percentile(&deviations, 0.5),
        "samples": samples.len(),
    })
}

fn over_budget(samples: &std::collections::VecDeque<f32>, limit_ms: f32) -> usize {
    samples.iter().filter(|value| **value > limit_ms).count()
}

fn frame_stats(samples: &std::collections::VecDeque<f32>) -> serde_json::Value {
    let mut stats = sample_stats(samples);
    if let Some(object) = stats.as_object_mut() {
        for (key, limit) in [
            ("over_16_7", 16.7_f32),
            ("over_33", 33.3),
            ("over_50", 50.0),
            ("over_100", 100.0),
        ] {
            object.insert(key.to_string(), json!(over_budget(samples, limit)));
        }
    }
    stats
}

pub struct HostApp {
    config: HostConfig,
    assets: Rc<AssetSource>,
    steam: Rc<std::cell::RefCell<Option<SteamApi>>>,
    inner: Option<Inner>,
    frames: u32,
    failed: bool,
    frame_ms: std::collections::VecDeque<f32>,
    phases: BenchSamples,
    renderer: Option<String>,
    started: std::time::Instant,
    stats_last: std::time::Instant,
    stats_frames: u32,
    stats_ms: f32,
}

#[derive(Default)]
struct BenchSamples {
    js: std::collections::VecDeque<f32>,
    drain: std::collections::VecDeque<f32>,
    network: std::collections::VecDeque<f32>,
    audio: std::collections::VecDeque<f32>,
    present: std::collections::VecDeque<f32>,
}

struct FramePhases {
    js_ms: f32,
    drain_ms: f32,
    network_ms: f32,
    audio_ms: f32,
    present_ms: f32,
}

impl BenchSamples {
    fn push(samples: &mut std::collections::VecDeque<f32>, value: f32) {
        if samples.len() == FRAME_SAMPLES {
            samples.pop_front();
        }
        samples.push_back(value);
    }

    fn record(&mut self, phases: FramePhases) {
        Self::push(&mut self.js, phases.js_ms);
        Self::push(&mut self.drain, phases.drain_ms);
        Self::push(&mut self.network, phases.network_ms);
        Self::push(&mut self.audio, phases.audio_ms);
        Self::push(&mut self.present, phases.present_ms);
    }
}

struct Inner {
    window: Arc<Window>,
    gl_context: PossiblyCurrentContext,
    gl_surface: Surface<WindowSurface>,
    js: JsRuntime,
    state: Rc<HostState>,
    keys: KeyState,
    caps_lock: Cell<bool>,
    modifiers: ModifiersState,
    cursor_position: Cell<(f64, f64)>,
    watchdog: Arc<Watchdog>,
    last_present: Cell<(i32, i32, i32, i32)>,
    last_resize: Cell<(u32, u32)>,
}

impl Inner {
    fn mark(&self, phase: &str) {
        self.watchdog.mark(phase);
    }

    fn unmark(&self) {
        self.watchdog.unmark();
    }

    fn drain(&self) {
        self.mark("drain");
        self.js.drain_jobs();
        self.unmark();
    }

    fn pump_audio(&self) {
        let Some(audio) = self.state.audio.as_ref() else {
            return;
        };
        for event in audio.take_events() {
            match event {
                AudioEvent::Decoded {
                    request,
                    buffer,
                    error,
                } => {
                    let ok = error.is_none();
                    let message = error.unwrap_or_default();
                    if let Err(error) = self.js.call_global(
                        "__kapNative_onAudioDecoded",
                        &[
                            JsArg::Uint(request),
                            JsArg::Uint(buffer),
                            JsArg::Bool(ok),
                            JsArg::Str(&message),
                        ],
                    ) {
                        log::error!("audio decode callback {request}: {error}");
                    }
                }
                AudioEvent::VoiceEnded { voice } => {
                    if let Err(error) = self
                        .js
                        .call_global("__kapNative_onVoiceEnded", &[JsArg::Uint(voice)])
                    {
                        log::error!("audio ended callback {voice}: {error}");
                    }
                }
            }
        }
    }

    fn pump_network(&self) {
        let Some(network) = self.state.network.as_ref() else {
            return;
        };
        for _ in 0..8 {
            let events = network.drain(64);
            if events.is_empty() {
                break;
            }
            for event in events {
                self.dispatch_network(event);
            }
            self.drain();
        }
    }

    fn dispatch_network(&self, event: NetworkEvent) {
        match event {
            NetworkEvent::Response(response) => {
                let id = response.id;
                let url = response.url.clone();
                let status = response.status;
                let status_text = response.status_text.clone();
                let headers =
                    serde_json::to_string(&response.headers).unwrap_or_else(|_| "[]".to_string());
                let length = response.body.len();
                let ok = (200..300).contains(&status);
                *self.state.pending_response.borrow_mut() = Some(response);
                let result = self.js.call_global(
                    "__kapNative_onFetch",
                    &[
                        JsArg::Uint(id),
                        JsArg::Str(&url),
                        JsArg::Uint(u32::from(status)),
                        JsArg::Str(&status_text),
                        JsArg::Str(&headers),
                        JsArg::Uint(u32::try_from(length).unwrap_or(u32::MAX)),
                        JsArg::Bool(ok),
                    ],
                );
                self.state.pending_response.borrow_mut().take();
                if let Err(error) = result {
                    log::error!("fetch callback {id}: {error}");
                }
            }
            NetworkEvent::FetchError { id, message } => {
                log::debug!("fetch {id} failed: {message}");
                if let Err(error) = self.js.call_global(
                    "__kapNative_onFetchError",
                    &[JsArg::Uint(id), JsArg::Str(&message)],
                ) {
                    log::error!("fetch callback {id}: {error}");
                }
            }
            NetworkEvent::SocketOpened { id } => {
                if let Err(error) = self
                    .js
                    .call_global("__kapNative_onSocketOpen", &[JsArg::Uint(id)])
                {
                    log::error!("socket callback {id}: {error}");
                }
            }
            NetworkEvent::SocketMessage { id, text, data } => {
                let length = data.len();
                *self.state.pending_message.borrow_mut() =
                    Some(PendingMessage { socket: id, data });
                let result = self.js.call_global(
                    "__kapNative_onSocketMessage",
                    &[
                        JsArg::Uint(id),
                        JsArg::Uint(u32::try_from(length).unwrap_or(u32::MAX)),
                        JsArg::Bool(text),
                    ],
                );
                self.state.pending_message.borrow_mut().take();
                if let Err(error) = result {
                    log::error!("socket callback {id}: {error}");
                }
            }
            NetworkEvent::SocketClosed { id, code, reason } => {
                log::debug!("socket {id} closed: {code} {reason}");
                if let Err(error) = self.js.call_global(
                    "__kapNative_onSocketClose",
                    &[
                        JsArg::Uint(id),
                        JsArg::Uint(u32::from(code)),
                        JsArg::Str(&reason),
                    ],
                ) {
                    log::error!("socket callback {id}: {error}");
                }
            }
            NetworkEvent::SocketError { id, message } => {
                log::debug!("socket {id} error: {message}");
                if let Err(error) = self.js.call_global(
                    "__kapNative_onSocketError",
                    &[JsArg::Uint(id), JsArg::Str(&message)],
                ) {
                    log::error!("socket callback {id}: {error}");
                }
            }
        }
    }

    fn dispatch(&self, event: &str, payload: serde_json::Value) {
        let payload = payload.to_string();
        let event = event.to_string();
        let result = self.js.call_global(
            "__kapNative_dispatch",
            &[JsArg::Str(&event), JsArg::Str(&payload)],
        );
        if let Err(error) = result {
            log::error!("dispatch {event}: {error}");
        }
        self.drain();
    }

    fn dispatch_resize(&self, size: winit::dpi::PhysicalSize<u32>) {
        self.last_resize.set((size.width, size.height));
        if let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        {
            self.gl_surface.resize(&self.gl_context, width, height);
        }
        let scale = self.window.scale_factor();
        let logical = size.to_logical::<f64>(scale);
        self.dispatch(
            "resize",
            json!({
                "width": logical.width,
                "height": logical.height,
                "framebufferWidth": size.width,
                "framebufferHeight": size.height,
                "dpr": scale,
            }),
        );
    }

    fn frame(&self) -> FramePhases {
        let size = self.window.inner_size();
        if self.last_resize.get() != (size.width, size.height) {
            self.dispatch_resize(size);
        }
        let network_start = std::time::Instant::now();
        self.pump_network();
        let network_ms = network_start.elapsed().as_secs_f32() * 1000.0;
        let audio_start = std::time::Instant::now();
        self.pump_audio();
        let audio_ms = audio_start.elapsed().as_secs_f32() * 1000.0;
        let now = self.state.now_ms();
        self.mark("frame update");
        let js_start = std::time::Instant::now();
        let result = self
            .js
            .call_global("__kapNative_onFrame", &[JsArg::Float(now)]);
        let js_ms = js_start.elapsed().as_secs_f32() * 1000.0;
        self.unmark();
        if let Err(error) = result {
            log::error!("frame: {error}");
        }
        let drain_start = std::time::Instant::now();
        self.drain();
        let drain_ms = drain_start.elapsed().as_secs_f32() * 1000.0;

        if let Some(api) = self.state.steam.borrow().as_ref() {
            api.run_callbacks();
        }

        let size = self.window.inner_size();
        let (buffer_width, buffer_height) = self.state.gl.borrow().drawing_buffer_size();
        let present_size = (
            size.width as i32,
            size.height as i32,
            buffer_width,
            buffer_height,
        );
        if self.last_present.get() != present_size {
            log::debug!(
                "window {}x{}, drawing buffer {buffer_width}x{buffer_height}",
                size.width,
                size.height
            );
            self.last_present.set(present_size);
        }
        let present_start = std::time::Instant::now();
        self.state
            .gl
            .borrow_mut()
            .present(size.width as i32, size.height as i32, || {
                self.window.pre_present_notify();
                if let Err(error) = self.gl_surface.swap_buffers(&self.gl_context) {
                    log::error!("swap buffers: {error}");
                }
            });
        FramePhases {
            js_ms,
            drain_ms,
            network_ms,
            audio_ms,
            present_ms: present_start.elapsed().as_secs_f32() * 1000.0,
        }
    }
}

fn find_module_entry(html: &str) -> Option<String> {
    let mut candidates: Vec<(bool, String)> = Vec::new();
    let mut rest = html;
    while let Some(index) = rest.find("<script") {
        rest = &rest[index + "<script".len()..];
        let end = rest.find('>')?;
        let tag = &rest[..end];
        rest = &rest[end..];
        let Some(src) = attribute(tag, "src") else {
            continue;
        };
        let module =
            attribute(tag, "type").is_some_and(|value| value.eq_ignore_ascii_case("module"));
        candidates.push((module, src.to_string()));
    }
    let (_, src) = candidates
        .iter()
        .find(|(module, _)| *module)
        .or_else(|| candidates.first())?;
    normalize(src).filter(|path| !path.is_empty())
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let mut rest = tag;
    while let Some(index) = rest.find(&needle) {
        let boundary = rest[..index].chars().last().is_none_or(char::is_whitespace);
        let after = &rest[index + needle.len()..];
        if boundary {
            if let Some(quote @ ('"' | '\'')) = after.chars().next() {
                let value = &after[1..];
                return value.find(quote).map(|end| &value[..end]);
            }
        }
        rest = after;
    }
    None
}

fn gl_version(display: &impl GlDisplay) -> Option<(u32, u32)> {
    let pointer = display.get_proc_address(c"glGetString");
    if pointer.is_null() {
        return None;
    }
    let get_string: unsafe extern "C" fn(u32) -> *const c_char =
        unsafe { std::mem::transmute(pointer) };
    let value = unsafe { get_string(glow::VERSION) };
    if value.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    let mut parts = text
        .split(['.', ' '])
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    Some((major, minor))
}

impl HostApp {
    pub fn new(config: HostConfig, assets: Rc<AssetSource>) -> anyhow::Result<Self> {
        let steam = if config.steam { SteamApi::load() } else { None };
        if let (Some(api), Some(app_id)) = (steam.as_ref(), config.app_id) {
            if api.restart_app_if_necessary(app_id) {
                log::info!("relaunching through Steam");
                std::process::exit(0);
            }
        }
        if let Some(api) = steam.as_ref() {
            log::info!(
                "steam library loaded (client running: {})",
                api.is_client_running()
            );
        }
        Ok(Self {
            config,
            assets,
            steam: Rc::new(std::cell::RefCell::new(steam)),
            inner: None,
            frames: 0,
            failed: false,
            frame_ms: std::collections::VecDeque::new(),
            phases: BenchSamples::default(),
            renderer: None,
            started: std::time::Instant::now(),
            stats_last: std::time::Instant::now(),
            stats_frames: 0,
            stats_ms: 0.0,
        })
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        let mut builder = {
            let mut builder = EventLoop::builder();
            if self.config.x11 {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11();
            }
            builder
        };
        #[cfg(not(target_os = "linux"))]
        let mut builder = EventLoop::builder();
        let event_loop = builder.build()?;
        event_loop.set_control_flow(ControlFlow::Poll);
        let result = event_loop.run_app(&mut self);
        self.log_frame_stats();
        self.write_bench_json();
        self.write_frame_trace();
        if let Some(api) = self.steam.borrow_mut().as_mut() {
            api.shutdown();
        }
        result?;
        if self.failed {
            anyhow::bail!("failed to create a window with an OpenGL 2.1 context");
        }
        Ok(())
    }

    fn create(&mut self, event_loop: &ActiveEventLoop) -> Option<Inner> {
        if let Some(api) = self.steam.borrow_mut().as_mut() {
            match api.init() {
                Ok(()) => log::info!("steam initialised"),
                Err(error) => log::warn!("steam unavailable: {error}"),
            }
        }
        let attributes = Window::default_attributes()
            .with_title(self.config.game_name.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.window.0 as f64,
                self.config.window.1 as f64,
            ))
            .with_visible(!self.config.headless);

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_depth_size(24)
            .with_stencil_size(8);

        let (window, gl_config) = DisplayBuilder::new()
            .with_window_attributes(Some(attributes))
            .build(event_loop, template, |mut configs| configs.next().unwrap())
            .ok()?;
        let window = Arc::new(window?);
        let raw_window_handle = window.window_handle().ok()?.as_raw();
        let display = gl_config.display();

        let context_attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(2, 1))))
            .build(Some(raw_window_handle));
        let not_current =
            unsafe { display.create_context(&gl_config, &context_attributes) }.ok()?;
        let surface_attributes = window.build_surface_attributes(Default::default()).ok()?;
        let surface =
            unsafe { display.create_window_surface(&gl_config, &surface_attributes) }.ok()?;
        let gl_context = not_current.make_current(&surface).ok()?;
        let interval = if self.config.no_vsync {
            log::info!("vertical sync disabled");
            SwapInterval::DontWait
        } else {
            SwapInterval::Wait(NonZeroU32::new(1).unwrap())
        };
        let _ = surface.set_swap_interval(&gl_context, interval);

        match gl_version(&display) {
            Some(version) if version < (2, 1) => {
                log::error!(
                    "OpenGL {}.{} is too old, 2.1 or newer is required",
                    version.0,
                    version.1
                );
                return None;
            }
            Some(version) => log::info!("OpenGL {}.{}", version.0, version.1),
            None => log::warn!("could not determine the OpenGL version"),
        }

        let glow_context = unsafe {
            glow::Context::from_loader_function_cstr(|symbol| {
                display.get_proc_address(symbol) as *const _
            })
        };
        let renderer_name = unsafe { glow_context.get_parameter_string(glow::RENDERER) };
        unsafe {
            log::info!(
                "renderer: {} ({}, OpenGL {})",
                renderer_name,
                glow_context.get_parameter_string(glow::VENDOR),
                glow_context.get_parameter_string(glow::VERSION)
            );
        }
        self.renderer = Some(renderer_name);
        let mut gl = GlState::new(glow_context);
        let size = window.inner_size();
        gl.resize_drawing_buffer(size.width as i32, size.height as i32);

        let save_path = self.config.save_dir.join("storage.json");
        let network = self.config.allow_network.then(|| Rc::new(Network::new(4)));
        if network.is_some() {
            log::info!("network access enabled");
        }
        let audio = if self.config.no_audio {
            Rc::new(Audio::silent())
        } else {
            Rc::new(Audio::new())
        };
        if audio.has_device() {
            log::info!("audio output enabled at {} Hz", audio.sample_rate());
        } else {
            log::warn!("audio output unavailable, playing silently");
        }
        let state = Rc::new(HostState {
            gl: Rc::new(std::cell::RefCell::new(gl)),
            canvas: Rc::new(std::cell::RefCell::new(Canvas2d::new(
                DEFAULT_FONT.to_vec(),
            ))),
            images: std::cell::RefCell::new(std::collections::HashMap::new()),
            videos: std::cell::RefCell::new(std::collections::HashMap::new()),
            next_image: Cell::new(1),
            next_video: Cell::new(1),
            assets: self.assets.clone(),
            window: window.clone(),
            quit: Cell::new(false),
            fullscreen: Cell::new(self.config.fullscreen),
            cursor_locked: Cell::new(false),
            cursor: std::cell::RefCell::new("default".to_string()),
            start: std::time::Instant::now(),
            save_path,
            platform: std::env::consts::OS,
            host_version: env!("CARGO_PKG_VERSION"),
            allow_network: self.config.allow_network,
            start_url: self
                .config
                .url
                .clone()
                .unwrap_or_else(|| "kaplay-native://game/".to_string()),
            network,
            audio: Some(audio),
            pending_response: std::cell::RefCell::new(None),
            pending_message: std::cell::RefCell::new(None),
            game_name: self.config.game_name.clone(),
            steam: self.steam.clone(),
        });

        if self.config.fullscreen {
            window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }

        let watchdog = Watchdog::new();
        if let Some(app_id) = self.config.app_id {
            log::info!("steam app id: {app_id}");
        }
        let js = match JsRuntime::new(self.config.js_engine, state.clone(), SHIM, watchdog.clone())
        {
            Ok(js) => js,
            Err(error) => {
                log::error!("failed to initialise runtime: {error}");
                event_loop.exit();
                return None;
            }
        };
        match js.version() {
            Some(version) => log::info!("javascript engine: {} {version}", js.name()),
            None => log::info!("javascript engine: {}", js.name()),
        }

        let preloads = self.config.preload.clone();
        let html = ["index.html", "public/index.html"]
            .iter()
            .find_map(|path| state.assets.read_text(path));
        if let Some(html) = &html {
            if let Err(error) = js.call_global("__kapNative_loadDocument", &[JsArg::Str(html)]) {
                log::error!("document error: {error}");
                event_loop.exit();
                return None;
            }
        }
        let entry_source = if let Some(script) = &self.config.script {
            std::fs::read_to_string(script).ok().map(|source| {
                let name = script
                    .canonicalize()
                    .unwrap_or_else(|_| script.clone())
                    .to_string_lossy()
                    .into_owned();
                (name, source)
            })
        } else {
            html.as_ref()
                .and_then(|html| find_module_entry(html))
                .and_then(|path| state.assets.read_text(&path).map(|source| (path, source)))
                .or_else(|| {
                    ["main.js", "game/main.js", "index.js"]
                        .iter()
                        .find_map(|path| {
                            state
                                .assets
                                .read_text(path)
                                .map(|source| ((*path).to_string(), source))
                        })
                })
        };
        let mut preload_sources: Vec<String> = Vec::new();
        for preload in &preloads {
            match std::fs::read_to_string(preload) {
                Ok(source) => preload_sources.push(source),
                Err(error) => {
                    log::error!("failed to read preload {}: {error}", preload.display());
                    event_loop.exit();
                    return None;
                }
            }
        }
        for source in preload_sources {
            if let Err(error) = js.eval_preload(&source) {
                log::error!("preload error: {error}");
                event_loop.exit();
                return None;
            }
        }
        let Some((name, source)) = entry_source else {
            log::error!(
                "game entry script not found; provide --script or a directory containing index.html or main.js"
            );
            event_loop.exit();
            return None;
        };
        if let Err(error) = js.eval_module(&name, &source) {
            log::error!("script error: {error}");
            event_loop.exit();
            return None;
        }

        let initial_size = window.inner_size();
        Some(Inner {
            window,
            gl_context,
            gl_surface: surface,
            js,
            state,
            keys: KeyState::new(),
            caps_lock: Cell::new(false),
            modifiers: ModifiersState::empty(),
            cursor_position: Cell::new((0.0, 0.0)),
            watchdog,
            last_present: Cell::new((0, 0, 0, 0)),
            last_resize: Cell::new((initial_size.width, initial_size.height)),
        })
    }

    fn capture_screenshot(&self, path: &std::path::Path) {
        let Some(inner) = self.inner.as_ref() else {
            return;
        };
        let (width, height, pixels) = inner.state.gl.borrow_mut().read_default_pixels_top_down();
        match crate::screenshots::encode_bytes(&pixels, width, height) {
            Some(png) => {
                if let Err(error) = std::fs::write(path, png) {
                    log::error!("failed to write screenshot: {error}");
                }
            }
            None => log::error!("failed to encode screenshot"),
        }
    }

    fn capture_screenshot_raw(&self, path: &std::path::Path) {
        let Some(inner) = self.inner.as_ref() else {
            return;
        };
        let (_, _, pixels) = inner.state.gl.borrow_mut().read_default_pixels_top_down();
        if let Err(error) = std::fs::write(path, pixels) {
            log::error!("failed to write raw screenshot: {error}");
        }
    }

    fn record_frame(&mut self, ms: f32) {
        if self.frame_ms.len() == FRAME_SAMPLES {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(ms);
        if !self.config.stats {
            return;
        }
        self.stats_frames += 1;
        self.stats_ms += ms;
        let elapsed = self.stats_last.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            log::info!(
                "{:.1} fps ({:.2} ms avg over {} frames)",
                self.stats_frames as f32 / elapsed,
                self.stats_ms / self.stats_frames as f32,
                self.stats_frames
            );
            self.stats_last = std::time::Instant::now();
            self.stats_frames = 0;
            self.stats_ms = 0.0;
        }
    }

    fn record_phases(&mut self, phases: FramePhases) {
        self.phases.record(phases);
    }

    fn log_frame_stats(&self) {
        if self.frame_ms.is_empty() {
            return;
        }
        let mut sorted: Vec<f32> = self.frame_ms.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let elapsed = self.started.elapsed().as_secs_f32();
        log::info!(
            "{} frames in {:.2} s: {:.1} fps, {:.2} ms avg, {:.2} ms p50, {:.2} ms p99",
            self.frame_ms.len(),
            elapsed,
            self.frame_ms.len() as f32 / elapsed,
            self.frame_ms.iter().sum::<f32>() / self.frame_ms.len() as f32,
            percentile(&sorted, 0.5),
            percentile(&sorted, 0.99)
        );
    }

    fn write_bench_json(&self) {
        let Some(path) = self.config.bench_json.as_ref() else {
            return;
        };
        let elapsed = self.started.elapsed().as_secs_f64();
        let summary = json!({
            "engine": self.config.js_engine.name(),
            "renderer": self.renderer.clone().unwrap_or_default(),
            "frames": self.frames,
            "duration_s": elapsed,
            "fps": if elapsed > 0.0 { self.frames as f64 / elapsed } else { 0.0 },
            "frame_ms": frame_stats(&self.frame_ms),
            "phases_ms": {
                "js": sample_stats(&self.phases.js),
                "drain": sample_stats(&self.phases.drain),
                "network": sample_stats(&self.phases.network),
                "audio": sample_stats(&self.phases.audio),
                "present": sample_stats(&self.phases.present),
            },
        });
        let bytes = serde_json::to_vec_pretty(&summary).unwrap_or_default();
        if let Err(error) = std::fs::write(path, bytes) {
            log::error!("bench json {}: {error}", path.display());
        }
    }

    fn write_frame_trace(&self) {
        let Some(path) = self.config.frame_trace.as_ref() else {
            return;
        };
        let trace = json!({
            "engine": self.config.js_engine.name(),
            "frames": self.frames,
            "frame_ms": self.frame_ms.iter().copied().collect::<Vec<f32>>(),
            "phases_ms": {
                "js": self.phases.js.iter().copied().collect::<Vec<f32>>(),
                "drain": self.phases.drain.iter().copied().collect::<Vec<f32>>(),
                "network": self.phases.network.iter().copied().collect::<Vec<f32>>(),
                "audio": self.phases.audio.iter().copied().collect::<Vec<f32>>(),
                "present": self.phases.present.iter().copied().collect::<Vec<f32>>(),
            },
        });
        let bytes = serde_json::to_vec(&trace).unwrap_or_default();
        if let Err(error) = std::fs::write(path, bytes) {
            log::error!("frame trace {}: {error}", path.display());
        }
    }

    fn maybe_exit(&mut self, event_loop: &ActiveEventLoop) {
        let Some(inner) = self.inner.as_ref() else {
            return;
        };
        let finished =
            inner.state.quit.get() || self.config.frames.is_some_and(|limit| self.frames >= limit);
        if finished {
            if let Some(path) = self.config.screenshot.clone() {
                self.capture_screenshot(&path);
            }
            if let Some(path) = self.config.screenshot_raw.clone() {
                self.capture_screenshot_raw(&path);
            }
            event_loop.exit();
        }
    }
}

impl ApplicationHandler for HostApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.inner.is_some() {
            return;
        }
        self.inner = self.create(event_loop);
        if self.inner.is_none() {
            self.failed = true;
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                inner.dispatch_resize(size);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                inner.dispatch_resize(inner.window.inner_size());
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::CapsLock)
                    && event.state == ElementState::Pressed
                    && !event.repeat
                {
                    inner.caps_lock.set(!inner.caps_lock.get());
                }
                let payload = inner.keys.event(&event);
                let event_name = match event.state {
                    ElementState::Pressed => "keydown",
                    ElementState::Released => "keyup",
                };
                inner.dispatch(
                    event_name,
                    json!({
                        "key": payload.key,
                        "code": payload.code,
                        "repeat": payload.repeat,
                        "shiftKey": inner.modifiers.shift_key(),
                        "ctrlKey": inner.modifiers.control_key(),
                        "altKey": inner.modifiers.alt_key(),
                        "metaKey": inner.modifiers.super_key(),
                        "capsLock": inner.caps_lock.get(),
                    }),
                );
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                inner.modifiers = modifiers.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = inner.window.scale_factor();
                let logical = position.to_logical::<f64>(scale);
                let last = inner.cursor_position.get();
                inner.cursor_position.set((logical.x, logical.y));
                inner.dispatch(
                    "mousemove",
                    json!({
                        "clientX": logical.x,
                        "clientY": logical.y,
                        "movementX": logical.x - last.0,
                        "movementY": logical.y - last.1,
                        "dpr": scale,
                    }),
                );
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (name, button) = match (state, button) {
                    (ElementState::Pressed, MouseButton::Left) => ("mousedown", 0),
                    (ElementState::Released, MouseButton::Left) => ("mouseup", 0),
                    (ElementState::Pressed, MouseButton::Middle) => ("mousedown", 1),
                    (ElementState::Released, MouseButton::Middle) => ("mouseup", 1),
                    (ElementState::Pressed, MouseButton::Right) => ("mousedown", 2),
                    (ElementState::Released, MouseButton::Right) => ("mouseup", 2),
                    (ElementState::Pressed, MouseButton::Back) => ("mousedown", 3),
                    (ElementState::Released, MouseButton::Back) => ("mouseup", 3),
                    (ElementState::Pressed, MouseButton::Forward) => ("mousedown", 4),
                    (ElementState::Released, MouseButton::Forward) => ("mouseup", 4),
                    (ElementState::Pressed, MouseButton::Other(value)) => {
                        ("mousedown", value as i32)
                    }
                    (ElementState::Released, MouseButton::Other(value)) => {
                        ("mouseup", value as i32)
                    }
                };
                let (x, y) = inner.cursor_position.get();
                inner.dispatch(
                    name,
                    json!({
                        "button": button,
                        "clientX": x,
                        "clientY": y,
                    }),
                );
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        (x as f64 * 100.0, y as f64 * 100.0)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(position) => {
                        (position.x, position.y)
                    }
                };
                inner.dispatch("wheel", json!({ "deltaX": x, "deltaY": y }));
            }
            WindowEvent::Focused(focused) => {
                inner.dispatch(if focused { "focus" } else { "blur" }, json!({}));
            }
            WindowEvent::Occluded(occluded) => {
                inner.dispatch("visibilitychange", json!({ "hidden": occluded }));
            }
            WindowEvent::RedrawRequested => {
                let started = std::time::Instant::now();
                let phases = inner.frame();
                self.record_phases(phases);
                self.record_frame(started.elapsed().as_secs_f32() * 1000.0);
                self.frames += 1;
                self.maybe_exit(event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(inner) = self.inner.as_ref() {
            if inner.state.quit.get() {
                self.maybe_exit(event_loop);
                return;
            }
            inner.window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{find_module_entry, frame_stats, percentile};

    #[test]
    fn percentiles_land_on_expected_samples() {
        let mut samples: Vec<f32> = (1..=100).map(|value| value as f32).collect();
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(percentile(&samples, 0.5), 51.0);
        assert_eq!(percentile(&samples, 0.99), 99.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn frame_stats_count_samples_over_common_budgets() {
        let samples: std::collections::VecDeque<f32> =
            [2.0, 10.0, 20.0, 40.0, 120.0].into_iter().collect();
        let stats = frame_stats(&samples);
        assert_eq!(stats["over_16_7"], 3);
        assert_eq!(stats["over_33"], 2);
        assert_eq!(stats["over_50"], 1);
        assert_eq!(stats["over_100"], 1);
        assert_eq!(stats["min"], 2.0);
        assert_eq!(stats["max"], 120.0);
        assert_eq!(stats["mad"], 18.0);
        let sd = stats["sd"].as_f64().unwrap();
        assert!((sd - 42.734).abs() < 0.01, "sd was {sd}");
    }

    #[test]
    fn finds_the_module_entry_in_a_web_build() {
        assert_eq!(
            find_module_entry(
                r#"<html><body><div id="root"></div><script type="module" crossorigin src="/assets/index-LX8yxwYF.js"></script></body></html>"#
            ),
            Some("assets/index-LX8yxwYF.js".to_string())
        );
    }

    #[test]
    fn normalizes_module_entry_paths() {
        assert_eq!(
            find_module_entry(r#"<script type="module" src="./assets/index.js?v=1"></script>"#),
            Some("assets/index.js".to_string())
        );
        assert_eq!(
            find_module_entry(r#"<script type="module" src="assets\\index.js"></script>"#),
            Some("assets/index.js".to_string())
        );
        assert_eq!(
            find_module_entry(r#"<script type="module" src="/"></script>"#),
            None
        );
    }

    #[test]
    fn prefers_module_scripts_and_falls_back_to_any_script() {
        assert_eq!(
            find_module_entry(
                r#"<script src="legacy.js"></script><script type='module' src='game.js'></script>"#
            ),
            Some("game.js".to_string())
        );
        assert_eq!(
            find_module_entry(r#"<script src="main.js"></script>"#),
            Some("main.js".to_string())
        );
        assert_eq!(find_module_entry("<html></html>"), None);
    }
}
