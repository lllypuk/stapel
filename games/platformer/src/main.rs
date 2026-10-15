mod level;
mod levels;

use std::{
    fs, iter, mem,
    path::{Path, PathBuf},
};

use engine::{
    Aabb, Actor, Assets, Axis, Camera2D, Color, Commands, Config, Contact, Context, Entity, Font,
    FontData, Game, Gfx, Handle, IVec2, Input, KeyCode, Rect, Solid, Sound, SparseSet, Sprite,
    Texture, Tilemap, Vec2, World, grounded, ivec2, move_actor, move_solid, vec2,
};
use level::{Level, PlatformSpawn};

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
/// Монета, шипы и флаг выхода.
const COIN: IVec2 = ivec2(11, 7);
const SPIKES: IVec2 = ivec2(8, 3);
const FLAG: IVec2 = ivec2(11, 5);
const POLE: IVec2 = ivec2(11, 6);
const ITEM_LAYER: i32 = 2;
/// Сенсор шипов у низа клетки.
const SPIKE_SIZE: IVec2 = ivec2(18, 8);
const PLATFORM_LAYER: i32 = 3;
const CHARS_ATLAS: Vec2 = vec2(216.0, 72.0);
/// Сторона кадра персонажа.
const CHAR: i32 = 24;
/// Кадры игрока в атласе персонажей; нарисован лицом влево.
const IDLE: IVec2 = ivec2(0, 0);
const WALK: IVec2 = ivec2(1, 0);
const PLAYER_LAYER: i32 = 5;
/// Кадры врага.
const WALKER: IVec2 = ivec2(0, 2);
const WALKER_STEP: IVec2 = ivec2(1, 2);
const ENEMY_LAYER: i32 = 4;
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
/// Скорость вверх после топтания врага.
const STOMP_BOUNCE: f32 = 3.0;
/// Сколько тиков после края прыжок ещё срабатывает.
const COYOTE: u32 = 6;
/// За сколько тиков до приземления нажатие прыжка ещё срабатывает.
const JUMP_BUFFER: u32 = 6;
/// Тиков на кадр анимации бега.
const RUN_FRAME: u32 = 8;
const LEVELS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/levels");
/// Замороженная копия первого уровня: тесты не зависят от правок `assets/levels/`.
#[cfg(test)]
const LEVEL01: &str = include_str!("testdata/level01.txt");
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
const FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Regular.ttf"
);
/// При `HUD_SCALE` и окне 1280×720 (масштаб 4) тексель атласа — пиксель экрана.
const HUD_PX: f32 = 32.0;
const HUD_SCALE: f32 = 0.25;
const HUD_MARGIN: f32 = 4.0;
const HUD_LAYER: i32 = 6;
const SFX_VOLUME: f32 = 0.8;
const MUSIC_VOLUME: f32 = 0.5;

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
        mem::swap(&mut uv.min.x, &mut uv.max.x);
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

/// Клетка `art` атласа тайлов с центром в `pos`.
fn tile_sprite(texture: Handle<Texture>, pos: Vec2, art: IVec2, layer: i32) -> Sprite {
    Sprite {
        texture,
        pos,
        size: Vec2::splat(TILE as f32),
        uv: atlas_uv(art, TILE, TILES_ATLAS),
        layer,
        ..Default::default()
    }
}

/// Звуковое событие тика.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sfx {
    Jump,
    Coin,
    Stomp,
    Hurt,
    Win,
}

/// Звуки событий.
#[derive(Clone, Copy)]
struct Sounds {
    jump: Handle<Sound>,
    coin: Handle<Sound>,
    stomp: Handle<Sound>,
    hurt: Handle<Sound>,
    win: Handle<Sound>,
}

impl Sounds {
    fn load(assets: &mut Assets) -> Self {
        let mut wav = |name: &str| assets.load_sound(format!("{ASSETS}{name}.wav"));
        Self {
            jump: wav("jump"),
            coin: wav("coin"),
            stomp: wav("stomp"),
            hurt: wav("hurt"),
            win: wav("win"),
        }
    }

    fn sound(&self, sfx: Sfx) -> Handle<Sound> {
        match sfx {
            Sfx::Jump => self.jump,
            Sfx::Coin => self.coin,
            Sfx::Stomp => self.stomp,
            Sfx::Hurt => self.hurt,
            Sfx::Win => self.win,
        }
    }
}

#[derive(Clone, Copy)]
struct Hud {
    font: Handle<Font>,
    big: Handle<Font>,
    ink: Color,
}

/// Ширина строки без переносов; символ вне атласа считается как `'?'`.
fn text_width(font: &FontData, text: &str, scale: f32) -> f32 {
    text.chars()
        .filter_map(|c| font.glyph(c).or_else(|| font.glyph('?')))
        .map(|g| g.advance * scale)
        .sum()
}

