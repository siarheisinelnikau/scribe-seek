//! Reactive egui host with CPU rasterization and Win32 GDI bitmap presentation.
use crate::{app::App, storage::Storage};
use anyhow::{Context, Result};
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use std::{
    collections::hash_map::DefaultHasher,
    fs::{File, OpenOptions},
    hash::{Hash, Hasher},
    io::Write,
    num::NonZeroU32,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle},
    keyboard::{KeyCode, PhysicalKey},
    platform::windows::WindowAttributesExtWindows,
    window::{Icon, Window, WindowId},
};

#[path = "render_diagnostic.rs"]
mod diagnostic;

struct Repaint {
    when: Instant,
    pass: u64,
}

struct SoftwareWindow {
    app: App,
    surface: softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>,
    _context: softbuffer::Context<OwnedDisplayHandle>,
    window: Arc<Window>,
    input: egui_winit::State,
    viewport: egui::ViewportInfo,
    renderer: EguiSoftwareRender,
    pending_textures: egui::TexturesDelta,
    has_frame: bool,
    scene: Option<u64>,
    frame_interval: Duration,
}

struct Desktop {
    storage: Option<Storage>,
    source: Option<PathBuf>,
    context: egui::Context,
    window: Option<SoftwareWindow>,
    next_frame: Option<Instant>,
    last_frame: Option<Instant>,
    error: Option<anyhow::Error>,
    trace: Option<FrameTrace>,
    diagnostic: Option<diagnostic::Diagnostic>,
    counts: [u64; 3],
    cleanup_update: Option<PathBuf>,
}

