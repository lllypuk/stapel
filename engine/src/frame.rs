use std::time::Duration;

use crate::{
    Assets, Audio, Camera2D, Camera3D, FixedClock, Font, Handle, Input, InputEvent, Ray, Scene,
    Sprite, UVec2, Vec2, Viewport, screen_to_world, text,
};

/// Линейный RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Красный.
    pub r: f64,
    /// Зелёный.
    pub g: f64,
    /// Синий.
    pub b: f64,
    /// Непрозрачность.
    pub a: f64,
}

impl Color {
    /// Непрозрачный чёрный.
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    /// Непрозрачный белый.
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    /// Непрозрачный цвет из 8-битных sRGB-компонент.
    pub fn from_srgb8(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: srgb_to_linear(r),
            g: srgb_to_linear(g),
            b: srgb_to_linear(b),
            a: 1.0,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

pub(crate) fn srgb_to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn linear_to_srgb(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "c зажат в [0, 1], значит s * 255 — в [0, 255]"
    )]
    let byte = (s * 255.0).round() as u8;
    byte
}

/// Что нарисовать в этом кадре; игра заполняет в `draw`.
#[derive(Default)]
pub struct Gfx {
    /// Цвет заливки кадра.
    pub clear_color: Color,
    /// Камера, через которую рисуются спрайты.
    pub camera: Camera2D,
    /// Спрайты кадра; очищается перед `draw`.
    pub sprites: Vec<Sprite>,
    /// 3D под спрайтами; `None` перед каждым `draw`.
    pub scene: Option<Scene>,
}

impl Gfx {
    /// Добавляет спрайт в кадр.
    pub fn draw_sprite(&mut self, sprite: Sprite) {
        self.sprites.push(sprite);
    }

    /// `pos` — начало baseline, Y вверх; `scale` — мировых единиц на пиксель растеризации.
    #[expect(
        clippy::too_many_arguments,
        reason = "текст, позиция и стиль — независимые параметры"
    )]
    pub fn draw_text(
        &mut self,
        assets: &Assets,
        font: Handle<Font>,
        text: &str,
        pos: Vec2,
        scale: f32,
        color: Color,
        layer: i32,
    ) {
        let font = assets.font(font);
        self.sprites
            .extend(text::layout(font, text, pos, scale, color, layer));
    }
}

#[derive(Default)]
/// Время симуляции и замеры частоты.
pub struct Time {
    /// Длина тика.
    pub step: Duration,
    /// Тиков с запуска.
    pub ticks: u64,
    /// Кадров в секунду за последнее окно замера.
    pub fps: u32,
    /// Тиков в секунду за последнее окно замера.
    pub tps: u32,
}

/// Камера и вьюпорт последнего показанного кадра.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    /// Камера, через которую нарисован кадр.
    pub camera: Camera2D,
    /// Куда на поверхности лёг кадр, в физических пикселях.
    pub viewport: Viewport,
    /// 3D-часть кадра; `None` — кадр без сцены.
    pub scene: Option<ShownScene>,
}

/// 3D-часть показанного кадра.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownScene {
    /// Камера сцены.
    pub camera: Camera3D,
    /// Размер поверхности в физических пикселях: 3D рисуется на всю.
    pub size: UVec2,
}

#[derive(Default)]
/// Всё, что движок даёт игре.
pub struct Context {
    /// Клавиатура текущего тика.
    pub input: Input,
    /// Время и частоты.
    pub time: Time,
    /// Что нарисовать в этом кадре.
    pub gfx: Gfx,
    /// Текстуры, шрифты и звуки.
    pub assets: Assets,
    /// Запуск звуков; немой, пока нет устройства.
    pub audio: Audio,
    /// Последний показанный кадр; `None` до первого показа и после resize.
    pub shown: Option<Shown>,
    /// Окно в фокусе.
    pub focused: bool,
    /// Игра просит захватить курсор; app сбрасывает при отказе системы и потере фокуса.
    pub cursor_grab: bool,
    /// Курсор захвачен и спрятан; ставит app.
    pub grabbed: bool,
}

