use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

use crate::{Config, Context, FixedClock, Game, RateCounter, Renderer, step_frame};

/// Ниже 4 FPS симуляция замедляется, а не догоняет лавиной тиков.
const MAX_FRAME: Duration = Duration::from_millis(250);

/// Склейка событий `winit` с `step_frame` и `Renderer`.
pub(crate) struct App<G> {
    config: Config,
    running: Option<Running<G>>,
}

struct Running<G> {
    window: Arc<Window>,
    renderer: Renderer,
    game: G,
    ctx: Context,
    clock: FixedClock,
    last_frame: Instant,
    fps: RateCounter,
    tps: RateCounter,
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
        let renderer = Renderer::new(window.clone(), self.config.vsync);
        let clock = FixedClock::new(self.config.tick_hz, MAX_FRAME);
        let mut ctx = Context::new(clock.step());
        let game = G::init(&mut ctx);
        window.request_redraw();
        self.running = Some(Running {
            window,
            renderer,
            game,
            ctx,
            clock,
            last_frame: Instant::now(),
            fps: RateCounter::new(),
            tps: RateCounter::new(),
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(r) = self.running.as_mut() else {
            return;
        };
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
            WindowEvent::Focused(false) => r.ctx.input.focus_lost(),
            WindowEvent::Resized(size) => {
                r.renderer.resize(size);
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
}

impl<G: Game> Running<G> {
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;

        let ticks = step_frame(&mut self.game, &mut self.ctx, &mut self.clock, dt);
        for _ in 0..ticks {
            self.tps.record(now);
        }
        match self.renderer.render(&self.ctx.gfx, &mut self.ctx.assets) {
            Ok(true) => self.fps.record(now),
            Ok(false) => {}
            Err(e) => {
                log::error!("{e}");
                event_loop.exit();
            }
        }
        if let Some(fps) = self.fps.per_second() {
            self.ctx.time.fps = fps;
        }
        if let Some(tps) = self.tps.per_second() {
            self.ctx.time.tps = tps;
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
