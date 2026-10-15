use std::f32::consts::{FRAC_PI_3, FRAC_PI_6};

use engine::{
    Camera2D, Config, Context, Game, Handle, Input, KeyCode, Sprite, Texture, Vec2, vec2,
};

const TICK_HZ: u32 = 60;
const DT: f32 = 1.0 / TICK_HZ as f32;
const HALF_FIELD: Vec2 = vec2(320.0, 180.0);
const PADDLE_X: f32 = 300.0;
const PADDLE_HALF: Vec2 = vec2(4.0, 30.0);
const PADDLE_SPEED: f32 = 300.0;
const BALL_HALF: f32 = 5.0;
const SERVE_SPEED: f32 = 240.0;
const SERVE_ANGLE: f32 = FRAC_PI_6;
const SPEEDUP: f32 = 1.05;
const MAX_SPEED: f32 = 420.0;
const MAX_BOUNCE: f32 = FRAC_PI_3;
const WIN_SCORE: u32 = 10;
const DASH: Vec2 = vec2(4.0, 12.0);
const DASH_STEP: f32 = 24.0;
/// Штрихов средней линии по каждую сторону от центрального.
const DASHES: i8 = 7;
const DIGIT: Vec2 = vec2(20.0, 36.0);
const STROKE: f32 = 4.0;
const SCORE_POS: Vec2 = vec2(60.0, 140.0);
/// Сегменты цифр 0–9: бит 0..6 — a (верх), b, c, d (низ), e, f, g (середина), по часовой от верха.
const SEGMENTS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
const KEYS: [(KeyCode, KeyCode); 2] = [
    (KeyCode::KeyW, KeyCode::KeyS),
    (KeyCode::KeyO, KeyCode::KeyL),
];

/// Состояние партии; индекс 0 — левый игрок, 1 — правый.
struct Pong {
    paddles: [f32; 2],
    prev: Vec2,
    ball: Vec2,
    vel: Vec2,
    score: [u32; 2],
    serve_up: bool,
    ball_tex: Handle<Texture>,
}

fn paddle_x(side: usize) -> f32 {
    if side == 0 { -PADDLE_X } else { PADDLE_X }
}

/// Первое `t ∈ [0, 1]`, при котором `from + delta·t` попадает в AABB с центром в нуле; старт внутри — `0`.
fn sweep(from: Vec2, delta: Vec2, half: Vec2) -> Option<f32> {
    let mut enter = f32::NEG_INFINITY;
    let mut exit = f32::INFINITY;
    for i in 0..2 {
        if delta[i] == 0.0 {
            if from[i].abs() > half[i] {
                return None;
            }
        } else {
            let a = (-half[i] - from[i]) / delta[i];
            let b = (half[i] - from[i]) / delta[i];
            enter = enter.max(a.min(b));
            exit = exit.min(a.max(b));
        }
    }
    (enter <= exit && enter <= 1.0 && exit >= 0.0).then_some(enter.max(0.0))
}

impl Pong {
    fn new() -> Self {
        let mut p = Self {
            paddles: [0.0; 2],
            prev: Vec2::ZERO,
            ball: Vec2::ZERO,
            vel: Vec2::ZERO,
            score: [0; 2],
            serve_up: true,
            ball_tex: Handle::WHITE,
        };
        p.serve(0);
        p
    }

    /// Мяч из центра к стороне `to`.
    fn serve(&mut self, to: usize) {
        let up = if self.serve_up { 1.0 } else { -1.0 };
        self.serve_up = !self.serve_up;
        self.ball = Vec2::ZERO;
        self.prev = self.ball;
        self.vel = vec2(
            paddle_x(to).signum() * SERVE_ANGLE.cos(),
            up * SERVE_ANGLE.sin(),
        ) * SERVE_SPEED;
    }