/// Время уровня `m:ss` по тикам.
fn clock(ticks: u32) -> String {
    let s = ticks / TICK_HZ;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Что стоит за сенсором уровня.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sensor {
    Coin(Entity),
    Spike,
    Exit,
    Enemy(Entity),
}

/// Монета в клетке.
struct Coin(IVec2);

/// Сенсоры для шага тел и что за каждым стоит; индексы совпадают.
fn sensors(
    level: &Level,
    coins: &SparseSet<Coin>,
    enemies: &SparseSet<Enemy>,
) -> (Vec<Aabb>, Vec<Sensor>) {
    let spike = |&cell: &IVec2| {
        let min = cell * TILE;
        let size = SPIKE_SIZE;
        (Aabb { min, size }, Sensor::Spike)
    };
    coins
        .iter()
        .map(|(e, c)| (cell_aabb(c.0), Sensor::Coin(e)))
        .chain(level.spikes.iter().map(spike))
        .chain(level.exits.iter().map(|&c| (cell_aabb(c), Sensor::Exit)))
        .chain(
            enemies
                .iter()
                .map(|(e, en)| (en.body.aabb, Sensor::Enemy(e))),
        )
        .unzip()
}

/// Вход в сенсор сверху вниз.
fn from_above(c: Contact) -> bool {
    c.axis == Axis::Y && c.dir == -1
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

    fn step(
        &mut self,
        map: &Tilemap,
        solids: &[Aabb],
        c: Controls,
        sensors: &[Aabb],
        contacts: &mut Vec<Contact>,
        sfx: &mut Vec<Sfx>,
    ) {
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
        let hits = move_actor(map, solids, &mut self.body, self.vel, sensors, contacts);
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
                tile_sprite(texture, min + cell_center(ivec2(x, y)), art, PLATFORM_LAYER)
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
    /// Сущности с `Platform`, `Enemy` и `Coin`.
    world: World,
    cmds: Commands,
    camera: Follow,
    coins: u32,
    deaths: u32,
    /// Тики уровня до выхода.
    ticks: u32,
    /// Выход достигнут; мир стоит до Enter.
    cleared: bool,
    /// Enter после выхода; следующий уровень грузит `update`.
    advance: bool,
    /// События тика; звучат в `update`.
    sfx: Vec<Sfx>,
    tiles: Handle<Texture>,
    chars: Handle<Texture>,
    /// До `init` — `None`.
    hud: Option<Hud>,
    sounds: Option<Sounds>,
    levels_dir: PathBuf,
    /// Файл текущего уровня.
    path: PathBuf,
}

impl Platformer {
    fn new(level: Level) -> Self {
        let map = Tilemap::new(TILE, level.size.x, level.size.y, level.solid.clone());
        let player = Player::spawn(level.player);
        let mut world = World::default();
        world.register::<Platform>();
        world.register::<Enemy>();
        world.register::<Coin>();
        for &p in &level.platforms {
            let e = world.spawn();
            world.insert(e, Platform::spawn(p));
        }
        for &cell in &level.enemies {
            let e = world.spawn();
            world.insert(e, Enemy::spawn(cell));
        }
        for &cell in &level.coins {
            let e = world.spawn();
            world.insert(e, Coin(cell));
        }
        let mut camera = Follow::at(level.bounds() / 2);
        camera.follow(player.center(), level.bounds());
        camera.prev = camera.pos;
        Self {
            level,
            map,
            player,
            world,
            cmds: Commands::default(),
            camera,
            coins: 0,
            deaths: 0,
            ticks: 0,
            cleared: false,
            advance: false,
            sfx: Vec::new(),
            tiles: Handle::WHITE,
            chars: Handle::WHITE,
            hud: None,
            sounds: None,
            levels_dir: PathBuf::new(),
            path: PathBuf::new(),
        }
    }

    /// Новый уровень с начала; ассеты и каталог остаются.
    fn load(&mut self, level: Level) {
        *self = Self {
            tiles: self.tiles,
            chars: self.chars,
            hud: self.hud,
            sounds: self.sounds,
            levels_dir: mem::take(&mut self.levels_dir),
            path: mem::take(&mut self.path),
            ..Self::new(level)
        };
    }

    fn restart(&mut self) {
        self.load(self.level.clone());
    }

    /// Следующий по имени уровень каталога; не прочитался или не разобрался — текущий заново.
    fn next_level(&mut self) {
        let next = levels::list(&self.levels_dir)
            .map_err(|e| format!("{}: {e}", self.levels_dir.display()))
            .and_then(|list| {
                let path = levels::next(&list, &self.path)
                    .ok_or_else(|| format!("{}: нет уровней", self.levels_dir.display()))?;
                Ok((path.to_path_buf(), read_level(path)?))
            });
        match next {
            Ok((path, level)) => {
                self.path = path;
                self.load(level);
            }
            Err(e) => {
                log::warn!("{e}");
                self.restart();
            }
        }
    }

    fn tick(&mut self, input: &Input) {
        self.sfx.clear();
        self.save_prev();
        if self.cleared {
            self.advance |= input.just_pressed(KeyCode::Enter);
            return;
        }
        self.ticks += 1;
        let mut hits = Vec::new();
        let (solids, squished) = self.step_bodies(&mut hits);
        let (boxes, kinds) = sensors(
            &self.level,
            &self.world.storage::<Coin>(),
            &self.world.storage::<Enemy>(),
        );
        let mut contacts = Vec::new();
        self.player.step(
            &self.map,
            &solids,
            Controls::read(input),
            &boxes,
            &mut contacts,
            &mut self.sfx,
        );
        hits.extend(contacts.iter().map(|&c| (kinds[c.sensor], c)));
        let died = self.resolve(&hits, squished);
        self.camera
            .follow(self.player.center(), self.level.bounds());
        if died {
            self.camera.prev = self.camera.pos;
        }
        self.cmds.apply(&mut self.world);
    }

    /// Положения на начало тика — один раз, до всех перемещений.
    fn save_prev(&mut self) {
        self.camera.prev = self.camera.pos;
        self.player.prev = self.player.body.aabb.min;
        for (_, p) in self.world.storage_mut::<Platform>().iter_mut() {
            p.prev = p.body.aabb.min;
        }
        for (_, e) in self.world.storage_mut::<Enemy>().iter_mut() {
            e.prev = e.body.aabb.min;
        }
    }

    /// Платформы везут и толкают игрока и врагов, затем ходят враги; входы игрока в сенсоры — в `hits`.
    /// Отдаёт платформы для шага игрока и раздавлен ли он; раздавленные враги — на удаление.
    fn step_bodies(&mut self, hits: &mut Vec<(Sensor, Contact)>) -> (Vec<Aabb>, bool) {
        let mut platforms = self.world.storage_mut::<Platform>();
        let mut enemies = self.world.storage_mut::<Enemy>();
        let (boxes, kinds) = sensors(&self.level, &self.world.storage::<Coin>(), &enemies);
        let mut solids: Vec<Solid> = platforms.iter().map(|(_, p)| p.body).collect();
        let mut actors: Vec<Actor> = iter::once(self.player.body)
            .chain(enemies.iter().map(|(_, e)| e.body))
            .collect();
        let mut contacts = Vec::new();
        let mut squished = Vec::new();
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
                &boxes,
                &mut contacts,
                &mut squished,
            );
            p.body = solids[i];
        }
        hits.extend(
            contacts
                .iter()
                .filter(|&&(j, _)| j == 0)
                .map(|&(_, c)| (kinds[c.sensor], c)),
        );
        self.player.body = actors[0];
        let boxes: Vec<Aabb> = solids.iter().map(|s| s.aabb).collect();
        for (j, ((e, en), &body)) in enemies.iter_mut().zip(&actors[1..]).enumerate() {
            en.body = body;
            if squished.contains(&(j + 1)) {
                self.cmds.despawn(e);
                continue;
            }
            en.step(&self.map, &boxes);
        }
        (boxes, squished.contains(&0))
    }

    /// Последствия тика для игрока; `true` — погиб и возрождён на старте.
    /// Смерть отменяет топтания, монеты и выход.
    fn resolve(&mut self, hits: &[(Sensor, Contact)], squished: bool) -> bool {
        let aabb = self.player.body.aabb;
        let live = |e: Entity| !self.cmds.is_pending(e);
        let hurt = hits.iter().any(|&(s, c)| match s {
            Sensor::Spike => true,
            Sensor::Enemy(e) => live(e) && !from_above(c),
            Sensor::Coin(_) | Sensor::Exit => false,
        });
        // Без `hurt` каждый вход в живого врага — сверху.
        let stomped: Vec<Entity> = hits
            .iter()
            .filter_map(|&(s, _)| match s {
                Sensor::Enemy(e) => Some(e),
                _ => None,
            })
            .collect();
        // Сенсор не ловит врага, вошедшего в стоящего игрока.
        let inside = self
            .world
            .storage::<Enemy>()
            .iter()
            .any(|(e, en)| live(e) && !stomped.contains(&e) && en.body.aabb.overlaps(&aabb));
        if squished || hurt || inside || aabb.max().y <= 0 {
            self.player = Player::spawn(self.level.player);
            self.deaths += 1;
            self.sfx.push(Sfx::Hurt);
            return true;
        }
        for e in stomped {
            if !self.cmds.is_pending(e) {
                self.cmds.despawn(e);
                self.player.vel.y = STOMP_BOUNCE;
                self.sfx.push(Sfx::Stomp);
            }
        }
        for &(s, _) in hits {
            match s {
                Sensor::Coin(e) if !self.cmds.is_pending(e) => {
                    self.cmds.despawn(e);
                    self.coins += 1;
                    self.sfx.push(Sfx::Coin);
                }
                Sensor::Exit if !self.cleared => {
                    self.cleared = true;
                    self.sfx.push(Sfx::Win);
                }
                _ => {}
            }
        }
        false
    }

    /// Монеты, шипы и выход: флаг на клетку выше древка.
    fn draw_items(&self, gfx: &mut Gfx) {
        let mut item = |cell, art| {
            gfx.draw_sprite(tile_sprite(self.tiles, cell_center(cell), art, ITEM_LAYER));
        };
        for (_, c) in self.world.storage::<Coin>().iter() {
            item(c.0, COIN);
        }
        for &cell in &self.level.spikes {
            item(cell, SPIKES);
        }
        for &cell in &self.level.exits {
            item(cell, POLE);
            item(cell + IVec2::Y, FLAG);
        }
    }

    /// Счёт в левом верхнем углу вида, после выхода — итоги по центру.
    fn draw_hud(&self, gfx: &mut Gfx, assets: &Assets, center: Vec2) {
        let Some(hud) = self.hud else {
            return;
        };
        let mut text = |font, s: &str, pos| {
            gfx.draw_text(assets, font, s, center + pos, HUD_SCALE, hud.ink, HUD_LAYER);
        };
        let total = self.level.coins.len();
        let coins = format!("COINS {}/{total}", self.coins);
        let deaths = format!("DEATHS {}", self.deaths);
        let time = format!("TIME {}", clock(self.ticks));
        let half = VIEW.as_vec2() / 2.0;
        if !self.cleared {
            let top = half.y - HUD_MARGIN - HUD_PX * HUD_SCALE;
            let status = format!("{coins}\n{deaths}\n{time}");
            text(hud.font, &status, vec2(HUD_MARGIN - half.x, top));
            return;
        }
        let line = assets.font(hud.font).line_height * HUD_SCALE;
        let lines = [
            (hud.big, "LEVEL CLEAR", 2.0 * line),
            (hud.font, &time, 0.0),
            (hud.font, &coins, -line),
            (hud.font, &deaths, -2.0 * line),
            (hud.font, "PRESS ENTER", -4.0 * line),
        ];
        for (font, s, y) in lines {
            let width = text_width(assets.font(font), s, HUD_SCALE);
            text(font, s, vec2(-width / 2.0, y));
        }
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
                gfx.draw_sprite(tile_sprite(self.tiles, cell_center(cell), art, TILE_LAYER));
            }
        }
    }

    /// Первый уровень каталога; уровни лежат в репозитории, поэтому без них — паника с путём.
    fn open(levels_dir: PathBuf) -> Self {
        let path = levels::list(&levels_dir)
            .ok()
            .and_then(|list| list.into_iter().next())
            .unwrap_or_else(|| panic!("{}: нет уровней", levels_dir.display()));
        let level = read_level(&path).unwrap_or_else(|e| panic!("{e}"));
        Self {
            levels_dir,
            path,
            ..Self::new(level)
        }
    }

    /// Текстуры, шрифты, звуки и музыка.
    fn with_assets(mut self, ctx: &mut Context) -> Self {
        ctx.gfx.clear_color = Color::from_srgb8(185, 225, 245);
        self.tiles = ctx.assets.load_texture(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/tilemap_packed.png"
        ));
        self.chars = ctx.assets.load_texture(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/tilemap-characters_packed.png"
        ));
        self.hud = Some(Hud {
            font: ctx.assets.load_font(FONT_PATH, HUD_PX),
            big: ctx.assets.load_font(FONT_PATH, 2.0 * HUD_PX),
            ink: Color::from_srgb8(30, 40, 60),
        });
        self.sounds = Some(Sounds::load(&mut ctx.assets));
        let music = ctx.assets.load_sound(format!("{ASSETS}music.ogg"));
        ctx.audio.play_music(ctx.assets.sound(music), MUSIC_VOLUME);
        self
    }
}

