use std::time::{SystemTime, UNIX_EPOCH};

use engine::{
    Camera2D, Commands, Config, Context, Entity, Game, Input, KeyCode, Sprite, Vec2, World, vec2,
};

const TICK_HZ: u32 = 60;
const DT: f32 = 1.0 / TICK_HZ as f32;
const HALF_WORLD: Vec2 = vec2(320.0, 180.0);
const TURN_SPEED: f32 = 4.0;
const THRUST: f32 = 300.0;
/// Доля скорости, остающаяся через секунду.
const DRAG: f32 = 0.5;
const MAX_SPEED: f32 = 300.0;
const SHIP_RADIUS: f32 = 8.0;
const BULLET_SPEED: f32 = 480.0;
const BULLET_RADIUS: f32 = 2.0;
const BULLET_TTL: u32 = TICK_HZ;
const FIRE_COOLDOWN: u32 = TICK_HZ / 5;

/// Положение и угол на начало и конец тика; угол не нормализуется, иначе lerp через 2π крутит назад.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Transform {
    pos: Vec2,
    prev: Vec2,
    angle: f32,
    prev_angle: f32,
}

impl Transform {
    fn at(pos: Vec2, angle: f32) -> Self {
        Self {
            pos,
            prev: pos,
            angle,
            prev_angle: angle,
        }
    }

    fn lerp(&self, alpha: f32) -> (Vec2, f32) {
        let angle = self.prev_angle + (self.angle - self.prev_angle) * alpha;
        (self.prev.lerp(self.pos, alpha), angle)
    }

    /// Единичный вектор носа; при угле 0 — вверх.
    fn forward(&self) -> Vec2 {
        vec2(-self.angle.sin(), self.angle.cos())
    }
}

struct Velocity(Vec2);

/// Угловая скорость, рад/с против часовой.
struct Spin(f32);

/// Размер спрайта; полудиагональ — радиус переноса.
struct Size(Vec2);

/// Оставшиеся тики.
struct Lifetime(u32);

struct Bullet;

/// xorshift64.
#[allow(dead_code, reason = "астероиды и частицы — следующие задачи")]
struct Rng(u64);

#[allow(dead_code, reason = "астероиды и частицы — следующие задачи")]
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Равномерно в `[lo, hi)`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let unit = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
        lo + unit * (hi - lo)
    }
}

struct Asteroids {
    world: World,
    cmds: Commands,
    #[allow(dead_code, reason = "астероиды и частицы — следующие задачи")]
    rng: Rng,
    ship: Entity,
    cooldown: u32,
}

/// Сдвиг, переносящий объект радиуса `r` на другую сторону, когда он целиком ушёл за край.
fn wrap_shift(pos: Vec2, r: f32) -> Vec2 {
    let edge = HALF_WORLD + r;
    let mut shift = Vec2::ZERO;
    for i in 0..2 {
        if pos[i] > edge[i] {
            shift[i] = -2.0 * edge[i];
        } else if pos[i] < -edge[i] {
            shift[i] = 2.0 * edge[i];
        }
    }
    shift
}

impl Asteroids {
    fn new(seed: u64) -> Self {
        let mut world = World::default();
        world.register::<Transform>();
        world.register::<Velocity>();
        world.register::<Spin>();
        world.register::<Size>();
        world.register::<Lifetime>();
        world.register::<Bullet>();
        let ship = world.spawn();
        world.insert(ship, Transform::at(Vec2::ZERO, 0.0));
        world.insert(ship, Velocity(Vec2::ZERO));
        world.insert(ship, Spin(0.0));
        world.insert(ship, Size(Vec2::splat(SHIP_RADIUS * 2.0)));
        Self {
            world,
            cmds: Commands::default(),
            rng: Rng::new(seed),
            ship,
            cooldown: 0,
        }
    }

    fn tick(&mut self, input: &Input) {
        self.control(input);
        self.integrate();
        self.fire(input);
        self.wrap();
        self.expire();
        self.cmds.apply(&mut self.world);
    }

    fn control(&mut self, input: &Input) {
        let turn = input.is_down(KeyCode::KeyA) as i32 - input.is_down(KeyCode::KeyD) as i32;
        self.world
            .storage_mut::<Spin>()
            .get_mut(self.ship)
            .unwrap()
            .0 = turn as f32 * TURN_SPEED;
        let forward = self
            .world
            .storage::<Transform>()
            .get(self.ship)
            .unwrap()
            .forward();
        let mut vels = self.world.storage_mut::<Velocity>();
        let vel = &mut vels.get_mut(self.ship).unwrap().0;
        if input.is_down(KeyCode::KeyW) {
            *vel += forward * THRUST * DT;
        }
        *vel = (*vel * DRAG.powf(DT)).clamp_length_max(MAX_SPEED);
    }

