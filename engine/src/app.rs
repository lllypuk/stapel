use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::audio::device;
use crate::{
    Config, Context, FixedClock, FrameStats, Game, RateCounter, Renderer, Shown, UiFrame, Vec2,
    step_frame, ui,
};

/// Ниже 4 FPS симуляция замедляется, а не догоняет лавиной тиков.
pub(crate) const MAX_FRAME: Duration = Duration::from_millis(250);

/// Как часто `Assets` проверяет файлы на диске.
const RELOAD_PERIOD: Duration = Duration::from_millis(500);

/// Склейка событий `winit` с `step_frame` и `Renderer`.
pub(crate) struct App<G> {
    config: Config,
    running: Option<Running<G>>,
}

struct Running<G> {
    // Раньше `ctx`: поля дропаются по порядку; иначе последний `Arc` звука умрёт в колбэке.
    _stream: Option<cpal::Stream>,
    window: Arc<Window>,
    renderer: Renderer,
    egui: egui::Context,
    egui_state: egui_winit::State,
    game: G,
    ctx: Context,
    clock: FixedClock,
    last_frame: Instant,
    last_reload: Instant,
    frames: FrameStats,
    tps: RateCounter,
    grab_warned: bool,
}

impl<G> App<G> {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            config,
            running: None,
        }
    }
}