/// Уровень из файла; ошибка — текстом с путём.
fn read_level(path: &Path) -> Result<Level, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Level::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

impl Game for Platformer {
    fn init(ctx: &mut Context) -> Self {
        Self::open(LEVELS_DIR.into()).with_assets(ctx)
    }

    fn update(&mut self, ctx: &mut Context) {
        self.tick(&ctx.input);
        if let Some(sounds) = self.sounds {
            for &sfx in &self.sfx {
                ctx.audio
                    .play(ctx.assets.sound(sounds.sound(sfx)), SFX_VOLUME);
            }
        }
        if self.advance {
            self.next_level();
        }
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        let center = self.camera.lerp(alpha);
        ctx.gfx.camera = Camera2D {
            center,
            size: VIEW.as_vec2(),
        };
        self.draw_tiles(&mut ctx.gfx, center.as_ivec2());
        self.draw_items(&mut ctx.gfx);
        for (_, p) in self.world.storage::<Platform>().iter() {
            for s in p.sprites(self.tiles, alpha) {
                ctx.gfx.draw_sprite(s);
            }
        }
        for (_, e) in self.world.storage::<Enemy>().iter() {
            ctx.gfx.draw_sprite(e.sprite(self.chars, alpha));
        }
        ctx.gfx.draw_sprite(self.player.sprite(self.chars, alpha));
        self.draw_hud(&mut ctx.gfx, &ctx.assets, center);
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
        let g = Platformer::new(parse(LEVEL01)).with_assets(&mut ctx);
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
        assert_eq!(g.camera.pos, Platformer::new(parse(LEVEL01)).camera.pos);
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
        let mut rows = vec![".".repeat(40); 5];
        rows[4].replace_range(2..3, "P");
        rows.push("#".repeat(40));
        let mut g = Platformer::new(parse(&rows.join("\n")));
        g.player.body.aabb.min = ivec2(400, 18);
        g.tick(&input(D));
        assert_eq!(g.player.prev, ivec2(400, 18));
        assert_eq!(g.camera.pos, ivec2(406 - DEAD_ZONE.x, VIEW.y / 2));
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

    fn coins(g: &Platformer) -> Vec<IVec2> {
        let store = g.world.storage::<Coin>();
        store.iter().map(|(_, c)| c.0).collect()
    }

    /// Все звуки за `n` тиков ввода `keys`.
    fn sounds(g: &mut Platformer, keys: &[KeyCode], n: usize) -> Vec<Sfx> {
        (0..n)
            .flat_map(|_| {
                g.tick(&input(keys));
                g.sfx.clone()
            })
            .collect()
    }

    fn hit(sensor: Sensor, axis: Axis, dir: i32) -> (Sensor, Contact) {
        let c = Contact {
            sensor: 0,
            axis,
            dir,
        };
        (sensor, c)
    }

    fn first<T: 'static>(g: &Platformer) -> Entity {
        g.world.storage::<T>().iter().next().unwrap().0
    }

