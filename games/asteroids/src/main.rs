use std::f32::consts::TAU;
use std::time::{SystemTime, UNIX_EPOCH};

use stapel::{
    Camera2D, Color, Commands, Config, Context, Entity, Font, Game, Gfx, Handle, Input, KeyCode,
    Sprite, Texture, Vec2, World, vec2,
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
const ASTEROID_RADIUS: [f32; 3] = [32.0, 16.0, 8.0];
const ASTEROID_SPEED: [(f32, f32); 3] = [(40.0, 80.0), (60.0, 110.0), (90.0, 150.0)];
const ASTEROID_SCORE: [u32; 3] = [20, 50, 100];
const ASTEROID_MAX_SPIN: f32 = 1.5;
const WAVE_START: u32 = 4;
const WAVE_MAX: u32 = 11;
const WAVE_CLEARANCE: f32 = 120.0;
const LIVES: u32 = 3;
const RESPAWN_DELAY: u32 = 2 * TICK_HZ;
const INVULNERABLE: u32 = 2 * TICK_HZ;
const PARTICLE_SIZE: f32 = 2.0;
const PARTICLE_SPEED: (f32, f32) = (20.0, 150.0);
const ASTEROID_PARTICLES: [u32; 3] = [300, 150, 75];
/// Секунды.
const ASTEROID_PARTICLE_TTL: (f32, f32) = (0.5, 2.0);
const SHIP_PARTICLES: u32 = 6_000;
/// Секунды; короче — и после смерти корабля > 5 000 частиц держится меньше окна статистики.
const SHIP_PARTICLE_TTL: (f32, f32) = (3.0, 4.0);
/// Полупериод мигания неуязвимого корабля, тики.
const BLINK: u32 = TICK_HZ / 10;
const PARTICLE_LAYER: i32 = 0;
const ASTEROID_LAYER: i32 = 1;
const BULLET_LAYER: i32 = 2;
const SHIP_LAYER: i32 = 3;
const HUD_LAYER: i32 = 4;
const FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Regular.ttf"
);
/// При `HUD_SCALE` и окне 1280×720 тексель атласа — пиксель экрана.
const HUD_PX: f32 = 32.0;
const HUD_SCALE: f32 = 0.5;
const HUD_MARGIN: f32 = 6.0;
const SHIP: &str = "корабль не удаляется из мира";

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

impl Size {
    /// Квадрат, описанный вокруг круга радиуса `r`.
    fn circle(r: f32) -> Self {
        Self(Vec2::splat(2.0 * r))
    }
}

/// Оставшиеся тики.
struct Lifetime(u32);

struct Bullet;

/// Полное время жизни в тиках; остаток — в `Lifetime`.
struct Particle {
    ttl: u32,
}

impl Particle {
    fn alpha(&self, left: &Lifetime) -> f32 {
        left.0 as f32 / self.ttl as f32
    }
}

/// Индекс размера в `ASTEROID_*`: 0 — большой.
struct Asteroid(usize);

#[derive(Clone, Copy, Debug, PartialEq)]
enum ShipState {
    Alive,
    Dead { respawn_in: u32 },
    GameOver,
}

/// xorshift64.
struct Rng(u64);

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

/// Текстуры сущностей; `WHITE` до `init`.
struct Art {
    ship: Handle<Texture>,
    asteroid: Handle<Texture>,
    bullet: Handle<Texture>,
    particle: Handle<Texture>,
}

impl Default for Art {
    fn default() -> Self {
        Self {
            ship: Handle::WHITE,
            asteroid: Handle::WHITE,
            bullet: Handle::WHITE,
            particle: Handle::WHITE,
        }
    }
}

struct Hud {
    font: Handle<Font>,
    big: Handle<Font>,
}

struct Asteroids {
    world: World,
    art: Art,
    hud: Option<Hud>,
    cmds: Commands,
    rng: Rng,
    ship: Entity,
    state: ShipState,
    cooldown: u32,
    invulnerable: u32,
    score: u32,
    lives: u32,
    wave: u32,
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

/// Ближайшее `t ∈ [0, 1]`, при котором точка на отрезке `from → to` входит в круг радиуса `r` с центром в нуле.
#[expect(
    clippy::many_single_char_names,
    reason = "a, b, c — коэффициенты квадратного уравнения"
)]
fn sweep_circle(from: Vec2, to: Vec2, r: f32) -> Option<f32> {
    let c = from.length_squared() - r * r;
    if c <= 0.0 {
        return Some(0.0);
    }
    let d = to - from;
    let a = d.length_squared();
    if a == 0.0 {
        return None;
    }
    let b = from.dot(d);
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    (0.0..=1.0).contains(&t).then_some(t)
}

fn spawn_bullet(w: &mut World, pos: Vec2, vel: Vec2, angle: f32) -> Entity {
    let e = w.spawn();
    w.insert(e, Transform::at(pos, angle));
    w.insert(e, Velocity(vel));
    w.insert(e, Size::circle(BULLET_RADIUS));
    w.insert(e, Lifetime(BULLET_TTL));
    w.insert(e, Bullet);
    e
}

