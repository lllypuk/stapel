use std::{fmt, iter};

use engine::{
    Aabb, Actor, Axis, Camera2D, Color, Config, Context, Game, Gfx, Handle, IVec2, Input, KeyCode,
    Rect, Solid, Sprite, Texture, Tilemap, Vec2, World, grounded, ivec2, move_actor, move_solid,
    vec2,
};

const TICK_HZ: u32 = 60;
/// Сторона тайла в мировых пикселях.
const TILE: i32 = 18;
/// Видимая часть мира; чётная, поэтому края вида на целых пикселях при целом центре.
const VIEW: IVec2 = ivec2(320, 180);
/// Полуразмеры зоны вокруг центра камеры, внутри которой цель камеру не двигает.
const DEAD_ZONE: IVec2 = ivec2(24, 24);
const TILES_ATLAS: Vec2 = vec2(360.0, 162.0);
/// Ячейки атласа тайлов.
const GRASS: IVec2 = ivec2(2, 0);
const DIRT: IVec2 = ivec2(2, 6);
const TILE_LAYER: i32 = 1;
/// Ячейки платформ: одна клетка в ширину — блок, иначе доска с краями.
const BLOCK: IVec2 = ivec2(7, 2);
const PLANK_LEFT: IVec2 = ivec2(8, 2);
const PLANK: IVec2 = ivec2(9, 2);
const PLANK_RIGHT: IVec2 = ivec2(10, 2);
const PLATFORM_LAYER: i32 = 2;
const CHARS_ATLAS: Vec2 = vec2(216.0, 72.0);
/// Сторона кадра персонажа.
const CHAR: i32 = 24;
/// Кадры игрока в атласе персонажей; нарисован лицом влево.
const IDLE: IVec2 = ivec2(0, 0);
const WALK: IVec2 = ivec2(1, 0);
const PLAYER_LAYER: i32 = 4;
/// Кадры врага.
const WALKER: IVec2 = ivec2(0, 2);
const WALKER_STEP: IVec2 = ivec2(1, 2);
const ENEMY_LAYER: i32 = 3;
const ENEMY_SIZE: IVec2 = ivec2(14, 12);
const ENEMY_SPEED: f32 = 0.5;
/// Пикселей пути на кадр ходьбы врага.
const ENEMY_FRAME: i32 = 4;
const PLAYER_SIZE: IVec2 = ivec2(12, 16);
/// Скорости — в px/тик, ускорения — в px/тик².
const RUN_MAX: f32 = 1.5;
const RUN_ACCEL: f32 = 0.1;
const GRAVITY: f32 = 0.25;
const JUMP: f32 = 4.5;
const FALL_MAX: f32 = 5.0;
/// Множитель скорости при отпускании прыжка; через `min` подъём режет, а падение — нет.
const JUMP_CUT: f32 = 0.5;
/// Сколько тиков после края прыжок ещё срабатывает.
const COYOTE: u32 = 6;
/// За сколько тиков до приземления нажатие прыжка ещё срабатывает.
const JUMP_BUFFER: u32 = 6;
/// Тиков на кадр анимации бега.
const RUN_FRAME: u32 = 8;
const LEVEL1: &str = include_str!("../assets/level1.txt");

/// Ошибка разбора карты; `line` — с единицы, первая строка — верх.
#[derive(Debug, PartialEq)]
enum LevelError {
    Ragged { line: usize },
    Unknown { line: usize, ch: char },
    NoPlayer,
    TwoPlayers { line: usize },
}

impl fmt::Display for LevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ragged { line } => write!(f, "строка {line}: длина не как у первой"),
            Self::Unknown { line, ch } => write!(f, "строка {line}: неизвестный символ {ch:?}"),
            Self::NoPlayer => write!(f, "нет P"),
            Self::TwoPlayers { line } => write!(f, "строка {line}: второй P"),
        }
    }
}

/// Платформа из подряд идущих `=` (по X) или `H` (по Y); `cells` — в клетках.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlatformSpawn {
    cells: Aabb,
    axis: Axis,
}

/// Уровень из ASCII-карты; клетки — от левой нижней, Y вверх.
#[derive(Debug, PartialEq)]
struct Level {
    size: IVec2,
    /// По строкам снизу вверх.
    solid: Vec<bool>,
    player: IVec2,
    enemies: Vec<IVec2>,
    coins: Vec<IVec2>,
    spikes: Vec<IVec2>,
    exits: Vec<IVec2>,
    stops: Vec<IVec2>,
    platforms: Vec<PlatformSpawn>,
}

impl Level {
    fn parse(text: &str) -> Result<Self, LevelError> {
        let rows: Vec<Vec<char>> = text.lines().map(|l| l.chars().collect()).collect();
        let width = rows.first().map_or(0, Vec::len);
        let to_i32 = |n: usize| i32::try_from(n).expect("карта меньше 2³¹ клеток");
        let size = ivec2(to_i32(width), to_i32(rows.len()));
        let mut level = Self {
            size,
            solid: vec![false; width * rows.len()],
            player: IVec2::ZERO,
            enemies: Vec::new(),
            coins: Vec::new(),
            spikes: Vec::new(),
            exits: Vec::new(),
            stops: Vec::new(),
            platforms: Vec::new(),
        };
        let mut player = None;
        for (i, row) in rows.iter().enumerate() {
            let line = i + 1;
            if row.len() != width {
                return Err(LevelError::Ragged { line });
            }
            let y = size.y - 1 - to_i32(i);
            for (j, &ch) in row.iter().enumerate() {
                let cell = ivec2(to_i32(j), y);
                match ch {
                    '#' => level.solid[to_index(cell, size.x)] = true,
                    'P' if player.is_some() => return Err(LevelError::TwoPlayers { line }),
                    'P' => player = Some(cell),
                    'E' => level.enemies.push(cell),
                    'o' => level.coins.push(cell),
                    '^' => level.spikes.push(cell),
                    'X' => level.exits.push(cell),
                    '!' => level.stops.push(cell),
                    '=' if j == 0 || row[j - 1] != '=' => {
                        let len = row[j..].iter().take_while(|&&c| c == '=').count();
                        level.platforms.push(PlatformSpawn {
                            cells: Aabb {
                                min: cell,
                                size: ivec2(to_i32(len), 1),
                            },
                            axis: Axis::X,
                        });
                    }
                    'H' if i == 0 || rows[i - 1][j] != 'H' => {
                        let len = rows[i..]
                            .iter()
                            .take_while(|r| r.get(j) == Some(&'H'))
                            .count();
                        level.platforms.push(PlatformSpawn {
                            cells: Aabb {
                                min: ivec2(cell.x, y + 1 - to_i32(len)),
                                size: ivec2(1, to_i32(len)),
                            },
                            axis: Axis::Y,
                        });
                    }
                    ' ' | '.' | '=' | 'H' => {}
                    ch => return Err(LevelError::Unknown { line, ch }),
                }
            }
        }
        level.player = player.ok_or(LevelError::NoPlayer)?;
        Ok(level)
    }

    /// Вне карты — не твёрдое.
    fn solid_at(&self, cell: IVec2) -> bool {
        cell.cmpge(IVec2::ZERO).all()
            && cell.cmplt(self.size).all()
            && self.solid[to_index(cell, self.size.x)]
    }

    fn bounds(&self) -> IVec2 {
        self.size * TILE
    }
}

#[expect(
    clippy::cast_sign_loss,
    reason = "клетка внутри карты, координаты неотрицательны"
)]
fn to_index(cell: IVec2, width: i32) -> usize {
    (cell.y * width + cell.x) as usize
}