impl Desktop {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let icon = Icon::from_rgba(include_bytes!("../assets/app-128.rgba").to_vec(), 128, 128)?;
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Scribe Seek")
                    .with_window_icon(Some(icon.clone()))
                    .with_taskbar_icon(Some(icon))
                    .with_inner_size(LogicalSize::new(1180.0, 800.0))
                    .with_min_inner_size(LogicalSize::new(960.0, 700.0)),
            )?,
        );
        let context =
            softbuffer::Context::new(event_loop.owned_display_handle()).map_err(|error| {
                anyhow::anyhow!("Could not create the GDI drawing context: {error}")
            })?;
        let surface = softbuffer::Surface::new(&context, window.clone())
            .map_err(|error| anyhow::anyhow!("Could not create the GDI bitmap surface: {error}"))?;
        let input = egui_winit::State::new(
            self.context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );
        let app = App::new(
            &self.context,
            self.storage
                .take()
                .context("Application storage was already opened")?,
            self.source.take(),
        );
        self.window = Some(SoftwareWindow {
            app,
            surface,
            _context: context,
            frame_interval: monitor_frame_interval(&window),
            window,
            input,
            viewport: Default::default(),
            renderer: EguiSoftwareRender::new(ColorFieldOrder::Bgra),
            pending_textures: Default::default(),
            has_frame: false,
            scene: None,
        });
        self.request_frame(Instant::now());
        Ok(())
    }

    fn request_frame(&mut self, when: Instant) {
        let earliest = self
            .last_frame
            .zip(self.window.as_ref())
            .map_or(when, |(last, state)| last + state.frame_interval);
        let when = when.max(earliest);
        self.next_frame = Some(self.next_frame.map_or(when, |pending| pending.min(when)));
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let started = Instant::now();
        self.last_frame = Some(started);
        self.next_frame = None;
        let Some(state) = self.window.as_mut() else {
            return Ok(());
        };
        if !state.window.is_visible().unwrap_or(true) || state.window.is_minimized() == Some(true) {
            // Keep worker events queued for the next visible frame; do not build
            // or rasterize a hidden scene merely because a worker has progressed.
            return Ok(());
        }
        egui_winit::update_viewport_info(&mut state.viewport, &self.context, &state.window, false);
        let mut input = state.input.take_egui_input(&state.window);
        input
            .viewports
            .insert(egui::ViewportId::ROOT, state.viewport.clone());
        state.viewport.events.clear();
        self.counts[0] += 1;
        let mut output = self.context.run(input, |context| state.app.draw(context));
        let ui_finished = Instant::now();
        state.pending_textures.append(output.textures_delta);
        state
            .input
            .handle_platform_output(&state.window, output.platform_output);

        let mut close = false;
        let mut commands = Vec::new();
        let mut screenshots = Vec::new();
        if let Some(viewport) = output.viewport_output.get_mut(&egui::ViewportId::ROOT) {
            if viewport.repaint_delay != Duration::MAX {
                self.next_frame =
                    started.checked_add(viewport.repaint_delay.max(state.frame_interval));
            }
            for command in viewport.commands.drain(..) {
                match command {
                    egui::ViewportCommand::Close => close = true,
                    egui::ViewportCommand::CancelClose => close = false,
                    egui::ViewportCommand::Screenshot(data) => screenshots.push(data),
                    command => commands.push(command),
                }
            }
        }
        egui_winit::process_viewport_commands(
            &self.context,
            &mut state.viewport,
            commands,
            &state.window,
            &mut Vec::new(),
        );
        if close {
            event_loop.exit();
            return Ok(());
        }
        let size = state.window.inner_size();
        if state.window.is_minimized() == Some(true)
            || !state.window.is_visible().unwrap_or(true)
            || size.width == 0
            || size.height == 0
        {
            // Fonts and other texture changes remain pending until restoration.
            self.next_frame = None;
            return Ok(());
        }
        state
            .surface
            .resize(
                NonZeroU32::new(size.width).context("Missing bitmap width")?,
                NonZeroU32::new(size.height).context("Missing bitmap height")?,
            )
            .map_err(|error| anyhow::anyhow!("Could not resize the GDI bitmap: {error}"))?;
        let primitives = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        let scene = scene_fingerprint(
            &primitives,
            size.width,
            size.height,
            output.pixels_per_point,
        );
        if state.has_frame
            && scene.is_some()
            && scene == state.scene
            && state.pending_textures.is_empty()
            && screenshots.is_empty()
        {
            // Input can change the cursor without changing any visible pixels.
            if let Some(trace) = &mut self.trace {
                trace.unchanged += 1;
            }
            return Ok(());
        }
        // Rasterize before locking the native drawing buffer.
        self.counts[1] += 1;
        state.renderer.render_to_canvas(
            size.width as usize,
            size.height as usize,
            &primitives,
            &state.pending_textures,
            output.pixels_per_point,
        );
        state.pending_textures = Default::default();
        let raster_finished = Instant::now();
        let mut buffer = state
            .surface
            .buffer_mut()
            .map_err(|error| anyhow::anyhow!("Could not access the GDI bitmap: {error}"))?;
        buffer.fill(0);
        state.renderer.blit_canvas_to_buffer(&mut BufferMutRef::new(
            bytemuck::cast_slice_mut(&mut buffer),
            size.width as usize,
            size.height as usize,
        ));
        if !screenshots.is_empty() {
            let pixels = buffer
                .iter()
                .map(|pixel| {
                    egui::Color32::from_rgb((pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8)
                })
                .collect();
            let image = Arc::new(egui::ColorImage::new(
                [size.width as usize, size.height as usize],
                pixels,
            ));
            for data in screenshots {
                state
                    .input
                    .egui_input_mut()
                    .events
                    .push(egui::Event::Screenshot {
                        viewport_id: egui::ViewportId::ROOT,
                        user_data: data,
                        image: image.clone(),
                    });
            }
            self.next_frame = Some(started + state.frame_interval);
        }
        buffer
            .present()
            .map_err(|error| anyhow::anyhow!("Could not present the GDI bitmap: {error}"))?;
        self.counts[2] += 1;
        state.has_frame = true;
        if let Some(directory) = self.cleanup_update.take() {
            rust_auto_updator::cleanup_after_update(directory, crate::updater::config()?.install);
        }
        state.scene = scene;
        if let Some(trace) = &mut self.trace {
            trace.record(
                started,
                ui_finished,
                raster_finished,
                output.pixels_per_point,
            );
        }
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler<Repaint> for Desktop {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(state) = self.window.as_mut() else {
            return;
        };
        if state.window.id() != id {
            return;
        }
        // With local tracing enabled, F12 snapshots counters without requesting
        // a UI frame. This makes idle repaint checks independent of paint timers.
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.physical_key == PhysicalKey::Code(KeyCode::F12)
            && let Some(trace) = &mut self.trace
        {
            if event.state == ElementState::Pressed && !event.repeat {
                trace.flush();
                trace.finish();
            }
            return;
        }
        if matches!(event, WindowEvent::CloseRequested) {
            event_loop.exit();
            return;
        }
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            state.has_frame = false;
        }
        if matches!(
            event,
            WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::Moved(_)
        ) {
            state.frame_interval = monitor_frame_interval(&state.window);
        }
        if let Some(trace) = &mut self.trace {
            trace.window_state(&state.window);
        }
        if matches!(event, WindowEvent::RedrawRequested) && state.has_frame {
            if !state.window.is_visible().unwrap_or(true)
                || state.window.is_minimized() == Some(true)
            {
                return;
            }
            // Restore unchanged content after an OS expose without rebuilding UI.
            if let Err(error) = state
                .surface
                .buffer_mut()
                .and_then(|buffer| buffer.present())
            {
                self.fail(
                    event_loop,
                    anyhow::anyhow!("Could not restore the GDI bitmap: {error}"),
                );
            }
            if let Some(trace) = &mut self.trace {
                trace.restored += 1;
            }
            self.counts[2] += 1;
            return;
        }
        let response = state.input.on_window_event(&state.window, &event);
        if response.repaint
            || matches!(
                event,
                WindowEvent::RedrawRequested | WindowEvent::Occluded(false)
            )
        {
            self.request_frame(Instant::now());
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, repaint: Repaint) {
        let pass = self.context.cumulative_pass_nr();
        if pass == repaint.pass || pass == repaint.pass.saturating_add(1) {
            self.request_frame(repaint.when);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(diagnostic) = &mut self.diagnostic {
            match diagnostic.poll(self.counts) {
                diagnostic::Action::Hide => {
                    if let Some(state) = &self.window {
                        state.window.set_visible(false);
                    }

                    self.next_frame = None;
                }
                diagnostic::Action::Show => {
                    if let Some(state) = &mut self.window {
                        state.window.set_visible(true);
                        state.window.set_minimized(false);
                        state.has_frame = false;
                    }
                    self.request_frame(Instant::now());
                }
                diagnostic::Action::Exit => {
                    event_loop.exit();
                    return;
                }
                diagnostic::Action::None => {}
            }
        }
        if self.next_frame.is_some_and(|when| when <= Instant::now())
            && let Err(error) = self.draw(event_loop)
        {
            self.fail(event_loop, error);
        }
        let deadline = match (
            self.next_frame,
            self.diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.deadline()),
        ) {
            (Some(frame), Some(diagnostic)) => Some(frame.min(diagnostic)),
            (frame, diagnostic) => frame.or(diagnostic),
        };
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.context.set_request_repaint_callback(|_| {});
        if let Some(trace) = &mut self.trace {
            trace.flush();
            trace.finish()
        }
        self.window = None;
    }
}

fn monitor_frame_interval(window: &Window) -> Duration {
    let millihertz = window
        .current_monitor()
        .and_then(|monitor| monitor.refresh_rate_millihertz())
        .unwrap_or(60_000)
        .max(1);
    Duration::from_secs_f64(1000.0 / f64::from(millihertz))
}

fn scene_fingerprint(
    primitives: &[egui::ClippedPrimitive],
    width: u32,
    height: u32,
    scale: f32,
) -> Option<u64> {
    let mut hash = DefaultHasher::new();
    (width, height, scale.to_bits(), primitives.len()).hash(&mut hash);
    for primitive in primitives {
        for value in [
            primitive.clip_rect.min.x,
            primitive.clip_rect.min.y,
            primitive.clip_rect.max.x,
            primitive.clip_rect.max.y,
        ] {
            value.to_bits().hash(&mut hash);
        }
        let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
            return None;
        };
        mesh.texture_id.hash(&mut hash);
        mesh.indices.hash(&mut hash);
        mesh.vertices.len().hash(&mut hash);
        for vertex in &mesh.vertices {
            for value in [vertex.pos.x, vertex.pos.y, vertex.uv.x, vertex.uv.y] {
                value.to_bits().hash(&mut hash);
            }
            vertex.color.to_array().hash(&mut hash);
        }
    }
    Some(hash.finish())
}