    fn integrate(&mut self) {
        let mut transforms = self.world.storage_mut::<Transform>();
        let vels = self.world.storage::<Velocity>();
        let spins = self.world.storage::<Spin>();
        for (e, t) in transforms.iter_mut() {
            t.prev = t.pos;
            t.prev_angle = t.angle;
            if let Some(v) = vels.get(e) {
                t.pos += v.0 * DT;
            }
            if let Some(s) = spins.get(e) {
                t.angle += s.0 * DT;
            }
        }
    }

    /// После интегрирования: пуля появляется у носа там, где корабль в конце тика.
    fn fire(&mut self, input: &Input) {
        self.cooldown = self.cooldown.saturating_sub(1);
        if !input.is_down(KeyCode::Space) || self.cooldown > 0 {
            return;
        }
        self.cooldown = FIRE_COOLDOWN;
        let ship = *self.world.storage::<Transform>().get(self.ship).unwrap();
        let ship_vel = self.world.storage::<Velocity>().get(self.ship).unwrap().0;
        let dir = ship.forward();
        let pos = ship.pos + dir * SHIP_RADIUS;
        let vel = dir * BULLET_SPEED + ship_vel;
        self.cmds.spawn(move |w| {
            let e = w.spawn();
            w.insert(e, Transform::at(pos, ship.angle));
            w.insert(e, Velocity(vel));
            w.insert(e, Size(Vec2::splat(BULLET_RADIUS * 2.0)));
            w.insert(e, Lifetime(BULLET_TTL));
            w.insert(e, Bullet);
        });
    }

    fn wrap(&mut self) {
        let mut transforms = self.world.storage_mut::<Transform>();
        let sizes = self.world.storage::<Size>();
        for (e, t) in transforms.iter_mut() {
            let Some(size) = sizes.get(e) else {
                continue;
            };
            let shift = wrap_shift(t.pos, size.0.length() / 2.0);
            t.pos += shift;
            t.prev += shift;
        }
    }

    fn expire(&mut self) {
        for (e, left) in self.world.storage_mut::<Lifetime>().iter_mut() {
            left.0 -= 1;
            if left.0 == 0 {
                self.cmds.despawn(e);
            }
        }
    }
}

impl Game for Asteroids {
    fn init(ctx: &mut Context) -> Self {
        ctx.gfx.camera = Camera2D {
            center: Vec2::ZERO,
            size: HALF_WORLD * 2.0,
        };
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Self::new(seed)
    }