fn cell_aabb(cell: IVec2) -> Aabb {
    Aabb {
        min: cell * TILE,
        size: IVec2::splat(TILE),
    }
}

/// Центр клетки в мировых пикселях.
fn cell_center(cell: IVec2) -> Vec2 {
    (cell * TILE + TILE / 2).as_vec2()
}

/// UV ячейки `cell` атласа `atlas` с ячейками `size × size` без зазоров.
fn atlas_uv(cell: IVec2, size: i32, atlas: Vec2) -> Rect {
    let min = (cell * size).as_vec2() / atlas;
    Rect {
        min,
        max: min + Vec2::splat(size as f32) / atlas,
    }
}

/// Положение между тиками, округлённое до целого пикселя.
fn lerp_round(prev: IVec2, cur: IVec2, alpha: f32) -> Vec2 {
    prev.as_vec2().lerp(cur.as_vec2(), alpha).round()
}

/// Кадр `cell` атласа персонажей низом на низ хитбокса `[min, min + width)`, по центру.
fn char_sprite(
    texture: Handle<Texture>,
    min: Vec2,
    width: i32,
    cell: IVec2,
    facing: i32,
    layer: i32,
) -> Sprite {
    let mut uv = atlas_uv(cell, CHAR, CHARS_ATLAS);
    // Персонажи в атласе смотрят влево.
    if facing == 1 {
        std::mem::swap(&mut uv.min.x, &mut uv.max.x);
    }
    Sprite {
        texture,
        pos: min + vec2(width as f32 / 2.0, CHAR as f32 / 2.0),
        size: Vec2::splat(CHAR as f32),
        uv,
        layer,
        ..Default::default()
    }
}

/// Звуковое событие тика.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sfx {
    Jump,
}

const LEFT: &[KeyCode] = &[KeyCode::KeyA, KeyCode::ArrowLeft];
const RIGHT: &[KeyCode] = &[KeyCode::KeyD, KeyCode::ArrowRight];
const JUMP_KEYS: &[KeyCode] = &[KeyCode::Space, KeyCode::KeyW];

/// Ввод игрока за тик.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Controls {
    /// `-1`, `0` или `1`.
    run: i32,
    jump: bool,
    /// Отпущены все клавиши прыжка.
    release: bool,
}

impl Controls {
    fn read(input: &Input) -> Self {
        let any =
            |keys: &[KeyCode], f: fn(&Input, KeyCode) -> bool| keys.iter().any(|&k| f(input, k));
        Self {
            run: i32::from(any(RIGHT, Input::is_down)) - i32::from(any(LEFT, Input::is_down)),
            jump: any(JUMP_KEYS, Input::just_pressed),
            release: any(JUMP_KEYS, Input::just_released) && !any(JUMP_KEYS, Input::is_down),
        }
    }
}

/// `v`, сдвинутое к `target` не больше чем на `step`.
fn approach(v: f32, target: f32, step: f32) -> f32 {
    v + (target - v).clamp(-step, step)
}

struct Player {
    body: Actor,
    /// `body.aabb.min` на начало тика.
    prev: IVec2,
    vel: Vec2,
    /// `1` — вправо, `-1` — влево.
    facing: i32,
    /// Тиков с последнего тика на земле; `0` — на земле.
    air: u32,
    /// Тиков с нажатия прыжка, ещё не потраченного.
    since_jump: u32,
    /// Тиков бега по земле подряд.
    run_ticks: u32,
}

impl Player {
    /// Стоит на дне клетки `cell`, по центру.
    fn spawn(cell: IVec2) -> Self {
        let min = cell * TILE + ivec2((TILE - PLAYER_SIZE.x) / 2, 0);
        Self {
            body: Actor {
                aabb: Aabb {
                    min,
                    size: PLAYER_SIZE,
                },
                remainder: Vec2::ZERO,
            },
            prev: min,
            vel: Vec2::ZERO,
            facing: 1,
            air: 0,
            since_jump: u32::MAX,
            run_ticks: 0,
        }
    }

    fn step(&mut self, map: &Tilemap, solids: &[Aabb], c: Controls, sfx: &mut Vec<Sfx>) {
        self.air = if grounded(map, solids, &self.body.aabb) {
            0
        } else {
            self.air.saturating_add(1)
        };
        self.since_jump = if c.jump {
            0
        } else {
            self.since_jump.saturating_add(1)
        };
        if c.run != 0 {
            self.facing = c.run;
        }
        self.vel.x = approach(self.vel.x, c.run as f32 * RUN_MAX, RUN_ACCEL);
        self.vel.y = (self.vel.y - GRAVITY).max(-FALL_MAX);
        if self.air <= COYOTE && self.since_jump <= JUMP_BUFFER {
            self.vel.y = JUMP;
            self.air = u32::MAX;
            self.since_jump = u32::MAX;
            sfx.push(Sfx::Jump);
        }
        if c.release {
            self.vel.y = self.vel.y.min(self.vel.y * JUMP_CUT);
        }
        let hits = move_actor(map, solids, &mut self.body, self.vel, &[], &mut Vec::new());
        if hits.x {
            self.vel.x = 0.0;
        }
        if hits.y {
            self.vel.y = 0.0;
        }
        self.run_ticks = if self.air == 0 && self.vel.x != 0.0 {
            self.run_ticks + 1
        } else {
            0
        };
    }

    fn center(&self) -> IVec2 {
        self.body.aabb.min + self.body.aabb.size / 2
    }

    fn frame(&self) -> IVec2 {
        if self.air > 0 || (self.run_ticks / RUN_FRAME) % 2 == 1 {
            WALK
        } else {
            IDLE
        }
    }

    fn sprite(&self, texture: Handle<Texture>, alpha: f32) -> Sprite {
        let min = lerp_round(self.prev, self.body.aabb.min, alpha);
        char_sprite(
            texture,
            min,
            PLAYER_SIZE.x,
            self.frame(),
            self.facing,
            PLAYER_LAYER,
        )
    }
}

/// Движущаяся платформа; разворачивается перед `!` и твёрдым.
struct Platform {
    body: Solid,
    prev: IVec2,
    /// Шаг за тик: 1 px вдоль оси хода.
    dir: IVec2,
}

impl Platform {
    fn spawn(spawn: PlatformSpawn) -> Self {
        let aabb = Aabb {
            min: spawn.cells.min * TILE,
            size: spawn.cells.size * TILE,
        };
        Self {
            body: Solid {
                aabb,
                remainder: Vec2::ZERO,
            },
            prev: aabb.min,
            dir: match spawn.axis {
                Axis::X => IVec2::X,
                Axis::Y => IVec2::Y,
            },
        }
    }

    fn sprites(&self, texture: Handle<Texture>, alpha: f32) -> impl Iterator<Item = Sprite> {
        let min = lerp_round(self.prev, self.body.aabb.min, alpha);
        let cells = self.body.aabb.size / TILE;
        (0..cells.y).flat_map(move |y| {
            (0..cells.x).map(move |x| {
                let art = match x {
                    _ if cells.x == 1 => BLOCK,
                    0 => PLANK_LEFT,
                    x if x == cells.x - 1 => PLANK_RIGHT,
                    _ => PLANK,
                };
                Sprite {
                    texture,
                    pos: min + cell_center(ivec2(x, y)),
                    size: Vec2::splat(TILE as f32),
                    uv: atlas_uv(art, TILE, TILES_ATLAS),
                    layer: PLATFORM_LAYER,
                    ..Default::default()
                }
            })
        })
    }
}