fn spawn_asteroid(w: &mut World, tier: usize, pos: Vec2, (vel, spin): (Vec2, f32)) -> Entity {
    let e = w.spawn();
    w.insert(e, Transform::at(pos, 0.0));
    w.insert(e, Velocity(vel));
    w.insert(e, Spin(spin));
    w.insert(e, Size::circle(ASTEROID_RADIUS[tier]));
    w.insert(e, Asteroid(tier));
    e
}

/// Взрыв одной командой; свой ГПСЧ из `seed`, иначе взрывы одного тика совпали бы.
fn explosion(seed: u64, pos: Vec2, count: u32, (lo, hi): (f32, f32)) -> impl FnOnce(&mut World) {
    move |w| {
        let mut rng = Rng::new(seed);
        for _ in 0..count {
            let vel = Vec2::from_angle(rng.range(0.0, TAU))
                * rng.range(PARTICLE_SPEED.0, PARTICLE_SPEED.1);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "lo и hi — положительные секунды, тиков меньше u32::MAX"
            )]
            let ttl = ((rng.range(lo, hi) * TICK_HZ as f32) as u32).max(1);
            let e = w.spawn();
            w.insert(e, Transform::at(pos, 0.0));
            w.insert(e, Velocity(vel));
            w.insert(e, Size(Vec2::splat(PARTICLE_SIZE)));
            w.insert(e, Lifetime(ttl));
            w.insert(e, Particle { ttl });
        }
    }
}