    fn update(&mut self, ctx: &mut Context) {
        self.tick(&ctx.input);
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        let transforms = self.world.storage::<Transform>();
        let sizes = self.world.storage::<Size>();
        for (e, t) in transforms.iter() {
            let Some(size) = sizes.get(e) else {
                continue;
            };
            let (pos, rotation) = t.lerp(alpha);
            ctx.gfx.draw_sprite(Sprite {
                pos,
                size: size.0,
                rotation,
                ..Default::default()
            });
        }
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Asteroids>(Config {
        title: "asteroids".into(),
        size: (1280, 720),
        tick_hz: TICK_HZ,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
    })
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

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

    fn game() -> Asteroids {
        Asteroids::new(1)
    }

    fn ship(g: &Asteroids) -> Transform {
        *g.world.storage::<Transform>().get(g.ship).unwrap()
    }

    fn ship_vel(g: &Asteroids) -> Vec2 {
        g.world.storage::<Velocity>().get(g.ship).unwrap().0
    }

    fn set_ship(g: &mut Asteroids, t: Transform, vel: Vec2) {
        *g.world.storage_mut::<Transform>().get_mut(g.ship).unwrap() = t;
        g.world.storage_mut::<Velocity>().get_mut(g.ship).unwrap().0 = vel;
    }

    fn bullets(g: &Asteroids) -> Vec<(Transform, Vec2)> {
        let transforms = g.world.storage::<Transform>();
        let vels = g.world.storage::<Velocity>();
        g.world
            .storage::<Bullet>()
            .iter()
            .map(|(e, _)| (*transforms.get(e).unwrap(), vels.get(e).unwrap().0))
            .collect()
    }

    #[test]
    fn keys_turn_ship() {
        let mut g = game();
        g.tick(&input(&[KeyCode::KeyA]));
        assert!((ship(&g).angle - TURN_SPEED * DT).abs() < EPS);
        g.tick(&input(&[KeyCode::KeyD]));
        g.tick(&input(&[KeyCode::KeyD]));
        assert!((ship(&g).angle + TURN_SPEED * DT).abs() < EPS);
        g.tick(&input(&[KeyCode::KeyA, KeyCode::KeyD]));
        assert!((ship(&g).angle + TURN_SPEED * DT).abs() < EPS);
    }

    #[test]
    fn thrust_accelerates_along_nose() {
        let mut g = game();
        set_ship(&mut g, Transform::at(Vec2::ZERO, -TAU / 4.0), Vec2::ZERO);
        g.tick(&input(&[KeyCode::KeyW]));
        let v = ship_vel(&g);
        assert!((v.x - THRUST * DT * DRAG.powf(DT)).abs() < EPS && v.y.abs() < EPS);
        assert!((ship(&g).pos - v * DT).length() < EPS);
    }

    #[test]
    fn drag_halves_speed_per_second() {
        let mut g = game();
        set_ship(&mut g, Transform::at(Vec2::ZERO, 0.0), vec2(100.0, 0.0));
        for _ in 0..TICK_HZ {
            g.tick(&Input::default());
        }
        assert!((ship_vel(&g).x - 50.0).abs() < 0.01);
    }

    #[test]
    fn speed_is_limited() {
        let mut g = game();
        for _ in 0..10 * TICK_HZ {
            g.tick(&input(&[KeyCode::KeyW]));
        }
        assert!(ship_vel(&g).length() <= MAX_SPEED + EPS);
        assert!(ship_vel(&g).length() > MAX_SPEED - 1.0);
    }

    #[test]
    fn bullet_adds_ship_velocity_and_starts_without_interpolation() {
        let mut g = game();
        set_ship(&mut g, Transform::at(Vec2::ZERO, 0.0), vec2(100.0, 0.0));
        g.tick(&input(&[KeyCode::Space]));
        let shot = bullets(&g);
        assert_eq!(shot.len(), 1);
        let (t, vel) = shot[0];
        assert!((vel - (ship_vel(&g) + vec2(0.0, BULLET_SPEED))).length() < EPS);
        assert_eq!(t.prev, t.pos);
        assert!((t.pos - (ship(&g).pos + vec2(0.0, SHIP_RADIUS))).length() < EPS);
    }

    #[test]
    fn held_fire_respects_cooldown() {
        let mut g = game();
        let fire = input(&[KeyCode::Space]);
        for _ in 0..2 * FIRE_COOLDOWN {
            g.tick(&fire);
        }
        assert_eq!(bullets(&g).len(), 2);
        g.tick(&fire);
        assert_eq!(bullets(&g).len(), 3);
    }

    #[test]
    fn bullet_expires_after_ttl() {
        let mut g = game();
        g.tick(&input(&[KeyCode::Space]));
        for _ in 0..BULLET_TTL - 1 {
            g.tick(&Input::default());
        }
        assert_eq!(bullets(&g).len(), 1);
        g.tick(&Input::default());
        assert!(bullets(&g).is_empty());
        assert_eq!(g.world.storage::<Transform>().len(), 1);
    }

    #[test]
    fn wrap_waits_until_fully_out_and_shifts_prev() {
        let r = SHIP_RADIUS * 2f32.sqrt();
        let vel = vec2(60.0, 0.0);
        let mut g = game();
        set_ship(
            &mut g,
            Transform::at(vec2(320.0 + r - 1.5, 0.0), 0.0),
            vel / DRAG.powf(DT),
        );
        g.tick(&Input::default());
        assert!((ship(&g).pos.x - (320.0 + r - 0.5)).abs() < EPS);

        g.tick(&Input::default());
        let t = ship(&g);
        let step = vel * DRAG.powf(DT) * DT;
        let x = 320.0 + r - 0.5 + step.x - (640.0 + 2.0 * r);
        assert!((t.pos.x - x).abs() < EPS, "x = {}", t.pos.x);
        assert!((t.pos - t.prev - step).length() < EPS);
    }

    #[test]
    fn particle_wraps_too() {
        let mut g = game();
        let e = g.world.spawn();
        g.world.insert(e, Transform::at(vec2(0.0, -181.0), 0.0));
        g.world.insert(e, Velocity(vec2(0.0, -60.0)));
        g.world.insert(e, Size(Vec2::splat(2.0)));
        g.world.insert(e, Lifetime(TICK_HZ));
        g.tick(&Input::default());
        let t = *g.world.storage::<Transform>().get(e).unwrap();
        let r = 2f32.sqrt();
        assert!((t.pos.y - (-182.0 + 360.0 + 2.0 * r)).abs() < EPS);
        assert!((t.pos - t.prev - vec2(0.0, -1.0)).length() < EPS);
    }

    #[test]
    fn angle_across_tau_interpolates_short_way() {
        let mut g = game();
        let start = TAU - 0.01;
        set_ship(&mut g, Transform::at(Vec2::ZERO, start), Vec2::ZERO);
        g.tick(&input(&[KeyCode::KeyA]));
        let (_, angle) = ship(&g).lerp(0.5);
        assert!((angle - (start + TURN_SPEED * DT / 2.0)).abs() < EPS);
    }

    #[test]
    fn rng_is_seeded_and_in_range() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        assert_eq!(a.next(), b.next());
        assert_ne!(Rng::new(0).next(), 0);
        for _ in 0..1000 {
            let x = a.range(-2.0, 3.0);
            assert!((-2.0..3.0).contains(&x));
        }
    }
}