impl Context {
    /// Контекст с тиком длиной `step`.
    pub fn new(step: Duration) -> Self {
        Self {
            time: Time {
                step,
                ..Time::default()
            },
            ..Self::default()
        }
    }

    /// Курсор в мире по показанному кадру, а не по `gfx.camera`: игра видит то, что на экране.
    pub fn cursor_world(&self) -> Option<Vec2> {
        let shown = self.shown?;
        screen_to_world(shown.viewport, &shown.camera, self.input.cursor()?)
    }

    /// Луч из курсора по 3D-камере показанного кадра и всей поверхности, не по вьюпорту спрайтов.
    pub fn cursor_ray(&self) -> Option<Ray> {
        let scene = self.shown?.scene?;
        let cursor = self.input.cursor()?;
        if scene.size.min_element() == 0 {
            return None;
        }
        let size = scene.size.as_vec2();
        let ndc = Vec2::new(cursor.x / size.x * 2.0 - 1.0, 1.0 - cursor.y / size.y * 2.0);
        Some(scene.camera.ray(ndc, size.x / size.y))
    }

    /// Применяет событие ввода: `Input`, сдвиг по захвату или фокус.
    pub fn apply(&mut self, event: InputEvent) {
        match event {
            InputEvent::Key {
                code,
                state,
                repeat,
            } => self.input.on_key(code, state, repeat),
            InputEvent::Button { button, state } => self.input.on_button(button, state),
            InputEvent::Cursor(pos) => self.input.on_cursor(pos),
            InputEvent::Motion(delta) => self.on_mouse_motion(delta),
            InputEvent::Wheel(lines) => self.input.on_wheel(lines),
            InputEvent::Focus(focused) => self.on_focus(focused),
        }
    }

    /// Без захвата сдвиг — движение курсора по другим окнам и панелям, не обзор.
    pub(crate) fn on_mouse_motion(&mut self, delta: Vec2) {
        if self.focused && self.grabbed {
            self.input.on_mouse_motion(delta);
        }
    }

    /// Потерянный фокус снимает и захват: Wayland держит его и за пределами окна.
    pub(crate) fn on_focus(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.input.focus_lost();
            self.cursor_grab = false;
            self.grabbed = false;
        }
    }
}

/// Игра, которую крутит `run`.
pub trait Game {
    /// Создаёт игру и грузит её ресурсы.
    fn init(ctx: &mut Context) -> Self;
    /// Один фиксированный тик симуляции.
    fn update(&mut self, ctx: &mut Context);
    /// `alpha` — доля шага после последнего тика, для интерполяции.
    fn draw(&mut self, ctx: &mut Context, alpha: f32);
    /// Панели egui, раз в кадр после `draw`.
    fn ui(&mut self, _ui: &egui::Context) {}
}