impl<G: Game> ApplicationHandler for App<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        let (w, h) = self.config.size;
        let attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(LogicalSize::new(w, h));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let renderer = Renderer::new(window.clone(), self.config.vsync, self.config.pixel_perfect);
        let clock = FixedClock::new(self.config.tick_hz, MAX_FRAME);
        let mut ctx = Context::new(clock.step());
        ctx.focused = window.has_focus();
        let stream = device::open().map(|(stream, audio, rate)| {
            ctx.audio = audio;
            ctx.assets.set_sample_rate(rate);
            stream
        });
        let game = G::init(&mut ctx);
        let egui = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui.clone(),
            egui::ViewportId::ROOT,
            &window,
            None,
            None,
            None,
        );
        window.request_redraw();
        self.running = Some(Running {
            _stream: stream,
            window,
            renderer,
            egui,
            egui_state,
            game,
            ctx,
            clock,
            last_frame: Instant::now(),
            last_reload: Instant::now(),
            frames: FrameStats::new(),
            tps: RateCounter::new(),
            grab_warned: false,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(r) = self.running.as_mut() else {
            return;
        };
        let _ = r.egui_state.on_window_event(&r.window, &event);
        if !ui::to_input(&event, ui::wants_pointer(&r.egui, r.ctx.input.cursor())) {
            return;
        }
        match event {
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                // X11 присылает фальшивые нажатия удерживаемых клавиш при возврате фокуса.
                if is_synthetic {
                    return;
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    r.ctx.input.on_key(code, event.state, event.repeat);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => r.ctx.input.on_button(button, state),
            WindowEvent::CursorMoved { position, .. } => {
                let p = position.cast::<f32>();
                r.ctx.input.on_cursor(Some(Vec2::new(p.x, p.y)));
            }
            WindowEvent::CursorLeft { .. } => r.ctx.input.on_cursor(None),
            WindowEvent::Focused(focused) => {
                if !focused && r.ctx.grabbed {
                    release_cursor(&r.window);
                }
                r.ctx.on_focus(focused);
            }
            WindowEvent::Resized(size) => {
                r.renderer.resize(size);
                // Старый вьюпорт перевёл бы курсор в мир по кадру другого размера.
                r.ctx.shown = None;
                r.resume_redraw();
            }
            WindowEvent::Occluded(occluded) => {
                r.renderer.set_occluded(occluded);
                r.resume_redraw();
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => r.frame(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let (Some(r), DeviceEvent::MouseMotion { delta: (x, y) }) =
            (self.running.as_mut(), event)
        {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "сдвиг за событие — единицы-сотни, f32 хватает"
            )]
            r.ctx.on_mouse_motion(Vec2::new(x as f32, y as f32));
        }
    }
}

impl<G: Game> Running<G> {
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;

        if dt > MAX_FRAME {
            self.tps.reset();
        }
        if now - self.last_reload >= RELOAD_PERIOD {
            self.last_reload = now;
            self.ctx.assets.reload_changed();
        }
        self.egui
            .begin_pass(self.egui_state.take_egui_input(&self.window));
        let ticks = step_frame(&mut self.game, &mut self.ctx, &mut self.clock, dt);
        for _ in 0..ticks {
            self.tps.record(now);
        }
        self.sync_grab();
        self.game.ui(&self.egui);
        ui::release_focus(&self.egui);
        let out = self.egui.end_pass();
        self.egui_state
            .handle_platform_output(&self.window, out.platform_output);
        let ui = UiFrame {
            jobs: self.egui.tessellate(out.shapes, out.pixels_per_point),
            textures: out.textures_delta,
            pixels_per_point: out.pixels_per_point,
        };
        match self
            .renderer
            .render(&self.ctx.gfx, &mut self.ctx.assets, ui)
        {
            Ok(Some(viewport)) => {
                self.ctx.shown = Some(Shown {
                    camera: self.ctx.gfx.camera,
                    viewport,
                });
                self.frames
                    .record(Instant::now(), self.ctx.gfx.sprites.len());
            }
            Ok(None) => self.frames.reset(),
            Err(e) => {
                log::error!("{e}");
                event_loop.exit();
            }
        }
        if let Some(tps) = self.tps.per_second() {
            self.ctx.time.tps = tps;
        }
        if let Some(f) = self.frames.report() {
            self.ctx.time.fps = f.fps;
            log::info!(
                "fps {} tps {} worst {:.1}ms sprites {}..{}",
                f.fps,
                self.ctx.time.tps,
                f.worst.as_secs_f64() * 1e3,
                f.min_sprites,
                f.max_sprites
            );
            if let Some(a) = self.ctx.audio.report() {
                log::info!(
                    "audio cb {:.1}/{:.1}ms dropped {}",
                    a.callback_max.as_secs_f64() * 1e3,
                    a.budget.as_secs_f64() * 1e3,
                    a.dropped
                );
            }
        }
        // Цикл стоит в `Wait`, и следующий кадр просим только отсюда: на Wayland с vsync winit
        // отдаст его по сигналу композитора, а свёрнутому окну композитор кадров не даёт.
        self.resume_redraw();
    }

    /// Приводит захват окна к `ctx.cursor_grab`; `Locked` на X11 нет — тогда `Confined`.
    fn sync_grab(&mut self) {
        let want = self.ctx.cursor_grab && self.ctx.focused;
        if want == self.ctx.grabbed {
            return;
        }
        if !want {
            release_cursor(&self.window);
            self.ctx.grabbed = false;
            return;
        }
        let grab = self
            .window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined));
        match grab {
            Ok(()) => {
                self.window.set_cursor_visible(false);
                self.ctx.grabbed = true;
            }
            Err(e) => {
                self.ctx.cursor_grab = false;
                if !self.grab_warned {
                    self.grab_warned = true;
                    log::warn!("cursor grab: {e}");
                }
            }
        }
    }

    /// Цепочка кадров рвётся, пока поверхность недоступна; `Resized` и `Occluded` её возобновляют.
    fn resume_redraw(&self) {
        if self.renderer.is_available() {
            self.window.request_redraw();
        }
    }
}

/// Захват на Wayland не снимается сам ни при потере фокуса, ни при сворачивании.
fn release_cursor(window: &Window) {
    if let Err(e) = window.set_cursor_grab(CursorGrabMode::None) {
        log::warn!("cursor release: {e}");
    }
    window.set_cursor_visible(true);
}
