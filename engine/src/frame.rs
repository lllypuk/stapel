use std::time::Duration;

use crate::{Assets, Camera2D, FixedClock, Font, Handle, Input, Sprite, Vec2, text};

/// Линейный RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
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
    let s = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

/// Что нарисовать в этом кадре; игра заполняет в `draw`.
#[derive(Default)]
pub struct Gfx {
    pub clear_color: Color,
    pub camera: Camera2D,
    pub sprites: Vec<Sprite>,
}

impl Gfx {
    pub fn draw_sprite(&mut self, sprite: Sprite) {
        self.sprites.push(sprite);
    }

    /// `pos` — начало baseline, Y вверх; `scale` — мировых единиц на пиксель растеризации.
    #[allow(clippy::too_many_arguments)]
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
pub struct Time {
    pub step: Duration,
    pub ticks: u64,
    pub fps: u32,
    pub tps: u32,
}

#[derive(Default)]
pub struct Context {
    pub input: Input,
    pub time: Time,
    pub gfx: Gfx,
    pub assets: Assets,
}

impl Context {
    pub fn new(step: Duration) -> Self {
        Self {
            time: Time {
                step,
                ..Time::default()
            },
            ..Self::default()
        }
    }
}

pub trait Game {
    fn init(ctx: &mut Context) -> Self;
    fn update(&mut self, ctx: &mut Context);
    /// `alpha` — доля шага после последнего тика, для интерполяции.
    fn draw(&mut self, ctx: &mut Context, alpha: f32);
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
    game.draw(ctx, clock.alpha());
    ticks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyCode, Vec2};
    use winit::event::ElementState::Pressed;

    const K: KeyCode = KeyCode::Space;

    #[derive(Default)]
    struct Fake {
        updates: u32,
        draws: u32,
        pressed_in: Vec<u32>,
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
        }

        fn draw(&mut self, _: &mut Context, _: f32) {
            self.draws += 1;
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
        assert!(g.pressed_in.is_empty());
        step_frame(&mut g, &mut ctx, &mut clock, step - step / 2);
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
        assert!(ctx.gfx.sprites.is_empty());
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
}