/// Возвращает число выполненных тиков.
pub fn step_frame<G: Game>(
    game: &mut G,
    ctx: &mut Context,
    clock: &mut FixedClock,
    frame_dt: Duration,
) -> u32 {
    let ticks = clock.advance(frame_dt);
    for _ in 0..ticks {
        game.update(ctx);
        ctx.input.end_tick();
        ctx.time.ticks += 1;
    }
    // Здесь, а не после рендера: на кадрах без рендера спрайты копились бы.
    ctx.gfx.sprites.clear();
    ctx.gfx.scene = None;
    game.draw(ctx, clock.alpha());
    ticks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyCode, Vec2, Vec3};
    use winit::event::ElementState::Pressed;
    use winit::event::MouseButton;

    const K: KeyCode = KeyCode::Space;

    #[derive(Default)]
    struct Fake {
        updates: u32,
        draws: u32,
        pressed_in: Vec<u32>,
        deltas: Vec<Vec2>,
        draw_delta: Vec2,
    }

    impl Game for Fake {
        fn init(_: &mut Context) -> Self {
            Self::default()
        }

        fn update(&mut self, ctx: &mut Context) {
            self.updates += 1;
            if ctx.input.just_pressed(K) {
                self.pressed_in.push(self.updates);
            }
            self.deltas.push(ctx.input.mouse_delta());
        }

        fn draw(&mut self, ctx: &mut Context, _: f32) {
            self.draws += 1;
            self.draw_delta = ctx.input.mouse_delta();
        }
    }

    fn setup() -> (Fake, Context, FixedClock, Duration) {
        let clock = FixedClock::new(60, Duration::from_millis(250));
        let mut ctx = Context::new(clock.step());
        let step = clock.step();
        (Fake::init(&mut ctx), ctx, clock, step)
    }

    #[test]
    fn frame_runs_ticks_then_one_draw() {
        let (mut g, mut ctx, mut clock, step) = setup();
        let ticks = step_frame(&mut g, &mut ctx, &mut clock, step * 2);
        assert_eq!((ticks, g.updates, g.draws), (2, 2, 1));
    }

    #[test]
    fn press_is_seen_only_by_first_tick() {
        let (mut g, mut ctx, mut clock, step) = setup();
        ctx.input.on_key(K, Pressed, false);
        step_frame(&mut g, &mut ctx, &mut clock, step * 2);
        assert_eq!(g.pressed_in, [1]);
    }

    #[test]
    fn press_survives_frame_without_ticks() {
        let (mut g, mut ctx, mut clock, step) = setup();
        ctx.input.on_key(K, Pressed, false);
        step_frame(&mut g, &mut ctx, &mut clock, step / 2);
        assert_eq!(g.pressed_in, [0; 0]);
        step_frame(
            &mut g,
            &mut ctx,
            &mut clock,
            step.checked_sub(step / 2).unwrap(),
        );
        assert_eq!(g.pressed_in, [1]);
    }

    #[test]
    fn ticks_counter_grows_by_ticks_run() {
        let (mut g, mut ctx, mut clock, step) = setup();
        step_frame(&mut g, &mut ctx, &mut clock, step * 3);
        step_frame(&mut g, &mut ctx, &mut clock, step / 2);
        assert_eq!(ctx.time.ticks, 3);
        step_frame(&mut g, &mut ctx, &mut clock, step);
        assert_eq!(ctx.time.ticks, 4);
    }

    #[test]
    fn previous_sprites_vanish_even_without_ticks() {
        let (mut g, mut ctx, mut clock, step) = setup();
        let s = Sprite {
            color: Color::BLACK,
            ..Default::default()
        };
        ctx.gfx.draw_sprite(s);
        ctx.gfx.draw_sprite(s);
        let ticks = step_frame(&mut g, &mut ctx, &mut clock, step / 2);
        assert_eq!(ticks, 0);
        assert_eq!(ctx.gfx.sprites.len(), 0);
    }

    #[test]
    fn previous_scene_vanishes_even_without_ticks() {
        let (mut g, mut ctx, mut clock, step) = setup();
        ctx.gfx.scene = Some(Scene::default());
        let ticks = step_frame(&mut g, &mut ctx, &mut clock, step / 2);
        assert_eq!(ticks, 0);
        assert_eq!(ctx.gfx.scene, None);
    }

    #[test]
    fn clear_color_and_camera_survive_frame() {
        let (mut g, mut ctx, mut clock, step) = setup();
        let color = Color::from_srgb8(10, 20, 30);
        let camera = Camera2D {
            center: Vec2::new(1.0, 2.0),
            size: Vec2::new(640.0, 360.0),
        };
        ctx.gfx.clear_color = color;
        ctx.gfx.camera = camera;
        step_frame(&mut g, &mut ctx, &mut clock, step);
        assert_eq!((ctx.gfx.clear_color, ctx.gfx.camera), (color, camera));
    }

    #[test]
    fn srgb8_converts_to_linear() {
        let c = Color::from_srgb8(0, 255, 128);
        assert_eq!((c.r, c.g, c.a), (0.0, 1.0, 1.0));
        assert!((c.b - 0.216).abs() < 1e-3, "b = {}", c.b);
    }

    #[test]
    fn srgb8_roundtrips_through_linear() {
        for c in 0..=255 {
            assert_eq!(linear_to_srgb(srgb_to_linear(c)), c);
        }
    }

    #[test]
    fn draw_text_adds_layout_sprites() {
        let mut assets = Assets::default();
        let font = assets.load_font("missing.ttf", 10.0);
        let mut gfx = Gfx::default();
        let pos = Vec2::new(1.0, 2.0);
        gfx.draw_text(&assets, font, "ab", pos, 2.0, Color::BLACK, 4);
        let expected: Vec<_> =
            text::layout(assets.font(font), "ab", pos, 2.0, Color::BLACK, 4).collect();
        assert_eq!(expected.len(), 2);
        assert_eq!(gfx.sprites, expected);
    }

    fn shown_ctx() -> Context {
        Context {
            shown: Some(Shown {
                camera: Camera2D {
                    center: Vec2::new(10.0, 20.0),
                    size: Vec2::new(100.0, 50.0),
                },
                viewport: Viewport {
                    x: 0.0,
                    y: 0.0,
                    w: 200.0,
                    h: 100.0,
                },
                scene: None,
            }),
            ..Context::default()
        }
    }

    #[test]
    fn cursor_world_uses_shown_camera_not_gfx() {
        let mut ctx = shown_ctx();
        ctx.gfx.camera.center = Vec2::new(-500.0, -500.0);
        ctx.input.on_cursor(Some(Vec2::new(100.0, 50.0)));
        assert_eq!(ctx.cursor_world(), Some(Vec2::new(10.0, 20.0)));
    }

    #[test]
    fn cursor_world_needs_shown_frame_and_cursor() {
        let mut ctx = shown_ctx();
        assert_eq!(ctx.cursor_world(), None);
        ctx.input.on_cursor(Some(Vec2::new(100.0, 50.0)));
        ctx.shown = None;
        assert_eq!(ctx.cursor_world(), None);
    }

    /// Сцена на поверхности 400×100 при вьюпорте спрайтов 200×100.
    fn scene_ctx() -> (Context, Camera3D) {
        let camera = Camera3D {
            pos: Vec3::new(1.0, 2.0, 3.0),
            yaw: 0.4,
            pitch: -0.2,
            ..Camera3D::default()
        };
        let mut ctx = shown_ctx();
        if let Some(shown) = &mut ctx.shown {
            shown.scene = Some(ShownScene {
                camera,
                size: UVec2::new(400, 100),
            });
        }
        (ctx, camera)
    }

    #[test]
    fn cursor_ray_uses_shown_camera_and_full_surface() {
        let (mut ctx, camera) = scene_ctx();
        ctx.gfx.scene = Some(Scene {
            camera: Camera3D::default(),
            ..Scene::default()
        });
        for (cursor, ndc) in [
            (Vec2::new(200.0, 50.0), Vec2::ZERO),
            (Vec2::new(0.0, 0.0), Vec2::new(-1.0, 1.0)),
            (Vec2::new(300.0, 100.0), Vec2::new(0.5, -1.0)),
        ] {
            ctx.input.on_cursor(Some(cursor));
            assert_eq!(ctx.cursor_ray(), Some(camera.ray(ndc, 4.0)), "{cursor}");
        }
    }

    #[test]
    fn cursor_ray_top_of_screen_looks_up() {
        let (mut ctx, camera) = scene_ctx();
        ctx.input.on_cursor(Some(Vec2::new(200.0, 0.0)));
        let ray = ctx.cursor_ray().expect("луч");
        assert!(ray.dir.y > camera.forward().y, "{ray:?}");
    }

    #[test]
    fn cursor_ray_needs_shown_scene_cursor_and_size() {
        let (mut ctx, _) = scene_ctx();
        assert_eq!(ctx.cursor_ray(), None);
        ctx.input.on_cursor(Some(Vec2::new(100.0, 50.0)));
        assert!(ctx.cursor_ray().is_some());
        for size in [UVec2::new(0, 100), UVec2::new(400, 0)] {
            let mut zero = scene_ctx().0;
            zero.input.on_cursor(Some(Vec2::new(100.0, 50.0)));
            if let Some(shown) = &mut zero.shown {
                shown.scene.as_mut().expect("сцена").size = size;
            }
            assert_eq!(zero.cursor_ray(), None, "{size}");
        }
        if let Some(shown) = &mut ctx.shown {
            shown.scene = None;
        }
        assert_eq!(ctx.cursor_ray(), None);
        ctx.shown = None;
        assert_eq!(ctx.cursor_ray(), None);
    }

    const D: Vec2 = Vec2::new(3.0, -2.0);

    fn grabbed_ctx() -> Context {
        Context {
            focused: true,
            cursor_grab: true,
            grabbed: true,
            ..Context::default()
        }
    }

    #[test]
    fn mouse_delta_goes_to_first_tick_or_stays_for_draw() {
        let (mut g, mut ctx, mut clock, step) = setup();
        ctx.input.on_mouse_motion(D);
        step_frame(&mut g, &mut ctx, &mut clock, step / 2);
        assert_eq!((g.deltas.len(), g.draw_delta), (0, D));
        step_frame(&mut g, &mut ctx, &mut clock, step * 2);
        assert_eq!((g.deltas, g.draw_delta), (vec![D, Vec2::ZERO], Vec2::ZERO));
    }

    #[test]
    fn mouse_motion_counts_only_when_focused_and_grabbed() {
        for (focused, grabbed, expected) in [
            (true, true, D),
            (false, true, Vec2::ZERO),
            (true, false, Vec2::ZERO),
        ] {
            let mut ctx = Context {
                focused,
                grabbed,
                ..Context::default()
            };
            ctx.on_mouse_motion(D);
            assert_eq!(ctx.input.mouse_delta(), expected, "{focused} {grabbed}");
        }
    }

    #[test]
    fn focus_lost_releases_grab_and_input() {
        let mut ctx = grabbed_ctx();
        ctx.input.on_key(K, Pressed, false);
        ctx.on_mouse_motion(D);
        ctx.on_focus(false);
        assert!(!ctx.focused && !ctx.cursor_grab && !ctx.grabbed);
        assert!(!ctx.input.is_down(K));
        assert_eq!(ctx.input.mouse_delta(), Vec2::ZERO);
    }

    #[test]
    fn apply_routes_events_to_input_and_focus() {
        let mut ctx = grabbed_ctx();
        ctx.apply(InputEvent::Key {
            code: K,
            state: Pressed,
            repeat: false,
        });
        ctx.apply(InputEvent::Button {
            button: MouseButton::Left,
            state: Pressed,
        });
        ctx.apply(InputEvent::Cursor(Some(D)));
        ctx.apply(InputEvent::Motion(D));
        ctx.apply(InputEvent::Wheel(D));
        assert!(ctx.input.is_down(K) && ctx.input.mouse_down(MouseButton::Left));
        assert_eq!(ctx.input.cursor(), Some(D));
        assert_eq!(ctx.input.mouse_delta(), D);
        assert_eq!(ctx.input.wheel(), D);
        ctx.apply(InputEvent::Cursor(None));
        assert_eq!(ctx.input.cursor(), None);
        ctx.apply(InputEvent::Focus(false));
        assert!(!ctx.focused && !ctx.input.is_down(K));
    }

    #[test]
    fn focus_gained_keeps_grab_request() {
        let mut ctx = grabbed_ctx();
        ctx.input.on_key(K, Pressed, false);
        ctx.on_focus(true);
        assert!(ctx.focused && ctx.cursor_grab && ctx.grabbed && ctx.input.is_down(K));
    }
}