    fn tick(&mut self, input: &Input) {
        let old = self.paddles;
        let limit = HALF_FIELD.y - PADDLE_HALF.y;
        for (y, (up, down)) in self.paddles.iter_mut().zip(KEYS) {
            let dir = i32::from(input.is_down(up)) - i32::from(input.is_down(down));
            *y = (*y + dir as f32 * PADDLE_SPEED * DT).clamp(-limit, limit);
        }

        self.prev = self.ball;
        let delta = self.vel * DT;
        let mut next = self.ball + delta;
        for (side, (y0, y1)) in old.into_iter().zip(self.paddles).enumerate() {
            let x = paddle_x(side);
            if self.vel.x.is_sign_positive() != x.is_sign_positive() {
                continue;
            }
            // Мяч заметается относительно ракетки: она тоже сдвинулась за тик.
            let moved = y1 - y0;
            let from = self.ball - vec2(x, y0);
            let rel = delta - vec2(0.0, moved);
            let Some(t) = sweep(from, rel, PADDLE_HALF + BALL_HALF) else {
                continue;
            };
            let hit_y = self.ball.y + delta.y * t;
            let offset = (hit_y - (y0 + moved * t)) / PADDLE_HALF.y;
            let angle = offset.clamp(-1.0, 1.0) * MAX_BOUNCE;
            let speed = (self.vel.length() * SPEEDUP).min(MAX_SPEED);
            self.vel = vec2(-x.signum() * angle.cos(), angle.sin()) * speed;
            let face = x - x.signum() * (PADDLE_HALF.x + BALL_HALF);
            next = vec2(face, hit_y) + self.vel * DT * (1.0 - t);
            break;
        }

        let top = HALF_FIELD.y - BALL_HALF;
        if next.y > top {
            next.y = 2.0 * top - next.y;
            self.vel.y = -self.vel.y.abs();
        } else if next.y < -top {
            next.y = -2.0 * top - next.y;
            self.vel.y = self.vel.y.abs();
        }
        self.ball = next;

        if self.ball.x.abs() > HALF_FIELD.x + BALL_HALF {
            let missed = usize::from(self.ball.x.is_sign_positive());
            let scorer = 1 - missed;
            self.score[scorer] += 1;
            if self.score[scorer] == WIN_SCORE {
                self.score = [0; 2];
            }
            self.serve(missed);
        }
    }
}

fn quad(pos: Vec2, size: Vec2) -> Sprite {
    Sprite {
        pos,
        size,
        ..Default::default()
    }
}

/// Прямоугольники `(центр, размер)` горящих сегментов цифры `d`.
fn digit(d: u32, center: Vec2) -> impl Iterator<Item = (Vec2, Vec2)> {
    let (w, h, t) = (DIGIT.x, DIGIT.y, STROKE);
    let (dx, dy) = ((w - t) / 2.0, (h - t) / 2.0);
    let horiz = vec2(w, t);
    let vert = vec2(t, h / 2.0);
    let rects = [
        (vec2(0.0, dy), horiz),
        (vec2(dx, h / 4.0), vert),
        (vec2(dx, -h / 4.0), vert),
        (vec2(0.0, -dy), horiz),
        (vec2(-dx, -h / 4.0), vert),
        (vec2(-dx, h / 4.0), vert),
        (Vec2::ZERO, horiz),
    ];
    let mask = SEGMENTS[d as usize];
    rects
        .into_iter()
        .enumerate()
        .filter(move |&(i, _)| mask & (1 << i) != 0)
        .map(move |(_, (pos, size))| (center + pos, size))
}