    #[test]
    fn coin_is_taken_once_on_touch() {
        let mut g = room(&["......", ".Po...", "######"]);
        let sfx = sounds(&mut g, D, 40);
        assert!(g.player.body.aabb.min.x > 54, "{}", g.player.body.aabb.min);
        assert_eq!(sfx, [Sfx::Coin]);
        assert_eq!(g.coins, 1);
        assert_eq!(coins(&g), []);
    }

    #[test]
    fn same_coin_twice_in_tick_counts_once() {
        let mut g = room(&["Po", "##"]);
        let c = hit(Sensor::Coin(first::<Coin>(&g)), Axis::X, 1);
        assert!(!g.resolve(&[c, c], false));
        assert_eq!((g.coins, &g.sfx[..]), (1, &[Sfx::Coin][..]));
    }

    /// Вертикальная платформа под клеткой `(3, 2)` поднимается сквозь монету `(3, 4)`.
    fn lift(rider: char) -> Platformer {
        let row = format!("...{rider}...");
        room(&["...!...", "...o...", ".......", &row, "P..H...", "###!###"])
    }

    #[test]
    fn platform_carries_player_into_coin() {
        let mut g = lift('.');
        g.player.body.aabb.min = ivec2(57, 36);
        let sfx = sounds(&mut g, &[], 60);
        assert!(g.player.body.aabb.min.y > 72, "{}", g.player.body.aabb.min);
        assert_eq!(sfx, [Sfx::Coin]);
    }

    #[test]
    fn enemy_carried_into_coin_does_not_take_it() {
        let mut g = lift('E');
        run_enemy(&mut g, 60);
        assert!(enemies(&g)[0].min.y > 72, "{}", enemies(&g)[0].min);
        assert_eq!((g.coins, coins(&g).len()), (0, 1));
    }

    #[test]
    fn spike_kills_and_respawns() {
        let mut g = room(&[".....", "P..^.", "#####"]);
        let sfx = sounds(&mut g, D, 60);
        assert_eq!(&sfx[..1], [Sfx::Hurt]);
        assert_eq!(g.deaths, 1);
        assert!(g.player.body.aabb.max().x < 3 * TILE);
    }

    #[test]
    fn spike_sensor_is_low_strip_of_cell() {
        let g = room(&["P^", "##"]);
        let (boxes, kinds) = sensors(
            &g.level,
            &g.world.storage::<Coin>(),
            &g.world.storage::<Enemy>(),
        );
        assert_eq!(kinds, [Sensor::Spike]);
        let spike = Aabb {
            min: ivec2(18, 18),
            size: ivec2(18, 8),
        };
        assert_eq!(boxes, [spike]);
    }

    /// Враг под игроком: игрок падает на него с высоты.
    fn above_enemy() -> Platformer {
        room(&["..P.", "....", "..E.", "####"])
    }

    #[test]
    fn landing_on_enemy_stomps_and_bounces() {
        let mut g = above_enemy();
        for _ in 0..40 {
            g.tick(&Input::default());
            if g.sfx.contains(&Sfx::Stomp) {
                break;
            }
        }
        assert_eq!(g.sfx, [Sfx::Stomp]);
        assert_eq!(g.player.vel.y, STOMP_BOUNCE);
        assert_eq!((g.deaths, enemies(&g).len()), (0, 0));
    }