/// Патрульный враг: разворачивается у стены и перед краем опоры.
struct Enemy {
    body: Actor,
    prev: IVec2,
    /// `1` — вправо, `-1` — влево.
    dir: i32,
    fall: f32,
}

impl Enemy {
    /// Стоит на дне клетки `cell`, по центру.
    fn spawn(cell: IVec2) -> Self {
        let min = cell * TILE + ivec2((TILE - ENEMY_SIZE.x) / 2, 0);
        Self {
            body: Actor {
                aabb: Aabb {
                    min,
                    size: ENEMY_SIZE,
                },
                remainder: Vec2::ZERO,
            },
            prev: min,
            dir: -1,
            fall: 0.0,
        }
    }

    fn step(&mut self, map: &Tilemap, solids: &[Aabb]) {
        self.fall = (self.fall - GRAVITY).max(-FALL_MAX);
        let aabb = self.body.aabb;
        let ahead = Aabb {
            min: ivec2(
                if self.dir == 1 {
                    aabb.max().x
                } else {
                    aabb.min.x - 1
                },
                aabb.min.y - 1,
            ),
            size: IVec2::ONE,
        };
        let support = map.overlaps_solid(&ahead) || solids.iter().any(|s| s.overlaps(&ahead));
        if !support && grounded(map, solids, &aabb) {
            self.dir = -self.dir;
        }
        let delta = vec2(self.dir as f32 * ENEMY_SPEED, self.fall);
        let hits = move_actor(map, solids, &mut self.body, delta, &[], &mut Vec::new());
        if hits.x {
            self.dir = -self.dir;
        }
        if hits.y {
            self.fall = 0.0;
        }
    }

    fn sprite(&self, texture: Handle<Texture>, alpha: f32) -> Sprite {
        let min = lerp_round(self.prev, self.body.aabb.min, alpha);
        let frame = if self.body.aabb.min.x.div_euclid(ENEMY_FRAME) % 2 == 0 {
            WALKER
        } else {
            WALKER_STEP
        };
        char_sprite(texture, min, ENEMY_SIZE.x, frame, self.dir, ENEMY_LAYER)
    }
}

/// Центр камеры в мировых пикселях на конец и начало тика.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Follow {
    pos: IVec2,
    prev: IVec2,
}

impl Follow {
    fn at(pos: IVec2) -> Self {
        Self { pos, prev: pos }
    }

    /// Держит `target` в мёртвой зоне, а вид — внутри `bounds`; уровень уже вида прижат к началу.
    fn follow(&mut self, target: IVec2, bounds: IVec2) {
        let pos = self.pos.clamp(target - DEAD_ZONE, target + DEAD_ZONE);
        self.pos = pos.min(bounds - VIEW / 2).max(VIEW / 2);
    }

    fn lerp(&self, alpha: f32) -> Vec2 {
        lerp_round(self.prev, self.pos, alpha)
    }
}

struct Platformer {
    level: Level,
    map: Tilemap,
    player: Player,
    /// Сущности с `Platform` и `Enemy`.
    world: World,
    camera: Follow,
    /// События тика; звучат в `update`.
    sfx: Vec<Sfx>,
    tiles: Handle<Texture>,
    chars: Handle<Texture>,
}

impl Platformer {
    fn new(level: Level) -> Self {
        let map = Tilemap::new(TILE, level.size.x, level.size.y, level.solid.clone());
        let player = Player::spawn(level.player);
        let mut world = World::default();
        world.register::<Platform>();
        world.register::<Enemy>();
        for &p in &level.platforms {
            let e = world.spawn();
            world.insert(e, Platform::spawn(p));
        }
        for &cell in &level.enemies {
            let e = world.spawn();
            world.insert(e, Enemy::spawn(cell));
        }
        let mut camera = Follow::at(level.bounds() / 2);
        camera.follow(player.center(), level.bounds());
        camera.prev = camera.pos;
        Self {
            level,
            map,
            player,
            world,
            camera,
            sfx: Vec::new(),
            tiles: Handle::WHITE,
            chars: Handle::WHITE,
        }
    }

    fn tick(&mut self, input: &Input) {
        self.sfx.clear();
        self.camera.prev = self.camera.pos;
        self.player.prev = self.player.body.aabb.min;
        let solids = self.step_bodies();
        self.player
            .step(&self.map, &solids, Controls::read(input), &mut self.sfx);
        self.camera
            .follow(self.player.center(), self.level.bounds());
    }

    /// Платформы везут и толкают игрока и врагов, затем ходят враги; отдаёт платформы для шага игрока.
    fn step_bodies(&mut self) -> Vec<Aabb> {
        let mut platforms = self.world.storage_mut::<Platform>();
        let mut enemies = self.world.storage_mut::<Enemy>();
        for (_, p) in platforms.iter_mut() {
            p.prev = p.body.aabb.min;
        }
        for (_, e) in enemies.iter_mut() {
            e.prev = e.body.aabb.min;
        }
        let mut solids: Vec<Solid> = platforms.iter().map(|(_, p)| p.body).collect();
        let mut actors: Vec<Actor> = iter::once(self.player.body)
            .chain(enemies.iter().map(|(_, e)| e.body))
            .collect();
        for (i, (_, p)) in platforms.iter_mut().enumerate() {
            let blocked = |dir: IVec2| {
                let ahead = Aabb {
                    min: p.body.aabb.min + dir,
                    ..p.body.aabb
                };
                let stop = |&cell: &IVec2| cell_aabb(cell).overlaps(&ahead);
                self.map.overlaps_solid(&ahead) || self.level.stops.iter().any(stop)
            };
            if blocked(p.dir) {
                p.dir = -p.dir;
            }
            if blocked(p.dir) {
                continue;
            }
            move_solid(
                &self.map,
                &mut solids,
                i,
                p.dir.as_vec2(),
                &mut actors,
                &[],
                &mut Vec::new(),
                &mut Vec::new(),
            );
            p.body = solids[i];
        }
        self.player.body = actors[0];
        let boxes: Vec<Aabb> = solids.iter().map(|s| s.aabb).collect();
        for ((_, e), &body) in enemies.iter_mut().zip(&actors[1..]) {
            e.body = body;
            e.step(&self.map, &boxes);
        }
        boxes
    }

    /// Твёрдые клетки, задевающие вид с целым центром `center`; верхняя — трава.
    fn draw_tiles(&self, gfx: &mut Gfx, center: IVec2) {
        let lo = (center - VIEW / 2).div_euclid(IVec2::splat(TILE));
        let hi = (center + VIEW / 2 - 1).div_euclid(IVec2::splat(TILE));
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let cell = ivec2(x, y);
                if !self.level.solid_at(cell) {
                    continue;
                }
                let art = if self.level.solid_at(cell + IVec2::Y) {
                    DIRT
                } else {
                    GRASS
                };
                gfx.draw_sprite(Sprite {
                    texture: self.tiles,
                    pos: cell_center(cell),
                    size: Vec2::splat(TILE as f32),
                    uv: atlas_uv(art, TILE, TILES_ATLAS),
                    layer: TILE_LAYER,
                    ..Default::default()
                });
            }
        }
    }
}

