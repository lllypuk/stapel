use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::audio::device;
use crate::runner::{STALL_TIMEOUT, stalled};
use crate::{
    Config, Context, CursorGrab, FixedClock, FrameStats, Game, InputEvent, RateCounter, Renderer,
    RunError, RunReport, Runner, Script, Vec2, step_visual, ui, wheel_lines,
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
        ctx.seed = clock_seed();
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
        let cursor = r.ctx.input.cursor();
        let input = match event {
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => match event.physical_key {
                // X11 присылает фальшивые нажатия удерживаемых клавиш при возврате фокуса.
                PhysicalKey::Code(code) if !is_synthetic => Some(InputEvent::Key {
                    code,
                    state: event.state,
                    repeat: event.repeat,
                }),
                _ => None,
            },
            WindowEvent::MouseInput { state, button, .. } => {
                Some(InputEvent::Button { button, state })
            }
            WindowEvent::MouseWheel { delta, .. } => {
                #[expect(clippy::cast_possible_truncation, reason = "масштаб окна — 1–4")]
                let scale = r.window.scale_factor() as f32;
                Some(InputEvent::Wheel(wheel_lines(delta, scale)))
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = position.cast::<f32>();
                Some(InputEvent::Cursor(Some(Vec2::new(p.x, p.y))))
            }
            WindowEvent::CursorLeft { .. } => Some(InputEvent::Cursor(None)),
            WindowEvent::Focused(focused) => {
                if !focused && r.ctx.grabbed {
                    release_cursor(&r.window);
                }
                Some(InputEvent::Focus(focused))
            }
            WindowEvent::Resized(size) => {
                r.renderer.resize(size);
                // Старый вьюпорт перевёл бы курсор в мир по кадру другого размера.
                r.ctx.shown = None;
                r.resume_redraw();
                None
            }
            WindowEvent::Occluded(occluded) => {
                r.renderer.set_occluded(occluded);
                r.resume_redraw();
                None
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
                None
            }
            WindowEvent::RedrawRequested => {
                r.frame(event_loop);
                None
            }
            _ => None,
        };
        if let Some(input) = input
            && ui::to_input(&input, ui::wants_pointer(&r.egui, cursor))
        {
            r.ctx.apply(input);
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
            r.ctx
                .apply(InputEvent::Motion(Vec2::new(x as f32, y as f32)));
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
        let raw = self.egui_state.take_egui_input(&self.window);
        let mut grab = WindowGrab {
            window: &self.window,
            warned: &mut self.grab_warned,
        };
        let frame = step_visual(
            &mut self.game,
            &mut self.ctx,
            &mut self.clock,
            &self.egui,
            raw,
            dt,
            &mut grab,
        );
        for _ in 0..frame.ticks {
            self.tps.record(now);
        }
        self.egui_state
            .handle_platform_output(&self.window, frame.platform_output);
        let ui = frame.ui;
        match self
            .renderer
            .render(&self.ctx.gfx, &mut self.ctx.assets, ui)
        {
            Ok(Some(shown)) => {
                self.ctx.shown = Some(shown);
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

    /// Цепочка кадров рвётся, пока поверхность недоступна; `Resized` и `Occluded` её возобновляют.
    fn resume_redraw(&self) {
        if self.renderer.is_available() {
            self.window.request_redraw();
        }
    }
}

/// Захват курсора окна; `Locked` на X11 нет — тогда `Confined`.
struct WindowGrab<'a> {
    window: &'a Window,
    warned: &'a mut bool,
}

impl CursorGrab for WindowGrab<'_> {
    fn sync(&mut self, ctx: &mut Context) {
        let want = ctx.cursor_grab && ctx.focused;
        if want == ctx.grabbed {
            return;
        }
        if !want {
            release_cursor(self.window);
            ctx.grabbed = false;
            return;
        }
        let grab = self
            .window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined));
        match grab {
            Ok(()) => {
                self.window.set_cursor_visible(false);
                ctx.grabbed = true;
            }
            Err(e) => {
                ctx.cursor_grab = false;
                if !*self.warned {
                    *self.warned = true;
                    log::warn!("cursor grab: {e}");
                }
            }
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

#[expect(
    clippy::cast_possible_truncation,
    reason = "для сида хватает младших 64 бит"
)]
fn clock_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

/// Прогоняет сценарий в окне: `Runner` рисует в свой `Canvas`, `Renderer` выводит его на поверхность.
pub(crate) fn run_script_window<G: Game>(
    config: &Config,
    script: &Script,
    out: &Path,
) -> Result<RunReport, RunError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = ScriptApp::<G> {
        config,
        script,
        out,
        running: None,
        result: None,
        last_frame: Instant::now(),
    };
    event_loop.run_app(&mut app)?;
    app.result.unwrap_or(Err(RunError::Closed { tick: 0 }))
}

struct ScriptApp<'a, G: Game> {
    config: &'a Config,
    script: &'a Script,
    out: &'a Path,
    running: Option<(Arc<Window>, Renderer, Runner<G>)>,
    result: Option<Result<RunReport, RunError>>,
    last_frame: Instant,
}

impl<G: Game> ApplicationHandler for ScriptApp<'_, G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        let (w, h) = self.script.size.unwrap_or(self.config.size);
        let attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(PhysicalSize::new(w, h));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        // Без vsync: под Fifo на Wayland прогон вставал после тика 0 (design.md, «Рендер»).
        let renderer = Renderer::new(window.clone(), false, self.config.pixel_perfect);
        self.last_frame = Instant::now();
        match Runner::<G>::new(renderer.gpu(), self.config, self.script) {
            Ok(runner) => {
                window.request_redraw();
                self.running = Some((window, renderer, runner));
            }
            Err(e) => self.stop(event_loop, Err(e)),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some((window, renderer, runner)) = self.running.as_mut() else {
            return;
        };
        match event {
            WindowEvent::Resized(size) => {
                renderer.resize(size);
                resume_redraw(window, renderer);
            }
            WindowEvent::Occluded(occluded) => {
                renderer.set_occluded(occluded);
                resume_redraw(window, renderer);
            }
            WindowEvent::CloseRequested => {
                let tick = runner.tick();
                self.stop(event_loop, Err(RunError::Closed { tick }));
            }
            WindowEvent::RedrawRequested => match runner.advance(Some(self.out)) {
                Ok(Some(_)) => match renderer.present_canvas(runner.canvas()) {
                    Ok(_) => {
                        self.last_frame = Instant::now();
                        resume_redraw(window, renderer);
                    }
                    Err(e) => {
                        let e = std::io::Error::other(e);
                        self.stop(event_loop, Err(e.into()));
                    }
                },
                Ok(None) => {
                    let (_, _, runner) = self.running.take().expect("running");
                    self.stop(event_loop, Ok(runner.finish()));
                }
                Err(e) => self.stop(event_loop, Err(e)),
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some((_, _, runner)) = self.running.as_ref() else {
            return;
        };
        let idle = self.last_frame.elapsed();
        if stalled(idle) {
            let tick = runner.tick();
            self.stop(event_loop, Err(RunError::Stalled { tick }));
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_frame + STALL_TIMEOUT));
        }
    }
}

impl<G: Game> ScriptApp<'_, G> {
    fn stop(&mut self, event_loop: &ActiveEventLoop, result: Result<RunReport, RunError>) {
        self.result = Some(result);
        event_loop.exit();
    }
}

fn resume_redraw(window: &Window, renderer: &Renderer) {
    if renderer.is_available() {
        window.request_redraw();
    }
}