    #[test]
    fn enemy_contact_kills_unless_from_above() {
        for (axis, dir) in [(Axis::X, 1), (Axis::X, -1), (Axis::Y, 1)] {
            let mut g = above_enemy();
            let e = first::<Enemy>(&g);
            assert!(g.resolve(&[hit(Sensor::Enemy(e), axis, dir)], false));
            assert_eq!((g.deaths, &g.sfx[..]), (1, &[Sfx::Hurt][..]));
        }
        let mut g = above_enemy();
        let e = first::<Enemy>(&g);
        let stomp = hit(Sensor::Enemy(e), Axis::Y, -1);
        assert!(!g.resolve(&[stomp, stomp], false));
        assert_eq!(g.sfx, [Sfx::Stomp]);
        assert!(g.cmds.is_pending(e));
    }

    #[test]
    fn running_into_enemy_kills() {
        let mut g = room(&["......", "P...E.", "######"]);
        let sfx = sounds(&mut g, D, 40);
        assert_eq!(sfx, [Sfx::Hurt]);
        assert_eq!(enemies(&g).len(), 1);
    }

    #[test]
    fn enemy_walking_into_standing_player_kills() {
        let mut g = room(&["......", "P...E.", "######"]);
        let deadly = (0..200).find(|_| {
            g.tick(&Input::default());
            g.deaths == 1
        });
        assert!(deadly.is_some());
        assert_eq!(enemies(&g)[0].min.x, 14);
    }

    #[test]
    fn crossing_sweeps_with_touching_ends_do_not_kill() {
        let swept = |prev: IVec2, aabb: Aabb| {
            let min = prev.min(aabb.min);
            Aabb {
                min,
                size: aabb.size + (prev - aabb.min).abs(),
            }
        };
        let mut g = flat();
        let mut enemy = Enemy::spawn(ivec2(3, 1));
        enemy.body.aabb.min.x = g.player.body.aabb.max().x + 1;
        enemy.dir = 1;
        let e = g.world.spawn();
        g.world.insert(e, enemy);
        g.player.vel.x = RUN_MAX;
        g.tick(&input(&[KeyCode::ArrowRight]));
        assert_eq!(g.deaths, 0);
        let p = g.player.body.aabb;
        let store = g.world.storage::<Enemy>();
        let en = store.iter().next().unwrap().1;
        assert_eq!(
            (p.min - g.player.prev, en.body.aabb.min - en.prev),
            (ivec2(2, 0), ivec2(1, 0))
        );
        assert!(swept(g.player.prev, p).overlaps(&swept(en.prev, en.body.aabb)));
        assert_eq!(p.max().x, en.body.aabb.min.x);
    }

    #[test]
    fn player_inside_enemy_at_tick_start_dies() {
        let mut g = flat();
        let e = g.world.spawn();
        let mut enemy = Enemy::spawn(ivec2(2, 1));
        enemy.dir = 1;
        g.world.insert(e, enemy);
        g.tick(&Input::default());
        assert_eq!(g.deaths, 1);
    }

    #[test]
    fn stomped_or_squished_enemy_does_not_kill_by_overlap() {
        let mut g = above_enemy();
        let e = first::<Enemy>(&g);
        g.player.body.aabb.min = enemies(&g)[0].min;
        g.cmds.despawn(e);
        assert!(!g.resolve(&[], false));
        let mut g = above_enemy();
        let e = first::<Enemy>(&g);
        g.player.body.aabb.min = enemies(&g)[0].min;
        assert!(!g.resolve(&[hit(Sensor::Enemy(e), Axis::Y, -1)], false));
    }

    #[test]
    fn squished_player_respawns_in_same_tick() {
        let mut g = room(&["......", "#P.=..", "######"]);
        let spawn = g.player.body.aabb.min;
        for _ in 0..200 {
            g.tick(&Input::default());
            if g.deaths == 1 {
                break;
            }
            assert_eq!(g.sfx, []);
        }
        assert_eq!(g.deaths, 1);
        assert_eq!(g.sfx, [Sfx::Hurt]);
        assert_eq!((g.player.body.aabb.min, g.player.prev), (spawn, spawn));
    }

    #[test]
    fn squished_enemy_is_despawned() {
        let mut g = room(&["P.....", "#E.=..", "######"]);
        let gone = (0..300).find(|_| {
            g.tick(&Input::default());
            enemies(&g).is_empty()
        });
        assert!(gone.is_some());
        assert_eq!(g.deaths, 0);
    }

    #[test]
    fn falling_below_level_kills() {
        let mut g = flat();
        g.player.body.aabb.min.y = -15;
        assert!(!g.resolve(&[], false));
        g.player.body.aabb.min.y = -16;
        assert!(g.resolve(&[], false));
        let mut g = room(&["P..", "..."]);
        let sfx = sounds(&mut g, &[], 30);
        assert_eq!(sfx, [Sfx::Hurt]);
    }

    #[test]
    fn death_beats_exit_and_coins() {
        let mut g = room(&["Po", "##"]);
        let coin = Sensor::Coin(first::<Coin>(&g));
        let hits = [
            hit(Sensor::Exit, Axis::X, 1),
            hit(coin, Axis::X, 1),
            hit(Sensor::Spike, Axis::X, 1),
        ];
        assert!(g.resolve(&hits, false));
        assert!(!g.cleared);
        assert_eq!((g.coins, &g.sfx[..]), (0, &[Sfx::Hurt][..]));
        let mut g = room(&["Po", "##"]);
        assert!(g.resolve(&[], true));
    }

    #[test]
    fn exit_clears_level_once_and_freezes_world() {
        let mut g = room(&["......", "P..X..", "######"]);
        let sfx = sounds(&mut g, D, 40);
        assert_eq!(sfx, [Sfx::Win]);
        assert!(g.cleared);
        let mut g = room(&["P.X", "###"]);
        let exit = hit(Sensor::Exit, Axis::X, 1);
        assert!(!g.resolve(&[exit, exit], false));
        assert_eq!(g.sfx, [Sfx::Win]);
    }