impl Game for Platformer {
    fn init(ctx: &mut Context) -> Self {
        ctx.gfx.clear_color = Color::from_srgb8(185, 225, 245);
        let level = Level::parse(LEVEL1).unwrap_or_else(|e| panic!("level1.txt, {e}"));
        let mut game = Self::new(level);
        game.tiles = ctx.assets.load_texture(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/tilemap_packed.png"
        ));
        game.chars = ctx.assets.load_texture(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/tilemap-characters_packed.png"
        ));
        game
    }

    fn update(&mut self, ctx: &mut Context) {
        self.tick(&ctx.input);
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        let center = self.camera.lerp(alpha);
        ctx.gfx.camera = Camera2D {
            center,
            size: VIEW.as_vec2(),
        };
        self.draw_tiles(&mut ctx.gfx, center.as_ivec2());
        for (_, p) in self.world.storage::<Platform>().iter() {
            for s in p.sprites(self.tiles, alpha) {
                ctx.gfx.draw_sprite(s);
            }
        }
        for (_, e) in self.world.storage::<Enemy>().iter() {
            ctx.gfx.draw_sprite(e.sprite(self.chars, alpha));
        }
        ctx.gfx.draw_sprite(self.player.sprite(self.chars, alpha));
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Platformer>(Config {
        title: "platformer".into(),
        size: (1280, 720),
        tick_hz: TICK_HZ,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Level {
        Level::parse(text).unwrap()
    }

    #[test]
    fn first_line_is_top_and_hash_is_solid() {
        let l = parse("#..\n.P#\n");
        assert_eq!((l.size, l.player), (ivec2(3, 2), ivec2(1, 0)));
        assert_eq!(l.solid, [false, false, true, true, false, false]);
        assert!(l.solid_at(ivec2(0, 1)) && l.solid_at(ivec2(2, 0)));
        assert!(!l.solid_at(ivec2(1, 0)));
    }

    #[test]
    fn solid_at_outside_map_is_empty() {
        let l = parse("###\n#P#\n");
        for cell in [ivec2(-1, 0), ivec2(3, 0), ivec2(0, -1), ivec2(0, 2)] {
            assert!(!l.solid_at(cell), "{cell}");
        }
    }

    #[test]
    fn spawns_are_cells_from_bottom() {
        let l = parse("E o.^\nX!P  \n");
        assert_eq!(l.enemies, [ivec2(0, 1)]);
        assert_eq!(l.coins, [ivec2(2, 1)]);
        assert_eq!(l.spikes, [ivec2(4, 1)]);
        assert_eq!(l.exits, [ivec2(0, 0)]);
        assert_eq!(l.stops, [ivec2(1, 0)]);
        assert_eq!(l.player, ivec2(2, 0));
        assert!(l.solid.iter().all(|&s| !s));
    }

    #[test]
    fn runs_of_eq_and_h_are_one_platform_each() {
        let l = parse("==.=\n=..H\nP.HH\n");
        let p = |min, size, axis| PlatformSpawn {
            cells: Aabb { min, size },
            axis,
        };
        assert_eq!(
            l.platforms,
            [
                p(ivec2(0, 2), ivec2(2, 1), Axis::X),
                p(ivec2(3, 2), ivec2(1, 1), Axis::X),
                p(ivec2(0, 1), ivec2(1, 1), Axis::X),
                p(ivec2(3, 0), ivec2(1, 2), Axis::Y),
                p(ivec2(2, 0), ivec2(1, 1), Axis::Y),
            ]
        );
    }

    #[test]
    fn h_run_starts_at_top_row() {
        let l = parse("H\nH\nP\n");
        assert_eq!(l.platforms[0].cells.min, ivec2(0, 1));
        assert_eq!(l.platforms[0].cells.size, ivec2(1, 2));
    }

    #[test]
    fn parse_errors_name_the_line() {
        assert_eq!(
            Level::parse("P..\n..\n"),
            Err(LevelError::Ragged { line: 2 })
        );
        assert_eq!(
            Level::parse("P..\n.?.\n"),
            Err(LevelError::Unknown { line: 2, ch: '?' })
        );
        assert_eq!(Level::parse("...\n###\n"), Err(LevelError::NoPlayer));
        assert_eq!(Level::parse(""), Err(LevelError::NoPlayer));
        assert_eq!(
            Level::parse("P..\n..P\n"),
            Err(LevelError::TwoPlayers { line: 2 })
        );
    }

    #[test]
    fn errors_print_line_and_cause() {
        let msg = |text| Level::parse(text).unwrap_err().to_string();
        assert_eq!(msg("P..\n..\n"), "строка 2: длина не как у первой");
        assert_eq!(msg("P..\n.?.\n"), "строка 2: неизвестный символ '?'");
        assert_eq!(msg(""), "нет P");
        assert_eq!(msg("P\nP\n"), "строка 2: второй P");
    }

    #[test]
    fn level1_is_at_least_four_screens_wide() {
        let l = parse(LEVEL1);
        assert!(l.bounds().x >= 4 * VIEW.x, "{}", l.bounds());
        assert!(l.bounds().y >= VIEW.y, "{}", l.bounds());
        assert!(!l.exits.is_empty() && !l.platforms.is_empty());
    }

    const BOUNDS: IVec2 = ivec2(1000, 400);

    #[test]
    fn camera_ignores_target_inside_dead_zone() {
        let mut c = Follow::at(ivec2(500, 200));
        c.follow(ivec2(500, 200) + DEAD_ZONE, BOUNDS);
        c.follow(ivec2(500, 200) - DEAD_ZONE, BOUNDS);
        assert_eq!(c.pos, ivec2(500, 200));
    }

    #[test]
    fn camera_drags_target_on_dead_zone_edge() {
        let mut c = Follow::at(ivec2(500, 200));
        c.follow(ivec2(530, 170), BOUNDS);
        assert_eq!(c.pos, ivec2(530, 170) - ivec2(DEAD_ZONE.x, -DEAD_ZONE.y));
        c.follow(ivec2(400, 250), BOUNDS);
        assert_eq!(c.pos, ivec2(400, 250) + ivec2(DEAD_ZONE.x, -DEAD_ZONE.y));
    }

    #[test]
    fn camera_view_stays_inside_level() {
        let mut c = Follow::at(ivec2(500, 200));
        c.follow(ivec2(0, 0), BOUNDS);
        assert_eq!(c.pos, VIEW / 2);
        c.follow(BOUNDS, BOUNDS);
        assert_eq!(c.pos, BOUNDS - VIEW / 2);
        let mut small = Follow::at(ivec2(500, 200));
        small.follow(ivec2(500, 200), ivec2(100, 100));
        assert_eq!(small.pos, VIEW / 2);
    }

    #[test]
    fn camera_lerp_rounds_to_whole_pixels() {
        let c = Follow {
            pos: ivec2(3, -3),
            prev: IVec2::ZERO,
        };
        assert_eq!(c.lerp(0.0), Vec2::ZERO);
        assert_eq!(c.lerp(0.3), vec2(1.0, -1.0));
        assert_eq!(c.lerp(0.5), vec2(2.0, -2.0));
        assert_eq!(c.lerp(1.0), vec2(3.0, -3.0));
    }

    fn started() -> (Platformer, Context) {
        let mut ctx = Context::default();
        let g = Platformer::init(&mut ctx);
        (g, ctx)
    }

    #[test]
    fn init_puts_spawn_in_view_and_loads_tiles() {
        let (g, mut ctx) = started();
        let pending = ctx.assets.take_pending();
        let tiles = pending.iter().find(|p| p.handle == g.tiles).unwrap();
        assert_eq!(vec2(tiles.width as f32, tiles.height as f32), TILES_ATLAS);
        let spawn = cell_center(g.level.player).as_ivec2();
        assert!((spawn - g.camera.pos).abs().cmplt(VIEW / 2).all());
        assert_eq!(g.camera.pos, VIEW / 2);
        assert_eq!(g.camera.prev, g.camera.pos);
        assert_ne!(g.tiles, Handle::WHITE);
        assert_ne!(ctx.gfx.clear_color, Color::default());
    }

    #[test]
    fn new_camera_starts_still_on_spawn() {
        let g = Platformer::new(parse(&format!("{}P{}\n", ".".repeat(50), ".".repeat(149))));
        assert_eq!(g.player.center().x, 909);
        assert_eq!(g.camera.pos, ivec2(909 + DEAD_ZONE.x, VIEW.y / 2));
        assert_eq!(g.camera.prev, g.camera.pos);
    }

    #[test]
    fn update_saves_prev_then_follows() {
        let (mut g, mut ctx) = started();
        g.camera.pos = ivec2(900, 300);
        g.update(&mut ctx);
        assert_eq!(g.camera.prev, ivec2(900, 300));
        assert_eq!(g.camera.pos, Platformer::new(parse(LEVEL1)).camera.pos);
    }

    fn tile_sprites(gfx: &Gfx) -> Vec<Sprite> {
        gfx.sprites
            .iter()
            .filter(|s| s.layer == TILE_LAYER)
            .copied()
            .collect()
    }

    /// 40×12 клеток: пол в две строки и колонна из трёх клеток у `x = 20`.
    fn strip() -> Platformer {
        let mut rows = vec![".".repeat(40); 12];
        rows[9].replace_range(20..21, "#");
        rows[8].replace_range(20..21, "#");
        rows[7].replace_range(20..21, "#");
        rows[10] = "#".repeat(40);
        rows[11] = "#".repeat(40);
        rows[0].replace_range(0..1, "P");
        let mut g = Platformer::new(parse(&rows.join("\n")));
        g.tiles = Handle::WHITE;
        g
    }

    #[test]
    fn draws_only_tiles_touching_view() {
        let mut g = strip();
        let mut gfx = Gfx::default();
        g.camera = Follow::at(ivec2(169, 90));
        g.draw_tiles(&mut gfx, g.camera.pos);
        let xs: Vec<f32> = tile_sprites(&gfx).iter().map(|s| s.pos.x).collect();
        // Вид [9, 329): клетки 0..=18 по X, пол — две строки.
        assert_eq!(xs.len(), 2 * 19);
        assert_eq!(xs.iter().copied().fold(f32::MAX, f32::min), 9.0);
        assert_eq!(
            xs.iter().copied().fold(f32::MIN, f32::max),
            18.0 * 18.0 + 9.0
        );
    }

    #[test]
    fn view_edge_on_tile_border_skips_next_tile() {
        let mut g = strip();
        let mut gfx = Gfx::default();
        g.camera = Follow::at(ivec2(200, 125));
        g.draw_tiles(&mut gfx, g.camera.pos);
        // Вид [40, 360) × [35, 215): клетки 2..=19 верхней строки пола, колонна у x = 20 — нет.
        let ys: Vec<f32> = tile_sprites(&gfx).iter().map(|s| s.pos.y).collect();
        assert_eq!(ys, [27.0; 18]);
    }

    #[test]
    fn top_tile_is_grass_and_below_is_dirt() {
        let mut g = strip();
        let mut gfx = Gfx::default();
        g.camera = Follow::at(ivec2(360, 90));
        g.draw_tiles(&mut gfx, g.camera.pos);
        let uv_at = |pos: Vec2| {
            tile_sprites(&gfx)
                .into_iter()
                .find(|s| s.pos == pos)
                .unwrap()
                .uv
        };
        let grass = atlas_uv(GRASS, TILE, TILES_ATLAS);
        let dirt = atlas_uv(DIRT, TILE, TILES_ATLAS);
        assert_eq!(uv_at(cell_center(ivec2(20, 4))), grass);
        assert_eq!(uv_at(cell_center(ivec2(20, 3))), dirt);
        assert_eq!(uv_at(cell_center(ivec2(20, 1))), dirt);
        assert_eq!(uv_at(cell_center(ivec2(19, 1))), grass);
        assert_eq!(uv_at(cell_center(ivec2(19, 0))), dirt);
    }

    #[test]
    fn atlas_uv_covers_one_cell() {
        let uv = atlas_uv(ivec2(2, 6), TILE, TILES_ATLAS);
        assert_eq!(uv.min, vec2(36.0 / 360.0, 108.0 / 162.0));
        assert_eq!(uv.max, vec2(54.0 / 360.0, 126.0 / 162.0));
        let chars = atlas_uv(ivec2(1, 2), 24, vec2(216.0, 72.0));
        assert_eq!(chars.min, vec2(24.0 / 216.0, 48.0 / 72.0));
        assert_eq!(chars.max, vec2(48.0 / 216.0, 1.0));
    }

    #[test]
    fn draw_sets_camera_and_tile_edges_on_whole_pixels() {
        let (mut g, mut ctx) = started();
        g.camera = Follow {
            pos: ivec2(400, 100),
            prev: ivec2(397, 99),
        };
        g.draw(&mut ctx, 0.4);
        assert_eq!(ctx.gfx.camera.center, vec2(398.0, 99.0));
        assert_eq!(ctx.gfx.camera.size, VIEW.as_vec2());
        let tiles = tile_sprites(&ctx.gfx);
        assert_ne!(tiles.len(), 0);
        for s in tiles {
            let edge = s.pos - s.size / 2.0;
            assert_eq!(edge, edge.round(), "{s:?}");
            assert_eq!(s.size, Vec2::splat(18.0));
            assert_eq!(s.texture, g.tiles);
        }
    }

    use engine::ElementState::{Pressed, Released};

    fn input(keys: &[KeyCode]) -> Input {
        let mut i = Input::default();
        for &k in keys {
            i.on_key(k, Pressed, false);
        }
        i
    }

    /// `released` отпущены в этом тике, `held` удерживаются.
    fn release(released: &[KeyCode], held: &[KeyCode]) -> Input {
        let mut i = input(&[released, held].concat());
        i.end_tick();
        for &k in released {
            i.on_key(k, Released, false);
        }
        i
    }

    fn room(rows: &[&str]) -> Platformer {
        Platformer::new(parse(&rows.join("\n")))
    }

    /// Игрок стоит на полу, `min = (39, 18)`.
    fn flat() -> Platformer {
        room(&[
            "............",
            "............",
            "............",
            "............",
            "..P.........",
            "############",
        ])
    }

    /// Пол обрывается на `x = 90`, под обрывом — яма.
    fn ledge() -> Platformer {
        room(&[
            "............",
            "............",
            "...P........",
            "#####.......",
        ])
    }

    /// Игрок падает на пол с высоты 180 px.
    fn drop() -> Platformer {
        let mut rows = vec![".........."; 11];
        rows[0] = "..P.......";
        rows.push("##########");
        room(&rows)
    }

    fn on_ground(g: &Platformer) -> bool {
        grounded(&g.map, &[], &g.player.body.aabb)
    }

    const D: &[KeyCode] = &[KeyCode::KeyD];
    const SPACE: &[KeyCode] = &[KeyCode::Space];

    #[test]
    fn controls_map_keys() {
        let read = |keys: &[KeyCode]| Controls::read(&input(keys));
        assert_eq!(read(&[KeyCode::KeyD]).run, 1);
        assert_eq!(read(&[KeyCode::ArrowRight]).run, 1);
        assert_eq!(read(&[KeyCode::KeyA]).run, -1);
        assert_eq!(read(&[KeyCode::ArrowLeft]).run, -1);
        assert_eq!(read(&[KeyCode::KeyA, KeyCode::KeyD]).run, 0);
        assert_eq!(read(&[]), Controls::default());
        assert!(read(&[KeyCode::Space]).jump && read(&[KeyCode::KeyW]).jump);
        let mut held = input(&[KeyCode::Space]);
        held.end_tick();
        assert_eq!(Controls::read(&held), Controls::default());
    }

    #[test]
    fn release_needs_all_jump_keys_up() {
        let read =
            |released: &[KeyCode], held: &[KeyCode]| Controls::read(&release(released, held));
        assert!(read(&[KeyCode::Space], &[]).release);
        assert!(read(&[KeyCode::KeyW], &[]).release);
        assert!(!read(&[KeyCode::Space], &[KeyCode::KeyW]).release);
        assert!(!read(&[KeyCode::KeyD], &[]).release);
    }

    #[test]
    fn spawn_stands_centered_on_cell_bottom() {
        let g = flat();
        assert_eq!(g.player.body.aabb.min, ivec2(39, 18));
        assert_eq!(g.player.body.aabb.size, PLAYER_SIZE);
        assert_eq!(g.player.prev, ivec2(39, 18));
        assert_eq!(g.player.center(), ivec2(45, 26));
    }

    #[test]
    fn run_accelerates_to_limit_and_stops() {
        let mut g = flat();
        g.tick(&input(D));
        assert_eq!(g.player.vel.x, RUN_ACCEL);
        for _ in 0..30 {
            g.tick(&input(D));
        }
        assert_eq!(g.player.vel.x, RUN_MAX);
        assert!(g.player.body.aabb.min.x > 60, "{}", g.player.body.aabb.min);
        g.tick(&Input::default());
        assert_eq!(g.player.vel.x, RUN_MAX - RUN_ACCEL);
        for _ in 0..20 {
            g.tick(&Input::default());
        }
        assert_eq!(g.player.vel.x, 0.0);
        g.tick(&input(&[KeyCode::KeyA]));
        assert_eq!(g.player.vel.x, -RUN_ACCEL);
    }

    #[test]
    fn standing_player_stays_on_floor() {
        let mut g = flat();
        for _ in 0..60 {
            g.tick(&Input::default());
            assert_eq!(g.player.body.aabb.min, ivec2(39, 18));
        }
        assert_eq!(g.sfx, []);
    }

    #[test]
    fn wall_stops_run() {
        let mut g = flat();
        for _ in 0..60 {
            g.tick(&input(&[KeyCode::KeyA]));
        }
        assert_eq!(g.player.body.aabb.min.x, 0);
        assert!(g.player.vel.x > -0.5, "{}", g.player.vel.x);
    }

    #[test]
    fn jump_from_ground_sounds_jump() {
        let mut g = flat();
        g.tick(&input(SPACE));
        assert_eq!(g.player.vel.y, JUMP);
        assert_eq!(g.player.body.aabb.min.y, 23);
        assert_eq!(g.sfx, [Sfx::Jump]);
        g.tick(&Input::default());
        assert_eq!(g.sfx, []);
        assert_eq!(g.player.vel.y, JUMP - GRAVITY);
    }

    #[test]
    fn no_jump_in_air_after_jump() {
        let mut g = flat();
        g.tick(&input(SPACE));
        g.tick(&Input::default());
        g.tick(&input(SPACE));
        assert_eq!(g.sfx, []);
        assert_eq!(g.player.vel.y, JUMP - 2.0 * GRAVITY);
    }

    /// Тик 1 после края — первый, начатый не на земле.
    fn jumps_on_tick_after_edge(n: u32) -> bool {
        let mut g = ledge();
        for _ in 0..100 {
            if !on_ground(&g) {
                break;
            }
            g.tick(&input(D));
        }
        for _ in 1..n {
            g.tick(&Input::default());
        }
        g.tick(&input(SPACE));
        g.sfx == [Sfx::Jump]
    }

    #[test]
    fn coyote_jump_lasts_six_ticks() {
        assert!(jumps_on_tick_after_edge(1));
        assert!(jumps_on_tick_after_edge(COYOTE));
        assert!(!jumps_on_tick_after_edge(COYOTE + 1));
    }

    /// Прыжок нажат за `n` тиков до тика, начатого на земле.
    fn jumps_when_pressed_before_landing(n: u32) -> bool {
        let mut fall = drop();
        let land = (0..100)
            .find(|_| {
                let landed = on_ground(&fall);
                fall.tick(&Input::default());
                landed
            })
            .unwrap();
        let mut g = drop();
        for t in 0..land {
            g.tick(&input(if t == land - n { SPACE } else { &[] }));
            assert_eq!(g.sfx, []);
        }
        g.tick(&Input::default());
        g.sfx == [Sfx::Jump]
    }

    #[test]
    fn jump_buffer_lasts_six_ticks() {
        assert!(jumps_when_pressed_before_landing(1));
        assert!(jumps_when_pressed_before_landing(JUMP_BUFFER));
        assert!(!jumps_when_pressed_before_landing(JUMP_BUFFER + 1));
    }

    #[test]
    fn fall_is_capped_and_lands_on_floor() {
        let mut g = drop();
        for _ in 0..25 {
            g.tick(&Input::default());
        }
        assert_eq!(g.player.vel.y, -FALL_MAX);
        for _ in 0..60 {
            g.tick(&Input::default());
        }
        assert_eq!(g.player.body.aabb.min.y, 18);
        assert!(g.player.vel.y > -FALL_MAX);
    }

    #[test]
    fn release_cuts_rise_only() {
        let mut g = flat();
        g.tick(&input(SPACE));
        g.tick(&release(SPACE, &[]));
        assert_eq!(g.player.vel.y, (JUMP - GRAVITY) * JUMP_CUT);
        let mut g = drop();
        for _ in 0..3 {
            g.tick(&Input::default());
        }
        g.tick(&release(SPACE, &[]));
        assert_eq!(g.player.vel.y, -4.0 * GRAVITY);
    }

    #[test]
    fn ceiling_bump_zeroes_rise() {
        let mut g = room(&["..#.", "..P.", "####"]);
        g.tick(&input(SPACE));
        assert_eq!(g.player.body.aabb.min.y, 20);
        assert_eq!(g.player.vel.y, 0.0);
    }

    #[test]
    fn quick_landing_does_not_reuse_jump_press() {
        let mut g = room(&["..#.", "..P.", "####"]);
        g.tick(&input(SPACE));
        for _ in 0..JUMP_BUFFER {
            g.tick(&Input::default());
            assert_eq!(g.sfx, []);
        }
        assert_eq!(g.player.body.aabb.min.y, 18);
    }

    #[test]
    fn tick_saves_prev_and_camera_follows_player() {
        let mut g = flat();
        g.player.body.aabb.min = ivec2(100, 18);
        g.tick(&input(D));
        assert_eq!(g.player.prev, ivec2(100, 18));
        assert_eq!(g.camera.pos, ivec2(106 + DEAD_ZONE.x, 0).max(VIEW / 2));
    }

    #[test]
    fn update_reads_context_input() {
        let (mut g, mut ctx) = started();
        ctx.input.on_key(KeyCode::KeyD, Pressed, false);
        g.update(&mut ctx);
        assert_eq!(g.player.vel.x, RUN_ACCEL);
    }

    #[test]
    fn run_frames_alternate_on_ground() {
        let mut p = Player::spawn(IVec2::ZERO);
        p.air = 0;
        assert_eq!(p.frame(), IDLE);
        p.run_ticks = RUN_FRAME - 1;
        assert_eq!(p.frame(), IDLE);
        p.run_ticks = RUN_FRAME;
        assert_eq!(p.frame(), WALK);
        p.run_ticks = 2 * RUN_FRAME;
        assert_eq!(p.frame(), IDLE);
        p.air = 1;
        assert_eq!(p.frame(), WALK);
    }

    #[test]
    fn run_ticks_count_only_on_ground() {
        let mut g = flat();
        for _ in 0..10 {
            g.tick(&input(D));
        }
        assert_eq!(g.player.run_ticks, 10);
        g.tick(&input(&[KeyCode::KeyD, KeyCode::Space]));
        g.tick(&input(D));
        assert_eq!(g.player.run_ticks, 0);
        let mut g = flat();
        g.tick(&Input::default());
        assert_eq!(g.player.run_ticks, 0);
    }

    #[test]
    fn sprite_faces_run_direction() {
        let mut g = flat();
        let uv = atlas_uv(IDLE, CHAR, CHARS_ATLAS);
        let flipped = Rect {
            min: vec2(uv.max.x, uv.min.y),
            max: vec2(uv.min.x, uv.max.y),
        };
        assert_eq!(g.player.sprite(Handle::WHITE, 0.0).uv, flipped);
        g.tick(&input(&[KeyCode::KeyA]));
        g.tick(&Input::default());
        assert_eq!(g.player.facing, -1);
        assert_eq!(g.player.sprite(Handle::WHITE, 0.0).uv, uv);
        g.tick(&input(D));
        assert_eq!(g.player.facing, 1);
    }

    #[test]
    fn sprite_interpolates_to_whole_pixels() {
        let mut p = Player::spawn(IVec2::ZERO);
        p.prev = IVec2::ZERO;
        p.body.aabb.min = ivec2(3, -3);
        let s = p.sprite(Handle::WHITE, 0.3);
        assert_eq!(s.pos, vec2(1.0 + 6.0, -1.0 + 12.0));
        assert_eq!(s.size, Vec2::splat(24.0));
        assert_eq!(s.layer, PLAYER_LAYER);
        assert_eq!(p.sprite(Handle::WHITE, 1.0).pos, vec2(9.0, 9.0));
    }

    #[test]
    fn draw_shows_player_with_characters_atlas() {
        let (mut g, mut ctx) = started();
        let pending = ctx.assets.take_pending();
        let chars = pending.iter().find(|p| p.handle == g.chars).unwrap();
        assert_eq!(vec2(chars.width as f32, chars.height as f32), CHARS_ATLAS);
        g.draw(&mut ctx, 1.0);
        let players: Vec<_> = ctx
            .gfx
            .sprites
            .iter()
            .filter(|s| s.layer == PLAYER_LAYER)
            .collect();
        assert_eq!(players.len(), 1);
        assert_eq!(players[0].texture, g.chars);
        assert_ne!(g.chars, g.tiles);
        let bottom = players[0].pos.y - 12.0;
        assert_eq!(bottom, g.player.body.aabb.min.y as f32);
    }

    fn platforms(g: &Platformer) -> Vec<Aabb> {
        let store = g.world.storage::<Platform>();
        store.iter().map(|(_, p)| p.body.aabb).collect()
    }

    fn enemies(g: &Platformer) -> Vec<Aabb> {
        let store = g.world.storage::<Enemy>();
        store.iter().map(|(_, e)| e.body.aabb).collect()
    }

    /// Положения первой платформы после каждого из `n` тиков без ввода.
    fn run(g: &mut Platformer, n: usize) -> Vec<Aabb> {
        (0..n)
            .map(|_| {
                g.tick(&Input::default());
                platforms(g)[0]
            })
            .collect()
    }

    #[test]
    fn level_spawns_platforms_and_enemies_in_world() {
        let g = room(&["E.==.H", "P.####"]);
        let rect = |x, y, w, h| Aabb {
            min: ivec2(x, y),
            size: ivec2(w, h),
        };
        assert_eq!(platforms(&g), [rect(36, 18, 36, 18), rect(90, 18, 18, 18)]);
        assert_eq!(enemies(&g), [rect(2, 18, 14, 12)]);
        let store = g.world.storage::<Platform>();
        let dirs: Vec<IVec2> = store.iter().map(|(_, p)| p.dir).collect();
        assert_eq!(dirs, [IVec2::X, IVec2::Y]);
    }

    #[test]
    fn platform_turns_before_stop_and_solid() {
        let mut g = room(&["P......", "!..=..#"]);
        let xs: Vec<i32> = run(&mut g, 150).iter().map(|p| p.min.x).collect();
        assert_eq!(xs[..3], [55, 56, 57]);
        assert_eq!(xs.iter().max(), Some(&90));
        assert_eq!(xs.iter().min(), Some(&18));
        let turn = xs.iter().position(|&x| x == 90).unwrap();
        assert_eq!(xs[turn + 1], 89);
    }

    #[test]
    fn vertical_platform_turns_before_stop_and_solid() {
        let mut g = room(&["!P", "..", "..", "H.", "#."]);
        let ys: Vec<i32> = run(&mut g, 100).iter().map(|p| p.min.y).collect();
        assert_eq!(ys[0], 19);
        assert_eq!(ys.iter().max(), Some(&54));
        assert_eq!(ys.iter().min(), Some(&18));
    }

    /// Платформа `=` клетки `(1, 0)` ходит между `!` по краям, над ней — пусто.
    fn ferry() -> Platformer {
        room(&["P........", ".........", "!=......!"])
    }

    #[test]
    fn player_rides_platform_both_ways() {
        let mut g = ferry();
        g.player.body.aabb.min = ivec2(20, 18);
        let mut seen = Vec::new();
        for _ in 0..200 {
            g.tick(&Input::default());
            let p = platforms(&g)[0];
            assert_eq!(g.player.body.aabb.min - p.min, ivec2(2, 18));
            seen.push(p.min.x);
        }
        assert!(seen.contains(&(7 * TILE)) && seen.contains(&(TILE + 1)));
    }

    #[test]
    fn player_rides_vertical_platform_both_ways() {
        let mut g = room(&["!P", "..", "..", "..", "H.", "#."]);
        g.player.body.aabb.min = ivec2(3, 36);
        let mut seen = Vec::new();
        for _ in 0..200 {
            g.tick(&Input::default());
            let p = platforms(&g)[0];
            assert_eq!(g.player.body.aabb.min - p.min, ivec2(3, 18));
            seen.push(p.min.y);
        }
        assert!(seen.contains(&TILE) && seen.contains(&(4 * TILE)));
    }

    #[test]
    fn enemy_turns_at_wall() {
        let mut g = room(&["P.......", "#..E...#", "########"]);
        let xs: Vec<i32> = run_enemy(&mut g, 500);
        assert_eq!(xs.iter().min(), Some(&TILE));
        assert_eq!(xs.iter().max(), Some(&(7 * TILE - ENEMY_SIZE.x)));
        assert!(enemies(&g).iter().all(|e| e.min.y == 18));
    }

    fn run_enemy(g: &mut Platformer, n: usize) -> Vec<i32> {
        (0..n)
            .map(|_| {
                g.tick(&Input::default());
                enemies(g)[0].min.x
            })
            .collect()
    }

    #[test]
    fn enemy_walks_half_pixel_per_tick_both_ways() {
        let mut g = room(&["P.......", "...E....", "########"]);
        let start = enemies(&g)[0].min.x;
        assert_eq!(
            run_enemy(&mut g, 4),
            [start - 1, start - 1, start - 2, start - 2]
        );
        g.world
            .storage_mut::<Enemy>()
            .iter_mut()
            .for_each(|(_, e)| e.dir = 1);
        assert_eq!(run_enemy(&mut g, 4), [start - 1, start - 1, start, start]);
    }

    #[test]
    fn enemy_turns_before_tile_edge() {
        let mut g = room(&["P.......", "...E....", "..####.."]);
        let xs = run_enemy(&mut g, 500);
        assert_eq!(xs.iter().min(), Some(&(2 * TILE)));
        assert_eq!(xs.iter().max(), Some(&(6 * TILE - ENEMY_SIZE.x)));
        assert_eq!(enemies(&g)[0].min.y, 18);
    }

    #[test]
    fn enemy_walks_off_edge_only_in_air() {
        let mut g = room(&["P.E", "...", "#.."]);
        let start = enemies(&g)[0].min.x;
        run_enemy(&mut g, 10);
        assert!(enemies(&g)[0].min.y < 36);
        assert!(enemies(&g)[0].min.x < start);
    }

    #[test]
    fn enemy_rides_platform_and_turns_before_its_edges() {
        let mut g = room(&["P.........", "...E......", "!..===...!"]);
        let mut offsets = Vec::new();
        for _ in 0..600 {
            g.tick(&Input::default());
            let (p, e) = (platforms(&g)[0], enemies(&g)[0]);
            assert_eq!(e.min.y, 18);
            offsets.push(e.min.x - p.min.x);
        }
        assert_eq!(offsets.iter().min(), Some(&0));
        assert_eq!(offsets.iter().max(), Some(&(3 * TILE - ENEMY_SIZE.x)));
    }

    #[test]
    fn enemy_turns_before_platform_edge_from_tile() {
        let mut g = room(&["P......", "..E....", "!=##..."]);
        let xs = run_enemy(&mut g, 300);
        assert_eq!(platforms(&g)[0].min.x, TILE);
        assert_eq!(enemies(&g)[0].min.y, 18);
        assert_eq!(xs.iter().max(), Some(&(4 * TILE - ENEMY_SIZE.x)));
        assert_eq!(xs.iter().min(), Some(&TILE));
    }

    #[test]
    fn tick_saves_prev_of_platforms_and_enemies_before_moving() {
        let mut g = ferry();
        g.tick(&Input::default());
        {
            let store = g.world.storage::<Platform>();
            let (_, p) = store.iter().next().unwrap();
            assert_eq!((p.prev, p.body.aabb.min), (ivec2(18, 0), ivec2(19, 0)));
        }
        let mut g = room(&["P.....", "...E..", "######"]);
        g.tick(&Input::default());
        let store = g.world.storage::<Enemy>();
        let (_, e) = store.iter().next().unwrap();
        assert_eq!(e.prev.x, e.body.aabb.min.x + 1);
    }

    #[test]
    fn platform_sprites_cover_cells_with_plank_ends() {
        let g = room(&["P...H", "===.H"]);
        let store = g.world.storage::<Platform>();
        let sprites: Vec<Sprite> = store
            .iter()
            .flat_map(|(_, p)| p.sprites(Handle::WHITE, 0.0).collect::<Vec<_>>())
            .collect();
        let uv = |cell| atlas_uv(cell, TILE, TILES_ATLAS);
        let got: Vec<(Vec2, Rect)> = sprites.iter().map(|s| (s.pos, s.uv)).collect();
        assert_eq!(
            got,
            [
                (vec2(81.0, 9.0), uv(BLOCK)),
                (vec2(81.0, 27.0), uv(BLOCK)),
                (vec2(9.0, 9.0), uv(PLANK_LEFT)),
                (vec2(27.0, 9.0), uv(PLANK)),
                (vec2(45.0, 9.0), uv(PLANK_RIGHT)),
            ]
        );
        for s in sprites {
            assert_eq!((s.size, s.layer), (Vec2::splat(18.0), PLATFORM_LAYER));
        }
    }

    #[test]
    fn platform_sprites_interpolate_to_whole_pixels() {
        let mut g = room(&["P...", "==.."]);
        g.tick(&Input::default());
        g.tick(&Input::default());
        g.tick(&Input::default());
        let store = g.world.storage::<Platform>();
        let (_, p) = store.iter().next().unwrap();
        let first = |alpha| p.sprites(Handle::WHITE, alpha).next().unwrap().pos;
        assert_eq!(first(0.0), vec2(9.0 + 2.0, 9.0));
        assert_eq!(first(0.3), vec2(9.0 + 2.0, 9.0));
        assert_eq!(first(0.6), vec2(9.0 + 3.0, 9.0));
    }

    #[test]
    fn enemy_sprite_faces_walk_and_steps_frames() {
        let mut e = Enemy::spawn(IVec2::ZERO);
        let uv = atlas_uv(WALKER, CHAR, CHARS_ATLAS);
        let s = e.sprite(Handle::WHITE, 1.0);
        assert_eq!(s.uv, uv);
        assert_eq!(s.pos, vec2(2.0 + 7.0, 12.0));
        assert_eq!((s.size, s.layer), (Vec2::splat(24.0), ENEMY_LAYER));
        e.dir = 1;
        let flipped = e.sprite(Handle::WHITE, 1.0).uv;
        assert_eq!((flipped.min.x, flipped.max.x), (uv.max.x, uv.min.x));
        e.dir = -1;
        let step = atlas_uv(WALKER_STEP, CHAR, CHARS_ATLAS);
        let at = |e: &mut Enemy, x| {
            e.body.aabb.min.x = x;
            e.prev.x = x;
            e.sprite(Handle::WHITE, 1.0).uv
        };
        assert_eq!(at(&mut e, 3), uv);
        assert_eq!(at(&mut e, 4), step);
        assert_eq!(at(&mut e, 8), uv);
        assert_eq!(at(&mut e, -1), step);
    }

    #[test]
    fn enemy_sprite_interpolates_to_whole_pixels() {
        let mut e = Enemy::spawn(IVec2::ZERO);
        e.prev = IVec2::ZERO;
        e.body.aabb.min = ivec2(3, -3);
        assert_eq!(
            e.sprite(Handle::WHITE, 0.3).pos,
            vec2(1.0 + 7.0, -1.0 + 12.0)
        );
    }

    #[test]
    fn draw_shows_platforms_and_enemies() {
        let mut g = room(&["P....", "E.==."]);
        let mut ctx = Context::default();
        g.tiles = ctx.assets.load_texture(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/tilemap_packed.png"
        ));
        g.chars = ctx.assets.load_texture(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/tilemap-characters_packed.png"
        ));
        g.draw(&mut ctx, 1.0);
        let on = |layer| -> Vec<Sprite> {
            ctx.gfx
                .sprites
                .iter()
                .filter(|s| s.layer == layer)
                .copied()
                .collect()
        };
        let plats = on(PLATFORM_LAYER);
        assert_eq!(plats.len(), 2);
        assert!(plats.iter().all(|s| s.texture == g.tiles));
        let foes = on(ENEMY_LAYER);
        assert_eq!(foes.len(), 1);
        assert_eq!(foes[0].texture, g.chars);
    }
}