/// Случайные скорость и вращение астероида размера `tier`.
fn asteroid_motion(rng: &mut Rng, tier: usize) -> (Vec2, f32) {
    let (lo, hi) = ASTEROID_SPEED[tier];
    let dir = Vec2::from_angle(rng.range(0.0, TAU));
    let spin = rng.range(-ASTEROID_MAX_SPIN, ASTEROID_MAX_SPIN);
    (dir * rng.range(lo, hi), spin)
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
        world.register::<Asteroid>();
        world.register::<Particle>();
        let ship = world.spawn();
        world.insert(ship, Transform::at(Vec2::ZERO, 0.0));
        world.insert(ship, Velocity(Vec2::ZERO));
        world.insert(ship, Spin(0.0));
        world.insert(ship, Size::circle(SHIP_RADIUS));
        Self {
            world,
            art: Art::default(),
            hud: None,
            cmds: Commands::default(),
            rng: Rng::new(seed),
            ship,
            state: ShipState::Alive,
            cooldown: 0,
            invulnerable: 0,
            score: 0,
            lives: LIVES,
            wave: 0,
        }
    }

    fn tick(&mut self, input: &Input) {
        if self.state == ShipState::GameOver && input.just_pressed(KeyCode::Enter) {
            self.restart();
        }
        let alive = self.state == ShipState::Alive;
        if alive {
            self.control(input);
        }
        self.integrate();
        if alive {
            self.fire(input);
        }
        self.collide();
        self.wrap();
        self.expire();
        self.ship_timers();
        self.cmds.apply(&mut self.world);
        if self.wave > 0 && self.world.storage::<Asteroid>().is_empty() {
            self.next_wave();
        }
    }

    fn restart(&mut self) {
        let doomed: Vec<Entity> = (self.world.storage::<Asteroid>().iter().map(|(e, _)| e))
            .chain(self.world.storage::<Bullet>().iter().map(|(e, _)| e))
            .collect();
        for e in doomed {
            self.world.despawn(e);
        }
        self.score = 0;
        self.lives = LIVES;
        self.wave = 0;
        self.respawn();
        self.next_wave();
    }

    /// Телепорт в центр: `prev = pos`.
    fn respawn(&mut self) {
        *self
            .world
            .storage_mut::<Transform>()
            .get_mut(self.ship)
            .expect(SHIP) = Transform::at(Vec2::ZERO, 0.0);
        self.world
            .storage_mut::<Velocity>()
            .get_mut(self.ship)
            .expect(SHIP)
            .0 = Vec2::ZERO;
        self.state = ShipState::Alive;
        self.invulnerable = INVULNERABLE;
    }

    fn next_wave(&mut self) {
        self.wave += 1;
        let count = (WAVE_START + self.wave - 1).min(WAVE_MAX);
        let ship = match self.state {
            ShipState::Alive => {
                self.world
                    .storage::<Transform>()
                    .get(self.ship)
                    .expect(SHIP)
                    .pos
            }
            _ => Vec2::ZERO,
        };
        for _ in 0..count {
            let pos = loop {
                let p = vec2(
                    self.rng.range(-HALF_WORLD.x, HALF_WORLD.x),
                    self.rng.range(-HALF_WORLD.y, HALF_WORLD.y),
                );
                if p.distance(ship) >= WAVE_CLEARANCE {
                    break p;
                }
            };
            let motion = asteroid_motion(&mut self.rng, 0);
            spawn_asteroid(&mut self.world, 0, pos, motion);
        }
    }

    fn control(&mut self, input: &Input) {
        let turn =
            i32::from(input.is_down(KeyCode::KeyA)) - i32::from(input.is_down(KeyCode::KeyD));
        self.world
            .storage_mut::<Spin>()
            .get_mut(self.ship)
            .expect(SHIP)
            .0 = turn as f32 * TURN_SPEED;
        let forward = self
            .world
            .storage::<Transform>()
            .get(self.ship)
            .expect(SHIP)
            .forward();
        let mut vels = self.world.storage_mut::<Velocity>();
        let vel = &mut vels.get_mut(self.ship).expect(SHIP).0;
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
        let ship = *self
            .world
            .storage::<Transform>()
            .get(self.ship)
            .expect(SHIP);
        let ship_vel = self
            .world
            .storage::<Velocity>()
            .get(self.ship)
            .expect(SHIP)
            .0;
        let dir = ship.forward();
        let pos = ship.pos + dir * SHIP_RADIUS;
        let vel = dir * BULLET_SPEED + ship_vel;
        self.cmds.spawn(move |w| {
            spawn_bullet(w, pos, vel, ship.angle);
        });
    }

    /// По движению за тик, до переноса. Пара срабатывает, только если ни одна из сущностей не ждёт удаления.
    fn collide(&mut self) {
        let mut hits = Vec::new();
        let mut ship_hit = false;
        {
            let transforms = self.world.storage::<Transform>();
            let asteroids = self.world.storage::<Asteroid>();
            let bullets = self.world.storage::<Bullet>();
            let cmds = &mut self.cmds;
            let nearest = |cmds: &Commands, mover: Entity, r: f32| {
                let m = transforms
                    .get(mover)
                    .expect("движущееся тело живо до apply");
                asteroids
                    .iter()
                    .filter(|&(a, _)| !cmds.is_pending(a))
                    .filter_map(|(a, tier)| {
                        let t = transforms.get(a).expect("у астероида есть Transform");
                        sweep_circle(m.prev - t.prev, m.pos - t.pos, r + ASTEROID_RADIUS[tier.0])
                            .map(|hit| (hit, a))
                    })
                    .min_by(|x, y| x.0.total_cmp(&y.0))
                    .map(|(_, a)| a)
            };
            for (b, _) in bullets.iter() {
                if cmds.is_pending(b) {
                    continue;
                }
                if let Some(a) = nearest(cmds, b, BULLET_RADIUS) {
                    cmds.despawn(b);
                    cmds.despawn(a);
                    hits.push(a);
                }
            }
            if self.state == ShipState::Alive
                && self.invulnerable == 0
                && let Some(a) = nearest(cmds, self.ship, SHIP_RADIUS)
            {
                cmds.despawn(a);
                hits.push(a);
                ship_hit = true;
            }
        }
        for a in hits {
            self.destroy_asteroid(a);
        }
        if ship_hit {
            self.kill_ship();
        }
    }

    /// Очки, раскол и взрыв; сам астероид уже в очереди на удаление.
    fn destroy_asteroid(&mut self, a: Entity) {
        let tier = self
            .world
            .storage::<Asteroid>()
            .get(a)
            .expect("астероид удаляется только в apply")
            .0;
        let pos = self
            .world
            .storage::<Transform>()
            .get(a)
            .expect("астероид удаляется только в apply")
            .pos;
        self.score += ASTEROID_SCORE[tier];
        self.cmds.spawn(explosion(
            self.rng.next(),
            pos,
            ASTEROID_PARTICLES[tier],
            ASTEROID_PARTICLE_TTL,
        ));
        let next = tier + 1;
        if next < ASTEROID_RADIUS.len() {
            let fragments = [
                asteroid_motion(&mut self.rng, next),
                asteroid_motion(&mut self.rng, next),
            ];
            self.cmds.spawn(move |w| {
                for motion in fragments {
                    spawn_asteroid(w, next, pos, motion);
                }
            });
        }
    }

    fn kill_ship(&mut self) {
        self.lives -= 1;
        self.state = if self.lives == 0 {
            ShipState::GameOver
        } else {
            ShipState::Dead {
                respawn_in: RESPAWN_DELAY,
            }
        };
        let pos = self
            .world
            .storage::<Transform>()
            .get(self.ship)
            .expect(SHIP)
            .pos;
        self.cmds.spawn(explosion(
            self.rng.next(),
            pos,
            SHIP_PARTICLES,
            SHIP_PARTICLE_TTL,
        ));
        self.world
            .storage_mut::<Velocity>()
            .get_mut(self.ship)
            .expect(SHIP)
            .0 = Vec2::ZERO;
    }

    fn ship_timers(&mut self) {
        self.invulnerable = self.invulnerable.saturating_sub(1);
        if let ShipState::Dead { respawn_in } = &mut self.state {
            *respawn_in -= 1;
            if *respawn_in == 0 {
                self.respawn();
            }
        }
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

    fn ship_visible(&self) -> bool {
        self.state == ShipState::Alive && (self.invulnerable / BLINK).is_multiple_of(2)
    }

    /// Спрайт на каждую видимую сущность в положении между тиками.
    fn draw_world(&self, gfx: &mut Gfx, alpha: f32) {
        let transforms = self.world.storage::<Transform>();
        let sizes = self.world.storage::<Size>();
        let asteroids = self.world.storage::<Asteroid>();
        let bullets = self.world.storage::<Bullet>();
        let particles = self.world.storage::<Particle>();
        let lifetimes = self.world.storage::<Lifetime>();
        for (e, t) in transforms.iter() {
            let Some(size) = sizes.get(e) else {
                continue;
            };
            let (texture, layer, color) = if e == self.ship {
                if !self.ship_visible() {
                    continue;
                }
                (self.art.ship, SHIP_LAYER, Color::WHITE)
            } else if asteroids.get(e).is_some() {
                (self.art.asteroid, ASTEROID_LAYER, Color::WHITE)
            } else if bullets.get(e).is_some() {
                (self.art.bullet, BULLET_LAYER, Color::WHITE)
            } else if let Some(p) = particles.get(e) {
                let color = Color {
                    a: f64::from(p.alpha(lifetimes.get(e).expect("у частицы есть Lifetime"))),
                    ..Color::from_srgb8(255, 190, 110)
                };
                (self.art.particle, PARTICLE_LAYER, color)
            } else {
                continue;
            };
            let (pos, rotation) = t.lerp(alpha);
            gfx.draw_sprite(Sprite {
                texture,
                pos,
                size: size.0,
                rotation,
                color,
                layer,
                ..Default::default()
            });
        }
    }

    fn draw_hud(&self, ctx: &mut Context) {
        let Some(hud) = &self.hud else {
            return;
        };
        let (gfx, assets) = (&mut ctx.gfx, &ctx.assets);
        let top = HALF_WORLD.y - HUD_MARGIN - HUD_PX * HUD_SCALE;
        let left = -HALF_WORLD.x + HUD_MARGIN;
        let status = format!("SCORE {}\nLIVES {}", self.score, self.lives);
        let text = |gfx: &mut Gfx, font, s: &str, pos| {
            gfx.draw_text(assets, font, s, pos, HUD_SCALE, Color::WHITE, HUD_LAYER);
        };
        text(gfx, hud.font, &status, vec2(left, top));
        let fps = format!("FPS {}", ctx.time.fps);
        let width = assets.font(hud.font).width(&fps) * HUD_SCALE;
        text(
            gfx,
            hud.font,
            &fps,
            vec2(HALF_WORLD.x - HUD_MARGIN - width, top),
        );
        if self.state == ShipState::GameOver {
            let title = "GAME OVER";
            let width = assets.font(hud.big).width(title) * HUD_SCALE;
            text(gfx, hud.big, title, vec2(-width / 2.0, 8.0));
            let hint = "PRESS ENTER";
            let width = assets.font(hud.font).width(hint) * HUD_SCALE;
            text(gfx, hud.font, hint, vec2(-width / 2.0, -20.0));
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
        #[expect(
            clippy::cast_possible_truncation,
            reason = "для сида хватает младших 64 бит"
        )]
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        let mut game = Self::new(seed);
        let assets = &mut ctx.assets;
        let mut texture = |name: &str| {
            assets.load_texture(format!("{}/assets/{name}", env!("CARGO_MANIFEST_DIR")))
        };
        game.art = Art {
            ship: texture("ship.png"),
            asteroid: texture("asteroid.png"),
            bullet: texture("bullet.png"),
            particle: texture("particle.png"),
        };
        game.hud = Some(Hud {
            font: assets.load_font(FONT_PATH, HUD_PX),
            big: assets.load_font(FONT_PATH, 2.0 * HUD_PX),
        });
        game.next_wave();
        game
    }

    fn update(&mut self, ctx: &mut Context) {
        self.tick(&ctx.input);
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        self.draw_world(&mut ctx.gfx, alpha);
        self.draw_hud(ctx);
    }
}