    #[test]
    fn cleared_tick_keeps_bodies_still() {
        let mut g = room(&["P.....", "..E...", "!=..!#"]);
        g.tick(&Input::default());
        g.cleared = true;
        let before = (g.player.body.aabb, platforms(&g), enemies(&g));
        g.tick(&input(D));
        assert_eq!((g.player.body.aabb, platforms(&g), enemies(&g)), before);
        assert_eq!(g.player.prev, g.player.body.aabb.min);
        let store = g.world.storage::<Platform>();
        let (_, p) = store.iter().next().unwrap();
        assert_eq!(p.prev, p.body.aabb.min);
        let store = g.world.storage::<Enemy>();
        let (_, e) = store.iter().next().unwrap();
        assert_eq!(e.prev, e.body.aabb.min);
        assert_eq!(g.camera.prev, g.camera.pos);
    }

    #[test]
    fn respawn_resets_motion_and_camera() {
        let mut g = room(&[&format!("P{}^", ".".repeat(40)), &"#".repeat(42)]);
        let fresh = room(&[&format!("P{}^", ".".repeat(40)), &"#".repeat(42)]);
        g.player.body.aabb.min = ivec2(40 * TILE + 5, TILE);
        g.player.body.remainder = vec2(0.3, -0.2);
        g.player.vel = vec2(RUN_MAX, 0.0);
        g.camera = Follow::at(ivec2(40 * TILE, VIEW.y / 2));
        g.tick(&input(D));
        assert_eq!(g.deaths, 1);
        let p = &g.player;
        assert_eq!(
            (p.body.aabb.min, p.prev),
            (fresh.player.prev, fresh.player.prev)
        );
        assert_eq!((p.body.remainder, p.vel), (Vec2::ZERO, Vec2::ZERO));
        assert_eq!(g.camera, fresh.camera);
    }

    #[test]
    fn draws_coins_spikes_and_exit_flag() {
        let mut g = room(&["P.o^X", "#####"]);
        g.tiles = Handle::WHITE;
        let mut gfx = Gfx::default();
        g.draw_items(&mut gfx);
        let uv = |cell| atlas_uv(cell, TILE, TILES_ATLAS);
        let got: Vec<(Vec2, Rect)> = gfx.sprites.iter().map(|s| (s.pos, s.uv)).collect();
        assert_eq!(
            got,
            [
                (cell_center(ivec2(2, 1)), uv(COIN)),
                (cell_center(ivec2(3, 1)), uv(SPIKES)),
                (cell_center(ivec2(4, 1)), uv(POLE)),
                (cell_center(ivec2(4, 2)), uv(FLAG)),
            ]
        );
        assert!(gfx.sprites.iter().all(|s| s.layer == ITEM_LAYER));
        assert!(gfx.sprites.iter().all(|s| s.size == Vec2::splat(18.0)));
        g.world.despawn(first::<Coin>(&g));
        let mut gfx = Gfx::default();
        g.draw_items(&mut gfx);
        assert_eq!(gfx.sprites.len(), 3);
    }

    #[test]
    fn draw_includes_items() {
        let (mut g, mut ctx) = started();
        g.draw(&mut ctx, 1.0);
        let items = ctx.gfx.sprites.iter().filter(|s| s.layer == ITEM_LAYER);
        assert!(items.clone().count() > 0);
        assert!(items.clone().all(|s| s.texture == g.tiles));
    }

    #[test]
    fn each_sfx_has_its_own_sound() {
        let (g, mut ctx) = started();
        let sounds = g.sounds.unwrap();
        let cases = [
            (Sfx::Jump, "jump"),
            (Sfx::Coin, "coin"),
            (Sfx::Stomp, "stomp"),
            (Sfx::Hurt, "hurt"),
            (Sfx::Win, "win"),
        ];
        for (sfx, name) in cases {
            let file = ctx.assets.load_sound(format!("{ASSETS}{name}.wav"));
            assert_eq!(sounds.sound(sfx), file, "{sfx:?}");
        }
    }