impl Game for Pong {
    fn init(ctx: &mut Context) -> Self {
        ctx.gfx.camera = Camera2D {
            center: Vec2::ZERO,
            size: HALF_FIELD * 2.0,
        };
        let ball_tex = ctx
            .assets
            .load_texture(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ball.png"));
        Self {
            ball_tex,
            ..Self::new()
        }
    }

    fn update(&mut self, ctx: &mut Context) {
        self.tick(&ctx.input);
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        for i in -DASHES..=DASHES {
            ctx.gfx
                .draw_sprite(quad(vec2(0.0, f32::from(i) * DASH_STEP), DASH));
        }
        for (side, (&y, &score)) in self.paddles.iter().zip(&self.score).enumerate() {
            let x = paddle_x(side);
            ctx.gfx.draw_sprite(quad(vec2(x, y), PADDLE_HALF * 2.0));
            let center = vec2(x.signum() * SCORE_POS.x, SCORE_POS.y);
            for (pos, size) in digit(score, center) {
                ctx.gfx.draw_sprite(quad(pos, size));
            }
        }
        ctx.gfx.draw_sprite(Sprite {
            texture: self.ball_tex,
            layer: 1,
            ..quad(
                self.prev.lerp(self.ball, alpha),
                Vec2::splat(BALL_HALF * 2.0),
            )
        });
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Pong>(Config {
        title: "pong".into(),
        size: (1280, 720),
        tick_hz: TICK_HZ,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::ElementState::Pressed;

    const EPS: f32 = 1e-3;

    fn input(keys: &[KeyCode]) -> Input {
        let mut i = Input::default();
        for &k in keys {
            i.on_key(k, Pressed, false);
        }
        i
    }

    fn pong(ball: Vec2, vel: Vec2) -> Pong {
        let mut p = Pong::new();
        p.ball = ball;
        p.prev = ball;
        p.vel = vel;
        p
    }

    fn bounce_angle(vel: Vec2) -> f32 {
        vel.y.atan2(vel.x.abs())
    }

    #[test]
    fn segments_table() {
        assert_eq!(SEGMENTS[8].count_ones(), 7);
        assert_eq!(SEGMENTS[1].count_ones(), 2);
        let mut sets = SEGMENTS.to_vec();
        sets.sort_unstable();
        sets.dedup();
        assert_eq!(sets.len(), 10);
        assert_eq!(digit(8, Vec2::ZERO).count(), 7);
    }

    #[test]
    fn sweep_hits_face() {
        let t = sweep(vec2(-10.0, 0.0), vec2(20.0, 0.0), Vec2::splat(5.0));
        assert_eq!(t, Some(0.25));
    }

    #[test]
    fn sweep_misses_beside_and_short() {
        let half = Vec2::splat(5.0);
        assert_eq!(sweep(vec2(-10.0, 6.0), vec2(20.0, 0.0), half), None);
        assert_eq!(sweep(vec2(-10.0, 0.0), vec2(4.0, 0.0), half), None);
        assert_eq!(sweep(vec2(-10.0, 0.0), vec2(-20.0, 0.0), half), None);
    }

    #[test]
    fn sweep_start_inside_is_zero() {
        let t = sweep(vec2(1.0, -2.0), vec2(30.0, 7.0), Vec2::splat(5.0));
        assert_eq!(t, Some(0.0));
    }

    #[test]
    fn sweep_parallel_to_face() {
        let half = Vec2::splat(5.0);
        assert_eq!(sweep(vec2(-10.0, -10.0), vec2(0.0, 20.0), half), None);
        assert_eq!(sweep(vec2(3.0, -10.0), vec2(0.0, 20.0), half), Some(0.25));
        assert_eq!(sweep(vec2(5.0, -10.0), vec2(0.0, 20.0), half), Some(0.25));
    }

    #[test]
    fn paddle_moves_by_key_and_stays_in_field() {
        let mut p = Pong::new();
        p.tick(&input(&[KeyCode::KeyW, KeyCode::KeyL]));
        assert!((p.paddles[0] - 5.0).abs() < EPS && (p.paddles[1] + 5.0).abs() < EPS);

        for _ in 0..100 {
            p.tick(&input(&[KeyCode::KeyW, KeyCode::KeyL]));
        }
        assert_eq!(p.paddles, [150.0, -150.0]);
    }

    #[test]
    fn ball_bounces_off_top_and_bottom() {
        let mut p = pong(vec2(0.0, 174.0), vec2(0.0, 120.0));
        p.tick(&Input::default());
        assert!((p.ball.y - 174.0).abs() < EPS && p.vel.y < 0.0);

        let mut p = pong(vec2(0.0, -174.0), vec2(0.0, -120.0));
        p.tick(&Input::default());
        assert!((p.ball.y + 174.0).abs() < EPS && p.vel.y > 0.0);
    }

    #[test]
    fn hit_reflects_and_speeds_up_5_percent_up_to_limit() {
        let mut p = pong(vec2(290.0, 0.0), vec2(240.0, 0.0));
        p.tick(&Input::default());
        assert!(p.vel.x < 0.0);
        assert!((p.vel.length() - 252.0).abs() < EPS);
        assert!(p.ball.x <= 291.0);

        let mut p = pong(vec2(-290.0, 0.0), vec2(-410.0, 0.0));
        p.tick(&Input::default());
        assert!(p.vel.x > 0.0);
        assert!((p.vel.length() - MAX_SPEED).abs() < EPS);
    }

    #[test]
    fn bounce_angle_depends_on_hit_point() {
        let mut p = pong(vec2(290.0, 0.0), vec2(240.0, 0.0));
        p.tick(&Input::default());
        assert!(bounce_angle(p.vel).abs() < EPS);

        let mut p = pong(vec2(290.0, 30.0), vec2(240.0, 0.0));
        p.tick(&Input::default());
        assert!((bounce_angle(p.vel) - FRAC_PI_3).abs() < EPS);

        let mut p = pong(vec2(290.0, -15.0), vec2(240.0, 0.0));
        p.tick(&Input::default());
        assert!((bounce_angle(p.vel) + FRAC_PI_6).abs() < EPS);
    }

    #[test]
    fn serves_alternate_up_and_down() {
        let mut p = pong(vec2(-324.0, 0.0), vec2(-240.0, 0.0));
        let first = p.vel.y.signum();
        let mut signs = vec![];
        for _ in 0..2 {
            p.ball = vec2(-324.0, 0.0);
            p.vel = vec2(-240.0, 0.0);
            p.tick(&Input::default());
            signs.push(p.vel.y.signum());
        }
        assert_eq!(signs, [-first, first]);
    }

    #[test]
    fn receding_ball_is_not_reflected() {
        let mut p = pong(vec2(294.0, 0.0), vec2(-240.0, 0.0));
        p.tick(&Input::default());
        assert_eq!(p.vel, vec2(-240.0, 0.0));
        assert_eq!(p.ball, vec2(290.0, 0.0));
    }

    #[test]
    fn moving_paddle_catches_ball_by_corner() {
        let mut p = pong(vec2(290.0, 34.0), vec2(MAX_SPEED, 0.0));
        p.tick(&input(&[KeyCode::KeyL]));
        assert!(p.vel.x < 0.0);
    }

    #[test]
    fn paddle_moving_into_ball_is_swept_relatively() {
        let mut p = pong(vec2(290.0, 37.0), vec2(MAX_SPEED, 0.0));
        p.tick(&input(&[KeyCode::KeyO]));
        assert!(p.vel.x < 0.0);
    }

    #[test]
    fn fast_ball_does_not_pass_still_paddle() {
        let mut p = pong(vec2(200.0, 10.0), vec2(MAX_SPEED, 0.0));
        for _ in 0..30 {
            p.tick(&Input::default());
            assert!(p.ball.x <= 291.0 + EPS, "ball at {}", p.ball.x);
        }
        assert!(p.vel.x < 0.0);
    }

    #[test]
    fn goal_scores_and_serves_to_loser() {
        let mut p = pong(vec2(-324.0, 0.0), vec2(-240.0, 0.0));
        p.tick(&Input::default());
        assert_eq!(p.score, [0, 1]);
        assert_eq!(p.ball, Vec2::ZERO);
        assert_eq!(p.prev, p.ball);
        assert!(p.vel.x < 0.0);
        assert!((p.vel.length() - SERVE_SPEED).abs() < EPS);

        let mut p = pong(vec2(324.0, 0.0), vec2(240.0, 0.0));
        p.tick(&Input::default());
        assert_eq!(p.score, [1, 0]);
        assert!(p.vel.x > 0.0);
    }

    #[test]
    fn tenth_goal_resets_match() {
        let mut p = pong(vec2(-324.0, 0.0), vec2(-240.0, 0.0));
        p.score = [3, 9];
        p.tick(&Input::default());
        assert_eq!(p.score, [0, 0]);
    }

    #[test]
    fn digit_segments_are_placed_around_center() {
        let (horiz, vert) = (vec2(20.0, 4.0), vec2(4.0, 18.0));
        let c = vec2(60.0, 140.0);
        let eight: Vec<_> = digit(8, c).collect();
        assert_eq!(
            eight,
            [
                (c + vec2(0.0, 16.0), horiz),
                (c + vec2(8.0, 9.0), vert),
                (c + vec2(8.0, -9.0), vert),
                (c + vec2(0.0, -16.0), horiz),
                (c + vec2(-8.0, -9.0), vert),
                (c + vec2(-8.0, 9.0), vert),
                (c, horiz),
            ]
        );
        let one: Vec<_> = digit(1, c).collect();
        assert_eq!(one, [eight[1], eight[2]]);
    }

    #[test]
    fn serve_flies_at_30_degrees() {
        let p = Pong::new();
        assert!(p.vel.x < 0.0);
        assert!((bounce_angle(p.vel) - FRAC_PI_6).abs() < EPS);
        assert!((p.vel.length() - SERVE_SPEED).abs() < EPS);
    }

    #[test]
    fn hit_on_moving_paddle_uses_contact_point() {
        let mut p = pong(vec2(286.0, 110.0), vec2(360.0, 120.0));
        p.paddles = [0.0, 100.0];
        p.tick(&input(&[KeyCode::KeyL]));
        assert!((p.vel - vec2(-339.125, 209.175)).length() < EPS);
        assert!((p.ball - vec2(290.058, 112.248)).length() < EPS);
    }

    #[test]
    fn ball_touching_wall_is_not_reflected_yet() {
        let mut p = pong(vec2(0.0, 173.0), vec2(0.0, 120.0));
        p.tick(&Input::default());
        assert_eq!((p.ball.y, p.vel.y), (175.0, 120.0));

        let mut p = pong(vec2(0.0, -173.0), vec2(0.0, -120.0));
        p.tick(&Input::default());
        assert_eq!((p.ball.y, p.vel.y), (-175.0, -120.0));
    }

    #[test]
    fn goal_counts_once_ball_fully_left_field() {
        let mut p = pong(vec2(321.0, 100.0), vec2(240.0, 0.0));
        p.tick(&Input::default());
        assert_eq!((p.ball.x, p.score), (325.0, [0, 0]));
        p.tick(&Input::default());
        assert_eq!(p.score, [1, 0]);
    }

    #[test]
    fn update_ticks_with_context_input() {
        let mut ctx = Context::default();
        let mut p = Pong::init(&mut ctx);
        ctx.input = input(&[KeyCode::KeyW]);
        p.update(&mut ctx);
        assert!((p.paddles[0] - 5.0).abs() < EPS);
    }

    #[test]
    fn draw_fills_field_paddles_score_and_ball() {
        let mut ctx = Context::default();
        let mut p = Pong::init(&mut ctx);
        assert_eq!(ctx.gfx.camera.size, vec2(640.0, 360.0));
        p.paddles = [-50.0, 20.0];
        p.score = [1, 7];
        p.prev = vec2(10.0, 0.0);
        p.ball = vec2(20.0, 40.0);
        p.draw(&mut ctx, 0.25);

        let sprites = &ctx.gfx.sprites;
        let at = |size: Vec2| -> Vec<Vec2> {
            sprites
                .iter()
                .filter(|s| s.size == size)
                .map(|s| s.pos)
                .collect()
        };
        let dashes: Vec<_> = (-7..=7_i8)
            .map(|i| vec2(0.0, 24.0 * f32::from(i)))
            .collect();
        assert_eq!(at(vec2(4.0, 12.0)), dashes);
        assert_eq!(
            at(vec2(8.0, 60.0)),
            [vec2(-300.0, -50.0), vec2(300.0, 20.0)]
        );
        assert_eq!(at(vec2(20.0, 4.0)), [vec2(60.0, 156.0)]);
        assert_eq!(
            at(vec2(4.0, 18.0)),
            [
                vec2(-52.0, 149.0),
                vec2(-52.0, 131.0),
                vec2(68.0, 149.0),
                vec2(68.0, 131.0),
            ]
        );

        let (ball, rest) = sprites.split_last().unwrap();
        assert_eq!(rest.len(), 15 + 2 + 5);
        assert!(rest.iter().all(|s| *s == quad(s.pos, s.size)));
        let ball_png = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ball.png");
        assert_ne!(ball.texture, Handle::WHITE);
        assert_eq!(
            *ball,
            Sprite {
                texture: ctx.assets.load_texture(ball_png),
                pos: vec2(12.5, 10.0),
                size: vec2(10.0, 10.0),
                layer: 1,
                ..Sprite::default()
            }
        );
    }
}