/// Opt-in timings cover layout, CPU rasterization and native presentation.
struct FrameTrace {
    file: File,
    since: Instant,
    frames: Vec<[f64; 3]>,
    scale: f32,
    started: Instant,
    presented: u64,
    unchanged: u64,
    restored: u64,
    minimized: Option<bool>,
}

impl FrameTrace {
    fn new() -> Option<Self> {
        let path = std::env::var_os("SCRIBE_SEEK_RENDER_TRACE")?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let _ = writeln!(
            file,
            "{}",
            serde_json::json!({"renderer":"CPU / Win32 GDI"})
        );
        Some(Self {
            file,
            since: Instant::now(),
            frames: Vec::new(),
            scale: 1.0,
            started: Instant::now(),
            presented: 0,
            unchanged: 0,
            restored: 0,
            minimized: None,
        })
    }
    fn record(&mut self, start: Instant, ui: Instant, raster: Instant, scale: f32) {
        self.presented += 1;
        self.frames.push([
            start.elapsed().as_secs_f64() * 1000.0,
            ui.duration_since(start).as_secs_f64() * 1000.0,
            raster.duration_since(ui).as_secs_f64() * 1000.0,
        ]);
        self.scale = scale;
        if self.since.elapsed() >= Duration::from_secs(1) {
            self.flush()
        }
    }
    fn window_state(&mut self, window: &Window) {
        let minimized = window.is_minimized().unwrap_or(false);
        if self.minimized != Some(minimized) {
            self.minimized = Some(minimized);
            let _ = writeln!(
                self.file,
                "{}",
                serde_json::json!({
                    "session_seconds":self.started.elapsed().as_secs_f64(),
                    "minimized":minimized,
                    "total_presented":self.presented,
                })
            );
        }
    }
    fn flush(&mut self) {
        if self.frames.is_empty() {
            return;
        }
        self.frames.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
        let count = self.frames.len();
        let _ = writeln!(
            self.file,
            "{}",
            serde_json::json!({
                "frames":count,
                "frame_median_ms":self.frames[count / 2][0],
                "frame_p95_ms":self.frames[(count - 1) * 95 / 100][0],
                "frame_max_ms":self.frames[count - 1][0],
                "ui_mean_ms":self.frames.iter().map(|f|f[1]).sum::<f64>() / count as f64,
                "raster_mean_ms":self.frames.iter().map(|f|f[2]).sum::<f64>() / count as f64,
                "pixels_per_point":self.scale,
            "session_seconds":self.started.elapsed().as_secs_f64(),
            "total_presented":self.presented,
            "unchanged_frames_skipped":self.unchanged,
            "bitmap_restores":self.restored,
            })
        );
        self.frames.clear();
        self.since = Instant::now();
    }
    fn finish(&mut self) {
        let _ = writeln!(
            self.file,
            "{}",
            serde_json::json!({
                "session_seconds":self.started.elapsed().as_secs_f64(),
                "total_presented":self.presented,
                "unchanged_frames_skipped":self.unchanged,
                "bitmap_restores":self.restored,
            })
        );
    }
}

pub fn run(
    storage: Storage,
    source: Option<PathBuf>,
    cleanup_update: Option<PathBuf>,
) -> Result<()> {
    let event_loop = EventLoop::<Repaint>::with_user_event().build()?;
    let context = egui::Context::default();
    let proxy = event_loop.create_proxy();
    context.set_request_repaint_callback(move |info| {
        if let Some(when) = Instant::now().checked_add(info.delay) {
            let _ = proxy.send_event(Repaint {
                when,
                pass: info.current_cumulative_pass_nr,
            });
        }
    });
    let mut desktop = Desktop {
        storage: Some(storage),
        source,
        context,
        window: None,
        next_frame: None,
        last_frame: None,
        error: None,
        trace: FrameTrace::new(),
        diagnostic: diagnostic::Diagnostic::from_env("SCRIBE_SEEK_RENDER_DIAGNOSTIC"),
        counts: [0; 3],
        cleanup_update,
    };
    event_loop.run_app(&mut desktop)?;
    if let Some(error) = desktop.error {
        return Err(error);
    }
    Ok(())
}