    #[test]
    fn clock_shows_whole_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(TICK_HZ - 1), "0:00");
        assert_eq!(clock(61 * TICK_HZ), "1:01");
        assert_eq!(clock(609 * TICK_HZ), "10:09");
    }

    #[test]
    fn time_runs_until_exit() {
        let mut g = room(&["......", "P..X..", "######"]);
        sounds(&mut g, D, 40);
        let t = g.ticks;
        assert!(g.cleared && (1..40).contains(&t), "{t}");
        g.tick(&Input::default());
        assert_eq!(g.ticks, t);
    }

    fn hud(gfx: &Gfx) -> Vec<Sprite> {
        (gfx.sprites.iter())
            .filter(|s| s.layer == HUD_LAYER)
            .copied()
            .collect()
    }

    #[test]
    fn hud_pins_coins_deaths_and_time_to_view_corner() {
        let (mut g, mut ctx) = started();
        (g.coins, g.deaths, g.ticks) = (2, 1, 61 * TICK_HZ + 59);
        let pos = ivec2(500, 90);
        g.camera = Follow { pos, prev: pos };
        g.draw(&mut ctx, 0.0);
        let Hud { font, ink, .. } = g.hud.unwrap();
        let n = g.level.coins.len();
        let status = format!("COINS 2/{n}\nDEATHS 1\nTIME 1:01");
        let font = ctx.assets.font(font);
        let at = vec2(344.0, 168.0);
        let want: Vec<Sprite> =
            engine::text::layout(font, &status, at, HUD_SCALE, ink, HUD_LAYER).collect();
        assert_ne!(want, []);
        assert_eq!(hud(&ctx.gfx), want);
    }

    #[test]
    fn level_clear_centers_totals_and_hint() {
        let (mut g, mut ctx) = started();
        (g.coins, g.deaths, g.ticks, g.cleared) = (3, 4, 5 * TICK_HZ, true);
        let pos = ivec2(500, 90);
        g.camera = Follow { pos, prev: pos };
        g.draw(&mut ctx, 0.0);
        let Hud { font, big, ink } = g.hud.unwrap();
        let line = ctx.assets.font(font).line_height * HUD_SCALE;
        let ratio = ctx.assets.font(big).line_height * HUD_SCALE / line;
        assert!((1.9..2.1).contains(&ratio), "{ratio}");
        let n = g.level.coins.len();
        let coins = format!("COINS 3/{n}");
        let lines = [
            (big, "LEVEL CLEAR", 2.0 * line),
            (font, "TIME 0:05", 0.0),
            (font, &coins, -line),
            (font, "DEATHS 4", -2.0 * line),
            (font, "PRESS ENTER", -4.0 * line),
        ];
        let want: Vec<Sprite> = lines
            .into_iter()
            .flat_map(|(f, s, y)| {
                let f = ctx.assets.font(f);
                let at = vec2(500.0 - text_width(f, s, HUD_SCALE) / 2.0, 90.0 + y);
                engine::text::layout(f, s, at, HUD_SCALE, ink, HUD_LAYER)
            })
            .collect();
        assert_eq!(hud(&ctx.gfx), want);
    }

    #[test]
    fn hud_needs_init() {
        let mut g = flat();
        let mut ctx = Context::default();
        g.draw(&mut ctx, 0.0);
        assert_eq!(hud(&ctx.gfx), []);
    }

    #[test]
    fn text_width_sums_advances() {
        let mut assets = Assets::default();
        let handle = assets.load_font(FONT_PATH, HUD_PX);
        let font = assets.font(handle);
        let one = text_width(font, "0", HUD_SCALE);
        assert_eq!(one, font.glyph('0').unwrap().advance * HUD_SCALE);
        assert!((text_width(font, "0\u{e9}0", HUD_SCALE) - 3.0 * one).abs() < 1e-4);
    }

    const CLEAR: &str = "o.....\nP.oX..\n######\n";
    const SECOND: &str = "P..\n###\n";

    #[test]
    fn enter_after_exit_only_sets_advance() {
        let mut g = room(&["P.X", "###"]);
        sounds(&mut g, D, 40);
        assert!(g.cleared);
        g.tick(&input(&[KeyCode::Space]));
        assert!(!g.advance);
        g.tick(&input(&[KeyCode::Enter]));
        assert!(g.advance && g.cleared);
        g.tick(&Input::default());
        assert!(g.advance);
    }

    /// Пройденный первый уровень каталога `files`, с ассетами.
    fn cleared_in(name: &str, files: &[(&str, &str)]) -> (Platformer, Context) {
        let (art, mut ctx) = started();
        let dir = levels::temp_dir(name, files);
        let mut g = Platformer {
            levels_dir: dir.clone(),
            path: dir.join(files[0].0),
            ..Platformer::new(parse(files[0].1))
        };
        (g.tiles, g.chars, g.hud, g.sounds) = (art.tiles, art.chars, art.hud, art.sounds);
        sounds(&mut g, D, 40);
        g.deaths = 2;
        assert!(g.cleared && g.coins == 1);
        ctx.input = input(&[KeyCode::Enter]);
        (g, ctx)
    }

    #[test]
    fn update_after_enter_loads_next_level_and_keeps_assets() {
        let files = [("level01.txt", CLEAR), ("level02.txt", SECOND)];
        let (mut g, mut ctx) = cleared_in("next", &files);
        let art = (g.tiles, g.chars, g.hud.unwrap().big, g.sounds.unwrap().jump);
        g.update(&mut ctx);
        assert_eq!(g.path, g.levels_dir.join("level02.txt"));
        assert_eq!(g.level, parse(SECOND));
        assert!(!g.cleared && !g.advance);
        assert_eq!((g.coins, g.deaths, g.ticks), (0, 0, 0));
        assert_eq!(g.player.body.aabb, Player::spawn(g.level.player).body.aabb);
        let kept = (g.tiles, g.chars, g.hud.unwrap().big, g.sounds.unwrap().jump);
        assert_eq!(kept, art);
        g.cleared = true;
        g.update(&mut ctx);
        assert_eq!(g.path, g.levels_dir.join("level01.txt"));
        assert_eq!(coins(&g).len(), 2);
    }

    #[test]
    fn unparsable_next_level_restarts_current() {
        let files = [("level01.txt", CLEAR), ("level02.txt", "P?\n")];
        let (mut g, mut ctx) = cleared_in("broken-next", &files);
        g.update(&mut ctx);
        assert_eq!(g.path, g.levels_dir.join("level01.txt"));
        assert_eq!(g.level, parse(CLEAR));
        assert!(!g.cleared && !g.advance);
        assert_eq!((g.coins, g.deaths), (0, 0));
        assert_eq!(coins(&g).len(), 2);
    }

    #[test]
    fn open_takes_first_level_by_name() {
        let dir = levels::temp_dir("first", &[("b.txt", "P\n"), ("a.txt", SECOND)]);
        let g = Platformer::open(dir.clone());
        assert_eq!((&g.path, &g.level), (&dir.join("a.txt"), &parse(SECOND)));
        assert_eq!(g.levels_dir, dir);
    }

    #[test]
    #[should_panic(expected = "нет уровней")]
    fn open_empty_dir_panics() {
        Platformer::open(levels::temp_dir("empty", &[]));
    }

    #[test]
    #[should_panic(expected = "a.txt: строка 1")]
    fn open_unparsable_level_panics_with_path() {
        Platformer::open(levels::temp_dir("first-broken", &[("a.txt", "P?\n")]));
    }

    /// Игрок пересекает твёрдый тайл или платформу.
    fn stuck(g: &Platformer) -> bool {
        let a = g.player.body.aabb;
        g.map.overlaps_solid(&a) || platforms(g).iter().any(|p| p.overlaps(&a))
    }

    /// Скорости в пределах: мутант с растущей скоростью иначе зависает в попиксельном шаге.
    fn speeds_bounded(g: &Platformer) -> bool {
        (-FALL_MAX..=JUMP).contains(&g.player.vel.y)
            && g.world
                .storage::<Enemy>()
                .iter()
                .all(|(_, e)| (-FALL_MAX..=0.0).contains(&e.fall))
    }

    #[test]
    fn rider_stays_on_platform_faster_than_fall() {
        let mut rows = vec![".."; 12];
        rows[0] = "!P";
        rows[10] = "H.";
        rows[11] = "#.";
        let mut g = room(&rows);
        for (_, p) in g.world.storage_mut::<Platform>().iter_mut() {
            p.dir *= 6;
        }
        g.player.body.aabb.min = ivec2(3, 2 * TILE);
        let mut ys = Vec::new();
        for _ in 0..200 {
            g.tick(&Input::default());
            assert!(!stuck(&g), "{:?}", g.player.body.aabb);
            let p = platforms(&g)[0];
            assert_eq!(g.player.body.aabb.min - p.min, ivec2(3, TILE));
            ys.push(p.min.y);
        }
        assert_eq!(
            (ys.iter().min(), ys.iter().max()),
            (Some(&TILE), Some(&180))
        );
    }

    #[test]
    fn rider_on_one_pixel_of_platform_edge_rides_both_ways() {
        for dx in [1 - PLAYER_SIZE.x, TILE - 1] {
            let mut g = ferry();
            g.player.body.aabb.min = ivec2(TILE + dx, TILE);
            for _ in 0..200 {
                g.tick(&Input::default());
                assert!(!stuck(&g), "{:?}", g.player.body.aabb);
                let p = platforms(&g)[0];
                assert_eq!(g.player.body.aabb.min - p.min, ivec2(dx, TILE));
            }
        }
    }

    #[test]
    fn platform_carry_into_wall_stops_rider_alive() {
        let mut g = room(&[
            "P........",
            ".........",
            "......#..",
            "!=......!",
            "#########",
        ]);
        g.player.body.aabb.min = ivec2(TILE + 3, 2 * TILE);
        let mut walled = false;
        for _ in 0..300 {
            g.tick(&Input::default());
            assert!(!stuck(&g), "{:?}", g.player.body.aabb);
            let top = g.player.body.aabb.min.y == 2 * TILE;
            walled |= top && g.player.body.aabb.max().x == 6 * TILE;
        }
        assert!(walled);
        assert_eq!(g.deaths, 0);
    }

    #[test]
    fn jump_into_ceiling_corner_never_overlaps() {
        let mut bumps = 0;
        for x in 60..=110 {
            let mut g = room(&[
                "......####",
                "..........",
                "..........",
                "P.........",
                "##########",
            ]);
            g.player.body.aabb.min = ivec2(x, TILE);
            g.tick(&input(&[KeyCode::KeyD, KeyCode::Space]));
            for _ in 0..40 {
                g.tick(&input(D));
                assert!(!stuck(&g), "x {x}: {:?}", g.player.body.aabb);
                assert!(speeds_bounded(&g), "x {x}: {:?}", g.player.vel);
                if g.player.body.aabb.max().y == 4 * TILE {
                    bumps += 1;
                }
            }
        }
        assert!(bumps > 0);
    }

    #[test]
    fn run_over_tile_seams_keeps_speed_and_slides_down_wall() {
        let mut g = strip();
        g.player.body.aabb.min = ivec2(0, 2 * TILE);
        for _ in 0..40 {
            g.tick(&input(D));
        }
        for _ in 0..150 {
            g.tick(&input(D));
            assert_eq!(g.player.vel.x, RUN_MAX);
            assert_eq!(g.player.body.aabb.min.y, 2 * TILE);
        }
        for _ in 0..100 {
            g.tick(&input(D));
        }
        g.tick(&input(&[KeyCode::KeyD, KeyCode::Space]));
        for _ in 0..60 {
            g.tick(&input(D));
            assert!(!stuck(&g), "{:?}", g.player.body.aabb);
            assert_eq!(g.player.body.aabb.max().x, 20 * TILE);
        }
        assert_eq!(g.player.body.aabb.min.y, 2 * TILE);
    }

    #[test]
    fn spike_catches_fall_at_max_speed() {
        let mut rows = vec!["."; 13];
        rows[0] = "P";
        rows[10] = "^";
        rows[12] = "#";
        let mut g = room(&rows);
        let mut vel = 0.0;
        for _ in 0..100 {
            if g.deaths != 0 {
                break;
            }
            vel = g.player.vel.y;
            g.tick(&Input::default());
        }
        assert_eq!(g.deaths, 1, "{:?}", g.player.body.aabb);
        assert_eq!(vel, -FALL_MAX);
    }

    /// Xorshift64 для случайного прогона.
    struct Xorshift(u64);

    impl Xorshift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn random_play_on_level01_never_overlaps_solids() {
        // Клавиша, шансы 1/n нажать и 1/n отпустить за тик: D чаще A — прогон уходит вглубь уровня.
        const KEYS: [(KeyCode, u64, u64); 4] = [
            (KeyCode::KeyA, 16, 4),
            (KeyCode::KeyD, 4, 16),
            (KeyCode::Space, 6, 6),
            (KeyCode::Enter, 32, 2),
        ];
        // Старты: начало, за шипами после первой ямы, на скале у лифта; со старта сиды не уходят дальше первой ямы.
        let runs = [
            (1, ivec2(3, 2)),
            (0x9e37_79b9_7f4a_7c15, ivec2(42, 2)),
            (0xdead_beef, ivec2(60, 7)),
        ];
        for (seed, start) in runs {
            let mut rng = Xorshift(seed);
            let mut level = parse(LEVEL01);
            level.player = start;
            let mut g = Platformer::new(level);
            let mut keys = Input::default();
            for t in 0..10_000 {
                let r = rng.next();
                for (i, &(k, press, release)) in KEYS.iter().enumerate() {
                    let roll = (r >> (16 * i)) & 0xffff;
                    let (odds, state) = if keys.is_down(k) {
                        (release, Released)
                    } else {
                        (press, Pressed)
                    };
                    if roll.is_multiple_of(odds) {
                        keys.on_key(k, state, false);
                    }
                }
                g.tick(&keys);
                if g.advance {
                    g.restart();
                }
                keys.end_tick();
                assert!(
                    !stuck(&g),
                    "seed {seed}, tick {t}: {:?}",
                    g.player.body.aabb
                );
                assert!(speeds_bounded(&g), "seed {seed}, tick {t}");
            }
        }
    }
}
