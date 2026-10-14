use std::f32::consts::{FRAC_PI_3, FRAC_PI_6};

use engine::{
    Camera2D, Color, Config, Context, Game, Input, KeyCode, Sprite, TextureId, Vec2, vec2,
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
const KEYS: [(KeyCode, KeyCode); 2] = [
    (KeyCode::KeyW, KeyCode::KeyS),
    (KeyCode::ArrowUp, KeyCode::ArrowDown),
];

/// Состояние партии; индекс 0 — левый игрок, 1 — правый.
struct Pong {
    paddles: [f32; 2],
    prev: Vec2,
    ball: Vec2,
    vel: Vec2,
    score: [u32; 2],
    serve_up: bool,
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
            let dir = input.is_down(up) as i32 - input.is_down(down) as i32;
            *y = (*y + dir as f32 * PADDLE_SPEED * DT).clamp(-limit, limit);
        }

        self.prev = self.ball;
        let delta = self.vel * DT;
        let mut next = self.ball + delta;
        for (side, (y0, y1)) in old.into_iter().zip(self.paddles).enumerate() {
            let x = paddle_x(side);
            if self.vel.x * x <= 0.0 {
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
            let missed = usize::from(self.ball.x > 0.0);
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
        texture: TextureId::WHITE,
        pos,
        size,
        color: Color::from_srgb8(255, 255, 255),
        layer: 0,
    }
}

impl Game for Pong {
    fn init(ctx: &mut Context) -> Self {
        ctx.gfx.camera = Camera2D {
            center: Vec2::ZERO,
            size: HALF_FIELD * 2.0,
        };
        Self::new()
    }

    fn update(&mut self, ctx: &mut Context) {
        self.tick(&ctx.input);
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        for (side, &y) in self.paddles.iter().enumerate() {
            ctx.gfx
                .draw_sprite(quad(vec2(paddle_x(side), y), PADDLE_HALF * 2.0));
        }
        let ball = self.prev.lerp(self.ball, alpha);
        ctx.gfx
            .draw_sprite(quad(ball, Vec2::splat(BALL_HALF * 2.0)));
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Pong>(Config {
        title: "pong".into(),
        size: (1280, 720),
        tick_hz: TICK_HZ,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
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
    }

    #[test]
    fn paddle_moves_by_key_and_stays_in_field() {
        let mut p = Pong::new();
        p.tick(&input(&[KeyCode::KeyW, KeyCode::ArrowDown]));
        assert!((p.paddles[0] - 5.0).abs() < EPS && (p.paddles[1] + 5.0).abs() < EPS);

        for _ in 0..100 {
            p.tick(&input(&[KeyCode::KeyW, KeyCode::ArrowDown]));
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
        p.tick(&input(&[KeyCode::ArrowDown]));
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
}