fn main() -> Result<(), stapel::EventLoopError> {
    stapel::run::<Asteroids>(Config {
        title: "asteroids".into(),
        size: (1280, 720),
        tick_hz: TICK_HZ,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stapel::ElementState::Pressed;

    const EPS: f32 = 1e-3;

    fn input(keys: &[KeyCode]) -> Input {
        let mut i = Input::default();
        for &k in keys {
            i.on_key(k, Pressed, false);
        }
        i
    }

    /// Вырожденный `Rng::range` зацикливает выбор места в `next_wave`: тест должен упасть раньше.
    fn assert_range_spreads() {
        let mut rng = Rng::new(1);
        let (a, b) = (rng.range(0.0, 1.0), rng.range(0.0, 1.0));
        assert!((a - b).abs() > EPS && (0.0..1.0).contains(&a) && (0.0..1.0).contains(&b));
    }

    fn seeded(seed: u64) -> Asteroids {
        assert_range_spreads();
        Asteroids::new(seed)
    }

    fn game() -> Asteroids {
        seeded(1)
    }

    fn started() -> (Asteroids, Context) {
        assert_range_spreads();
        let mut ctx = Context::default();
        let g = Asteroids::init(&mut ctx);
        (g, ctx)
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
        assert_eq!(bullets(&g).len(), 0);
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
        let xs: Vec<f32> = (0..1000).map(|_| a.range(-2.0, 3.0)).collect();
        assert!(xs.iter().all(|x| (-2.0..3.0).contains(x)));
        assert!(xs.iter().any(|&x| x < -1.9) && xs.iter().any(|&x| x > 2.9));
    }

    #[test]
    fn rng_matches_xorshift64_reference() {
        let mut rng = Rng::new(1);
        let seq = [rng.next(), rng.next(), rng.next(), rng.next()];
        assert_eq!(
            seq,
            [
                1_082_269_761,
                1_152_992_998_833_853_505,
                11_177_516_664_432_764_457,
                17_678_023_832_001_937_445,
            ]
        );
    }

    #[test]
    fn asteroid_motion_speed_and_spin_follow_tier() {
        let mut rng = Rng::new(5);
        for (tier, (lo, hi)) in ASTEROID_SPEED.into_iter().enumerate() {
            let spins: Vec<f32> = (0..200)
                .map(|_| {
                    let (vel, spin) = asteroid_motion(&mut rng, tier);
                    let speed = vel.length();
                    assert!(speed > lo - EPS && speed < hi + EPS, "{tier}: {speed}");
                    spin
                })
                .collect();
            let max = ASTEROID_MAX_SPIN;
            assert!(spins.iter().all(|s| (-max..max).contains(s)));
            assert!(spins.iter().any(|&s| s < -1.0) && spins.iter().any(|&s| s > 1.0));
        }
    }

    #[test]
    fn wrap_shift_needs_strictly_beyond_edge() {
        assert_eq!(wrap_shift(vec2(321.0, -181.0), 1.0), Vec2::ZERO);
        assert_eq!(wrap_shift(vec2(-321.5, 181.5), 1.0), vec2(642.0, -362.0));
    }

    fn asteroid(g: &mut Asteroids, tier: usize, pos: Vec2) -> Entity {
        spawn_asteroid(&mut g.world, tier, pos, (Vec2::ZERO, 0.0))
    }

    fn asteroids(g: &Asteroids) -> Vec<(usize, Transform)> {
        let transforms = g.world.storage::<Transform>();
        g.world
            .storage::<Asteroid>()
            .iter()
            .map(|(e, a)| (a.0, *transforms.get(e).unwrap()))
            .collect()
    }

    fn clear_asteroids(g: &mut Asteroids) {
        let all: Vec<Entity> = g
            .world
            .storage::<Asteroid>()
            .iter()
            .map(|(e, _)| e)
            .collect();
        for e in all {
            g.world.despawn(e);
        }
    }

    #[test]
    fn sweep_circle_hits_misses_and_starts_inside() {
        let t = sweep_circle(vec2(-20.0, 0.0), Vec2::ZERO, 5.0).unwrap();
        assert!((t - 0.75).abs() < EPS);
        assert_eq!(sweep_circle(vec2(-20.0, 6.0), vec2(20.0, 6.0), 5.0), None);
        assert_eq!(sweep_circle(vec2(-20.0, 0.0), vec2(-10.0, 0.0), 5.0), None);
        assert_eq!(
            sweep_circle(vec2(1.0, 1.0), vec2(30.0, 0.0), 5.0),
            Some(0.0)
        );
    }

    #[test]
    fn sweep_circle_counts_tangent_touch() {
        assert_eq!(
            sweep_circle(vec2(-10.0, 5.0), vec2(10.0, 5.0), 5.0),
            Some(0.5)
        );
    }

    #[test]
    fn sweep_circle_catches_graze_between_endpoints() {
        let (from, to) = (vec2(-1.0, 9.99), vec2(1.0, 9.99));
        assert!(from.length() > 10.0 && to.length() > 10.0);
        let t = sweep_circle(from, to, 10.0).unwrap();
        assert!(t > 0.0 && t < 1.0);
    }

    #[test]
    fn two_bullets_in_one_asteroid_score_and_split_once() {
        let mut g = game();
        asteroid(&mut g, 0, vec2(100.0, 0.0));
        spawn_bullet(&mut g.world, vec2(60.0, 0.0), vec2(BULLET_SPEED, 0.0), 0.0);
        spawn_bullet(&mut g.world, vec2(60.0, 1.0), vec2(BULLET_SPEED, 0.0), 0.0);
        g.tick(&Input::default());
        assert_eq!(g.score, ASTEROID_SCORE[0]);
        assert_eq!(bullets(&g).len(), 1);
        let left = asteroids(&g);
        assert_eq!(left.len(), 2);
        for (tier, t) in left {
            assert_eq!(tier, 1);
            assert_eq!(t.prev, t.pos);
            assert!((t.pos - vec2(100.0, 0.0)).length() < EPS);
        }
    }

    #[test]
    fn bullet_hits_nearest_asteroid_on_its_path() {
        let mut g = game();
        let far = asteroid(&mut g, 2, vec2(58.0, 0.0));
        let near = asteroid(&mut g, 2, vec2(52.0, 0.0));
        spawn_bullet(&mut g.world, vec2(40.0, 0.0), vec2(1200.0, 0.0), 0.0);
        g.tick(&Input::default());
        assert!(!g.world.is_alive(near));
        assert!(g.world.is_alive(far));
        assert_eq!(g.score, ASTEROID_SCORE[2]);
    }

    #[test]
    fn bullet_takes_asteroid_before_ship() {
        let mut g = game();
        set_ship(
            &mut g,
            Transform::at(Vec2::ZERO, 0.0),
            vec2(MAX_SPEED, 0.0) / DRAG.powf(DT),
        );
        asteroid(&mut g, 1, vec2(26.0, 0.0));
        spawn_bullet(&mut g.world, vec2(50.0, 0.0), vec2(-BULLET_SPEED, 0.0), 0.0);
        g.tick(&Input::default());
        assert_eq!(g.score, ASTEROID_SCORE[1]);
        assert_eq!(g.state, ShipState::Alive);
        assert_eq!(g.lives, LIVES);
    }

    #[test]
    fn ship_death_costs_life_then_respawns_invulnerable() {
        let mut g = game();
        set_ship(&mut g, Transform::at(vec2(100.0, 0.0), 0.0), Vec2::ZERO);
        asteroid(&mut g, 2, vec2(105.0, 0.0));
        g.tick(&Input::default());
        assert_eq!(g.lives, LIVES - 1);
        assert_eq!(g.score, ASTEROID_SCORE[2]);
        for _ in 1..2 * TICK_HZ {
            assert!(matches!(g.state, ShipState::Dead { .. }));
            g.tick(&input(&[KeyCode::Space]));
        }
        assert_eq!(g.state, ShipState::Alive);
        assert_eq!(bullets(&g).len(), 0);
        let t = ship(&g);
        assert_eq!(t.pos, Vec2::ZERO);
        assert_eq!(t.prev, t.pos);

        asteroid(&mut g, 2, Vec2::ZERO);
        for _ in 0..2 * TICK_HZ {
            g.tick(&Input::default());
        }
        assert_eq!(g.lives, LIVES - 1);
        g.tick(&Input::default());
        assert_eq!(g.lives, LIVES - 2);
    }

    #[test]
    fn cleared_wave_brings_one_more_asteroid_away_from_ship() {
        let mut g = game();
        g.next_wave();
        assert_eq!(asteroids(&g).len(), WAVE_START as usize);
        let ship_pos = vec2(200.0, 100.0);
        set_ship(&mut g, Transform::at(ship_pos, 0.0), Vec2::ZERO);
        let mut spawned = Vec::new();
        for expected in (WAVE_START + 1..=WAVE_MAX).chain([WAVE_MAX]) {
            clear_asteroids(&mut g);
            g.tick(&Input::default());
            let wave = asteroids(&g);
            assert_eq!(wave.len(), expected as usize);
            for (tier, t) in wave {
                assert_eq!(tier, 0);
                assert_eq!(t.prev, t.pos);
                assert!(t.pos.distance(ship_pos) >= WAVE_CLEARANCE);
                spawned.push(t.pos);
            }
        }
        let inside = |p: &Vec2| (-320.0..320.0).contains(&p.x) && (-180.0..180.0).contains(&p.y);
        assert!(spawned.iter().all(inside));
        assert!(spawned.iter().any(|p| p.x < -160.0) && spawned.iter().any(|p| p.y < -90.0));
    }

    #[test]
    fn init_sets_camera_fonts_and_first_wave() {
        let (g, ctx) = started();
        assert_eq!(ctx.gfx.camera.size, vec2(640.0, 360.0));
        assert_eq!((g.wave, asteroids(&g).len()), (1, WAVE_START as usize));
        let hud = g.hud.as_ref().unwrap();
        let px = |font| ctx.assets.font(font).line_height;
        let ratio = px(hud.big) / px(hud.font);
        assert!((ratio - 2.0).abs() < 0.01, "ratio {ratio}");
    }

    #[test]
    fn update_ticks_with_context_input() {
        let (mut g, mut ctx) = started();
        ctx.input.on_key(KeyCode::KeyA, Pressed, false);
        g.update(&mut ctx);
        assert!((ship(&g).angle - TURN_SPEED * DT).abs() < EPS);
    }

    #[test]
    fn draw_uses_loaded_textures() {
        let (mut g, mut ctx) = started();
        g.draw(&mut ctx, 0.0);
        let of = |layer| ctx.gfx.sprites.iter().filter(move |s| s.layer == layer);
        assert_eq!(of(ASTEROID_LAYER).count(), WAVE_START as usize);
        assert!(of(ASTEROID_LAYER).all(|s| s.texture == g.art.asteroid));
        assert_eq!(of(SHIP_LAYER).next().unwrap().texture, g.art.ship);
        assert_ne!(g.art.asteroid, Handle::WHITE);
        assert_ne!(g.art.asteroid, g.art.ship);
    }

    fn hud(gfx: &Gfx) -> Vec<Sprite> {
        (gfx.sprites.iter())
            .filter(|s| s.layer == HUD_LAYER)
            .copied()
            .collect()
    }

    #[test]
    fn hud_pins_status_and_fps_to_corners_and_centers_game_over() {
        let (mut g, mut ctx) = started();
        (g.score, g.lives, ctx.time.fps) = (120, 2, 60);
        g.draw(&mut ctx, 0.0);
        let playing = hud(&ctx.gfx);
        ctx.gfx.sprites.clear();
        g.state = ShipState::GameOver;
        g.draw(&mut ctx, 0.0);
        let over = hud(&ctx.gfx);

        let Hud { font, big } = *g.hud.as_ref().unwrap();
        let width = |f, s| ctx.assets.font(f).width(s) * HUD_SCALE;
        let text = |f, s, pos| {
            stapel::text::layout(
                ctx.assets.font(f),
                s,
                pos,
                HUD_SCALE,
                Color::WHITE,
                HUD_LAYER,
            )
        };
        let status: Vec<Sprite> = text(font, "SCORE 120\nLIVES 2", vec2(-314.0, 158.0))
            .chain(text(
                font,
                "FPS 60",
                vec2(314.0 - width(font, "FPS 60"), 158.0),
            ))
            .collect();
        assert_eq!(playing, status);
        let title = text(big, "GAME OVER", vec2(-width(big, "GAME OVER") / 2.0, 8.0));
        let hint = text(
            font,
            "PRESS ENTER",
            vec2(-width(font, "PRESS ENTER") / 2.0, -20.0),
        );
        let expected: Vec<Sprite> = status.into_iter().chain(title).chain(hint).collect();
        assert_eq!(over, expected);
    }

    #[test]
    fn last_life_ends_game_and_enter_restarts() {
        let mut g = game();
        g.next_wave();
        clear_asteroids(&mut g);
        g.lives = 1;
        asteroid(&mut g, 2, vec2(3.0, 0.0));
        asteroid(&mut g, 0, vec2(-250.0, 150.0));
        g.tick(&Input::default());
        assert_eq!(g.state, ShipState::GameOver);
        for _ in 0..2 * RESPAWN_DELAY {
            g.tick(&Input::default());
        }
        assert_eq!(g.state, ShipState::GameOver);
        g.tick(&input(&[KeyCode::Enter]));
        assert_eq!(g.state, ShipState::Alive);
        assert_eq!((g.lives, g.score, g.wave), (LIVES, 0, 1));
        assert_eq!(asteroids(&g).len(), WAVE_START as usize);
    }

    fn particles(g: &Asteroids) -> Vec<(Transform, Vec2, f32)> {
        let transforms = g.world.storage::<Transform>();
        let vels = g.world.storage::<Velocity>();
        let lifetimes = g.world.storage::<Lifetime>();
        g.world
            .storage::<Particle>()
            .iter()
            .map(|(e, p)| {
                let t = *transforms.get(e).unwrap();
                (
                    t,
                    vels.get(e).unwrap().0,
                    p.alpha(lifetimes.get(e).unwrap()),
                )
            })
            .collect()
    }

    #[test]
    fn big_asteroid_explodes_into_300_particles_from_one_command() {
        let mut g = game();
        let mut cmds = Commands::default();
        let pos = vec2(10.0, 20.0);
        cmds.spawn(explosion(
            7,
            pos,
            ASTEROID_PARTICLES[0],
            ASTEROID_PARTICLE_TTL,
        ));
        cmds.apply(&mut g.world);
        let ps = particles(&g);
        assert_eq!(ps.len(), 300);
        let (lo, hi) = PARTICLE_SPEED;
        for (t, vel, alpha) in ps {
            assert_eq!((t.pos, t.prev), (pos, pos));
            assert!((lo..hi).contains(&vel.length()));
            assert_eq!(alpha, 1.0);
        }
        let (lo, hi) = ASTEROID_PARTICLE_TTL;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "границы TTL — положительные секунды"
        )]
        let ttl = (lo * TICK_HZ as f32) as u32..=(hi * TICK_HZ as f32) as u32;
        for (_, left) in g.world.storage::<Lifetime>().iter() {
            assert!(ttl.contains(&left.0));
        }

        let mut g = game();
        asteroid(&mut g, 0, vec2(100.0, 0.0));
        spawn_bullet(&mut g.world, vec2(60.0, 0.0), vec2(BULLET_SPEED, 0.0), 0.0);
        g.tick(&Input::default());
        assert_eq!(particles(&g).len(), 300);
    }

    #[test]
    fn two_explosions_in_one_tick_differ() {
        let mut g = game();
        asteroid(&mut g, 0, vec2(100.0, 0.0));
        asteroid(&mut g, 0, vec2(100.0, 0.0));
        spawn_bullet(&mut g.world, vec2(60.0, 0.0), vec2(BULLET_SPEED, 0.0), 0.0);
        spawn_bullet(&mut g.world, vec2(60.0, 1.0), vec2(BULLET_SPEED, 0.0), 0.0);
        g.tick(&Input::default());
        assert_eq!(g.score, 2 * ASTEROID_SCORE[0]);
        let vels: std::collections::HashSet<_> = particles(&g)
            .into_iter()
            .map(|(_, v, _)| (v.x.to_bits(), v.y.to_bits()))
            .collect();
        assert_eq!(vels.len(), 2 * ASTEROID_PARTICLES[0] as usize);
    }

    #[test]
    fn particle_fades_with_ttl_then_disappears() {
        let mut g = game();
        let ttl = TICK_HZ / 2;
        g.cmds.spawn(explosion(3, vec2(50.0, 50.0), 1, (0.5, 0.5)));
        g.cmds.apply(&mut g.world);
        let mut last = particles(&g)[0].2;
        assert_eq!(last, 1.0);
        for _ in 1..ttl {
            g.tick(&Input::default());
            let alpha = particles(&g)[0].2;
            assert!(alpha < last && alpha > 0.0);
            last = alpha;
        }
        g.tick(&Input::default());
        assert_eq!(particles(&g).len(), 0);
        assert_eq!(g.world.storage::<Transform>().len(), 1);
    }

    #[test]
    fn ship_death_holds_over_5000_particles_for_two_seconds() {
        let mut g = seeded(42);
        g.next_wave();
        asteroid(&mut g, 2, vec2(3.0, 0.0));
        g.tick(&Input::default());
        assert!(matches!(g.state, ShipState::Dead { .. }));
        let mut streak = 0;
        while particles(&g).len() > 5_000 {
            assert!(streak < 5 * TICK_HZ, "частицы не гаснут");
            streak += 1;
            g.tick(&Input::default());
        }
        assert!(streak >= 2 * TICK_HZ, "streak {streak}");
    }

    #[test]
    fn sprite_system_draws_visible_entities_interpolated() {
        let moving = |prev: Vec2, pos: Vec2, prev_angle: f32, angle: f32| Transform {
            pos,
            prev,
            angle,
            prev_angle,
        };
        let mut g = game();
        set_ship(
            &mut g,
            moving(Vec2::ZERO, vec2(10.0, 0.0), 0.0, 1.0),
            Vec2::ZERO,
        );
        let a = asteroid(&mut g, 0, Vec2::ZERO);
        *g.world.storage_mut::<Transform>().get_mut(a).unwrap() =
            moving(vec2(100.0, 0.0), vec2(100.0, 20.0), 2.0, 3.0);
        spawn_bullet(&mut g.world, vec2(-50.0, 0.0), Vec2::ZERO, 0.5);
        g.cmds.spawn(explosion(3, vec2(0.0, -50.0), 5, (1.0, 1.0)));
        g.cmds.apply(&mut g.world);

        let draw = |g: &Asteroids| {
            let mut gfx = Gfx::default();
            g.draw_world(&mut gfx, 0.25);
            gfx.sprites
        };
        let sprites = draw(&g);
        assert_eq!(sprites.len(), 1 + 1 + 1 + 5);
        let of = |layer| sprites.iter().filter(move |s| s.layer == layer);
        let ship = of(SHIP_LAYER).next().unwrap();
        assert!((ship.pos - vec2(2.5, 0.0)).length() < EPS);
        assert!((ship.rotation - 0.25).abs() < EPS);
        assert_eq!(ship.size, Vec2::splat(SHIP_RADIUS * 2.0));
        let rock = of(ASTEROID_LAYER).next().unwrap();
        assert!((rock.pos - vec2(100.0, 5.0)).length() < EPS);
        assert!((rock.rotation - 2.25).abs() < EPS);
        assert_eq!(of(BULLET_LAYER).next().unwrap().rotation, 0.5);
        assert_eq!(of(PARTICLE_LAYER).count(), 5);
        for s in of(PARTICLE_LAYER) {
            assert_eq!(s.color.a, 1.0);
        }

        assert_eq!(rock.color, Color::WHITE);
        let ember = Color::from_srgb8(255, 190, 110);
        assert!(of(PARTICLE_LAYER).all(|s| s.color == ember));
        for (_, left) in g.world.storage_mut::<Lifetime>().iter_mut() {
            left.0 = TICK_HZ / 4;
        }
        let faded = Color { a: 0.25, ..ember };
        let colors: Vec<Color> = (draw(&g).iter())
            .filter(|s| s.layer == PARTICLE_LAYER)
            .map(|s| s.color)
            .collect();
        assert_eq!(colors, [faded; 5]);

        g.invulnerable = TICK_HZ / 10;
        assert_eq!(draw(&g).len(), 7);
        g.invulnerable = 2 * TICK_HZ / 10;
        assert_eq!(draw(&g).len(), 8);
        g.state = ShipState::Dead { respawn_in: 1 };
        assert!(draw(&g).iter().all(|s| s.layer != SHIP_LAYER));
    }
}
