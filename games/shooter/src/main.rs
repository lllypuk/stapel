mod enemy;
mod map;
mod obstacle;
mod player;
mod weapon;

use std::{fs, mem};

use engine::{
    Aabb3, Assets, Config, Context, Game, Handle, IVec2, KeyCode, Mat4, Model, MouseButton, Scene,
    Vec2, Vec3,
};

use crate::enemy::Enemy;
use crate::map::Map;
use crate::obstacle::Obstacles;
use crate::player::{MAX_HP, Player, RADIUS, forward};
use crate::weapon::{Arsenal, Impact, Projectile, Shot, Target};

const TICK_HZ: u32 = 60;
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
const LEVEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/levels/level01.txt");
#[cfg(test)]
const LEVEL01: &str = include_str!("../assets/levels/level01.txt");
/// Сторона клетки карты, м.
const CELL: f32 = 2.0;
/// Единица Mini Forest — клетка.
#[cfg_attr(not(test), expect(dead_code, reason = "сцена — в следующих задачах"))]
const FOREST_SCALE: f32 = CELL;
/// Рост Blocky Characters — 2,7 единицы, персонаж — 1,8 м.
#[cfg_attr(not(test), expect(dead_code, reason = "сцена — в следующих задачах"))]
const CHARACTER_SCALE: f32 = 1.8 / 2.7;
/// Здоровье от аптечки.
const MEDKIT: i32 = 25;
/// Патроны пачки по оружиям `Arsenal::weapons`.
const AMMO_PACK: [u32; 2] = [20, 5];
/// Расстояние от центра игрока до центра клетки предмета, с которого он берётся, клетки.
const PICKUP: f32 = 0.5;
/// Игрок с ключом открывает дверь вплотную: упор в неё — ровно `RADIUS`.
const DOOR_REACH: f32 = RADIUS + 0.05;

/// Клипы, которые враги ищут по имени.
#[cfg(test)]
const CLIPS: [&str; 7] = [
    "idle",
    "walk",
    "sprint",
    "die",
    "holding-right-shoot",
    "attack-melee-right",
    "attack-kick-right",
];

#[cfg_attr(not(test), expect(dead_code, reason = "сцена — в следующих задачах"))]
struct Models {
    tree: Handle<Model>,
    tree_high: Handle<Model>,
    rock: Handle<Model>,
    fence: Handle<Model>,
    grass: Handle<Model>,
    brawler: Handle<Model>,
    gunner: Handle<Model>,
    runner: Handle<Model>,
    blaster: Handle<Model>,
    launcher: Handle<Model>,
    bullet: Handle<Model>,
    clip: Handle<Model>,
    crate_: Handle<Model>,
    key: Handle<Model>,
    gate: Handle<Model>,
}

impl Models {
    fn load(assets: &mut Assets) -> Self {
        let mut load = |name: &str| assets.load_model(format!("{ASSETS}{name}.glb"));
        Self {
            tree: load("forest/tree"),
            tree_high: load("forest/tree-high"),
            rock: load("forest/rocks-high"),
            fence: load("forest/fence"),
            grass: load("forest/patch-grass"),
            brawler: load("characters/character-a"),
            gunner: load("characters/character-b"),
            runner: load("characters/character-c"),
            blaster: load("blaster/blaster-a"),
            launcher: load("blaster/blaster-h"),
            bullet: load("blaster/bullet-foam"),
            clip: load("blaster/clip-small"),
            crate_: load("blaster/crate-small"),
            key: load("dungeon/key"),
            gate: load("dungeon/gate"),
        }
    }
}

/// Масштаб `scale` с подъёмом низа габарита на пол: `rocks-high` отцентрован по высоте.
#[cfg_attr(not(test), expect(dead_code, reason = "сцена — в следующих задачах"))]
fn standing(model: &Model, scale: f32) -> Mat4 {
    let lift = -model.bounds().map_or(0.0, |b: Aabb3| b.min.y) * scale;
    Mat4::from_translation(Vec3::Y * lift) * Mat4::from_scale(Vec3::splat(scale))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Door {
    cell: IVec2,
    /// Секунд с открытия ключом.
    opened: Option<f32>,
}

/// Состояние уровня, которое сбрасывает рестарт.
#[derive(Clone, Debug, PartialEq)]
struct Level {
    obstacles: Obstacles,
    player: Player,
    arsenal: Arsenal,
    enemies: Vec<Enemy>,
    projectiles: Vec<Projectile>,
    /// Не взятые.
    keys: Vec<IVec2>,
    /// Ключ в руках; один открывает все двери.
    key: bool,
    doors: Vec<Door>,
    /// Не взятые.
    medkits: Vec<IVec2>,
    /// Не взятые.
    ammo: Vec<IVec2>,
    won: bool,
}

impl Level {
    fn new(map: &Map, models: &Models) -> Self {
        let centre = |cell: IVec2| cell.as_vec2() + 0.5;
        Self {
            obstacles: Obstacles::new(map),
            player: Player::new(centre(map.player), map.facing.yaw()),
            arsenal: Arsenal::new(models.blaster, models.launcher),
            enemies: map
                .enemies
                .iter()
                .map(|&(kind, cell)| Enemy::new(kind, centre(cell)))
                .collect(),
            projectiles: Vec::new(),
            keys: map.keys.clone(),
            key: false,
            doors: map
                .doors
                .iter()
                .map(|&cell| Door { cell, opened: None })
                .collect(),
            medkits: map.medkits.clone(),
            ammo: map.ammo.clone(),
            won: false,
        }
    }

    fn fire(&mut self) {
        let (targets, ids) = targets(&self.enemies);
        let p = &self.player;
        match (self.arsenal).fire(p.pos, forward(p.yaw), &self.obstacles, &targets) {
            Some(Shot::Hit { target, damage }) => self.enemies[ids[target]].hurt(damage),
            Some(Shot::Launch(projectile)) => self.projectiles.push(projectile),
            Some(Shot::Miss) | None => {}
        }
    }

    fn pick_up(&mut self) {
        let pos = self.player.pos;
        let near = |cell: &IVec2| pos.distance(cell.as_vec2() + 0.5) < PICKUP;
        let keys = self.keys.len();
        self.keys.retain(|c| !near(c));
        self.key |= self.keys.len() < keys;
        let hp = &mut self.player.hp;
        if *hp < MAX_HP
            && let Some(i) = self.medkits.iter().position(near)
        {
            self.medkits.remove(i);
            *hp = (*hp + MEDKIT).min(MAX_HP);
        }
        let packs = self.ammo.len();
        self.ammo.retain(|c| !near(c));
        for _ in self.ammo.len()..packs {
            for (w, n) in self.arsenal.weapons.iter_mut().zip(AMMO_PACK) {
                w.ammo += n;
            }
        }
    }

    /// Дверь открывает касание с ключом и пропускает только после `open_secs` анимации.
    fn doors(&mut self, dt: f32, open_secs: f32) {
        let pos = self.player.pos;
        for door in &mut self.doors {
            match &mut door.opened {
                Some(t) => {
                    *t += dt;
                    if *t >= open_secs {
                        self.obstacles.open(door.cell);
                    }
                }
                None if self.key && touches_cell(pos, door.cell) => door.opened = Some(0.0),
                None => {}
            }
        }
    }

    /// Снаряды за тик; попавший пропадает.
    fn fly(&mut self, dt: f32) {
        let player = Target {
            pos: self.player.pos,
            radius: RADIUS,
        };
        let (targets, ids) = targets(&self.enemies);
        let mut projectiles = mem::take(&mut self.projectiles);
        projectiles.retain_mut(|p| match p.step(dt, &self.obstacles, player, &targets) {
            None => true,
            Some(Impact::Wall) => false,
            Some(Impact::Player) => {
                self.player.hurt(p.damage);
                false
            }
            Some(Impact::Enemy(i)) => {
                self.enemies[ids[i]].hurt(p.damage);
                false
            }
        });
        self.projectiles = projectiles;
    }
}

/// Цели живых врагов и их индексы в `enemies`.
fn targets(enemies: &[Enemy]) -> (Vec<Target>, Vec<usize>) {
    enemies
        .iter()
        .enumerate()
        .filter_map(|(i, e)| Some((e.target()?, i)))
        .unzip()
}

/// Центр игрока `pos` у самой клетки `cell`.
fn touches_cell(pos: Vec2, cell: IVec2) -> bool {
    let min = cell.as_vec2();
    pos.distance(pos.clamp(min, min + 1.0)) <= DOOR_REACH
}

struct Shooter {
    models: Models,
    map: Map,
    /// Длительность анимации `open` двери, с.
    open_secs: f32,
    level: Level,
}

impl Shooter {
    fn new(map: Map, assets: &mut Assets) -> Self {
        let models = Models::load(assets);
        let open_secs = (assets.model(models.gate).animations.iter())
            .find(|a| a.name == "open")
            .expect("у двери есть open")
            .duration;
        Self {
            level: Level::new(&map, &models),
            models,
            map,
            open_secs,
        }
    }

    /// Тик до победы; мёртвый игрок стоит, а мир идёт.
    fn play(&mut self, ctx: &Context) {
        let dt = ctx.time.step.as_secs_f32();
        let l = &mut self.level;
        l.player.update(&ctx.input, dt, &l.obstacles);
        l.arsenal.update(&ctx.input, dt);
        if l.player.alive() {
            // Клик захвата курсора — не выстрел.
            if ctx.grabbed && ctx.input.mouse_down(MouseButton::Left) {
                l.fire();
            }
            l.pick_up();
        }
        l.doors(dt, self.open_secs);
        for e in &mut l.enemies {
            l.projectiles
                .extend(e.update(dt, &mut l.player, &l.obstacles));
        }
        l.fly(dt);
        l.won = l.player.alive() && l.player.pos.floor().as_ivec2() == self.map.exit;
        if l.won {
            // `play` больше не позовётся, и `draw` качал бы тела между `prev` и `pos` каждый кадр.
            l.player.prev = l.player.pos;
            for e in &mut l.enemies {
                e.prev = e.pos;
            }
            for p in &mut l.projectiles {
                p.prev = p.pos;
            }
        }
    }
}

impl Game for Shooter {
    fn init(ctx: &mut Context) -> Self {
        let text = fs::read_to_string(LEVEL).unwrap_or_else(|e| panic!("{LEVEL}: {e}"));
        let map = Map::parse(&text).unwrap_or_else(|e| panic!("{LEVEL}: {e}"));
        Self::new(map, &mut ctx.assets)
    }

    fn update(&mut self, ctx: &mut Context) {
        // Клик, пришедший до потери фокуса, захватил бы курсор в фоновом окне.
        if ctx.input.mouse_just_pressed(MouseButton::Left) && ctx.focused {
            ctx.cursor_grab = true;
        }
        if ctx.input.just_pressed(KeyCode::Escape) {
            ctx.cursor_grab = false;
        }
        let l = &self.level;
        if (l.won || !l.player.alive()) && ctx.input.just_pressed(KeyCode::Enter) {
            self.level = Level::new(&self.map, &self.models);
        } else if !l.won {
            self.play(ctx);
        }
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        // После выхода `update` не крутит игрока, и непотреблённый сдвиг дёргал бы камеру до тика.
        let delta = if self.level.won {
            Vec2::ZERO
        } else {
            ctx.input.mouse_delta()
        };
        ctx.gfx.scene = Some(Scene {
            camera: self.level.player.camera(alpha, delta),
            ..Scene::default()
        });
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Shooter>(Config {
        title: "shooter".into(),
        size: (1280, 720),
        tick_hz: TICK_HZ,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: false,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use engine::{
        Camera3D,
        ElementState::{Pressed, Released},
        FixedClock, Texture, ivec2, vec2, vec3,
    };

    use super::*;
    use crate::enemy::{State, stats};
    use crate::map::{Dir, EnemyKind};
    use crate::player::{EYE, SENSITIVITY, SPEED};

    fn all(m: &Models) -> [Handle<Model>; 15] {
        [
            m.tree,
            m.tree_high,
            m.rock,
            m.fence,
            m.grass,
            m.brawler,
            m.gunner,
            m.runner,
            m.blaster,
            m.launcher,
            m.bullet,
            m.clip,
            m.crate_,
            m.key,
            m.gate,
        ]
    }

    fn textures(model: &Model) -> Vec<Handle<Texture>> {
        model
            .nodes
            .iter()
            .flat_map(|n| n.primitives.iter().map(|p| p.texture))
            .collect()
    }

    #[test]
    fn every_model_loads_and_characters_have_clips() {
        let mut ctx = Context::default();
        let g = Shooter::init(&mut ctx);
        let m = &g.models;
        let handles = all(m);
        for (i, h) in handles.iter().enumerate() {
            assert!(!ctx.assets.model(*h).nodes.is_empty(), "модель {i}");
            assert!(!handles[..i].contains(h), "модель {i} — повтор");
        }
        for h in [m.brawler, m.gunner, m.runner] {
            let model = ctx.assets.model(h);
            for clip in CLIPS {
                assert!(model.animations.iter().any(|a| a.name == clip), "{clip}");
            }
            assert!(model.nodes.iter().any(|n| n.name == "arm-right"));
        }
    }

    #[test]
    fn sets_keep_their_own_textures() {
        let mut ctx = Context::default();
        let m = Models::load(&mut ctx.assets);
        let tex = |h| textures(ctx.assets.model(h));
        let sets = [m.tree, m.blaster, m.brawler, m.gunner, m.key];
        let firsts: Vec<_> = sets.iter().map(|&h| tex(h)[0]).collect();
        for (i, t) in firsts.iter().enumerate() {
            assert_ne!(*t, Handle::WHITE, "набор {i}");
            assert!(!firsts[..i].contains(t), "набор {i} делит текстуру");
        }
        for (a, b) in [(m.tree, m.rock), (m.blaster, m.crate_), (m.key, m.gate)] {
            assert_eq!(tex(a)[0], tex(b)[0]);
        }
    }

    #[test]
    fn obstacles_fit_cell_and_character_stands_on_floor() {
        let mut ctx = Context::default();
        let m = Models::load(&mut ctx.assets);
        let placed = |h, scale| {
            let model = ctx.assets.model(h);
            model.bounds().unwrap().transformed(standing(model, scale))
        };
        for h in [m.tree, m.tree_high, m.rock] {
            let b = placed(h, FOREST_SCALE);
            assert!(b.min.y.abs() < 1e-5, "{b:?}");
            assert!(b.max.y >= CELL * 0.8, "{b:?}");
            for v in [b.min, b.max] {
                assert!(v.x.abs() <= CELL / 2.0 && v.z.abs() <= CELL / 2.0, "{b:?}");
            }
        }
        for h in [m.brawler, m.gunner, m.runner] {
            let b = placed(h, CHARACTER_SCALE);
            assert!(b.min.y.abs() < 1e-5, "{b:?}");
            assert!((1.7..1.9).contains(&b.max.y), "{b:?}");
            assert!(b.max.x - b.min.x < CELL, "{b:?}");
        }
    }

    #[test]
    fn init_reads_level_from_disk() {
        let mut ctx = Context::default();
        let g = Shooter::init(&mut ctx);
        assert_eq!(g.map, Map::parse(LEVEL01).unwrap());
    }

    fn shooter(text: &str) -> (Shooter, Context) {
        let mut ctx = Context::new(FixedClock::new(TICK_HZ, Duration::from_millis(250)).step());
        let g = Shooter::new(Map::parse(text).unwrap(), &mut ctx.assets);
        (g, ctx)
    }

    fn camera(ctx: &Context) -> Camera3D {
        ctx.gfx.scene.as_ref().unwrap().camera
    }

    const ROOM: &str = "#####\n#P.X#\n#####\n";

    #[test]
    fn player_starts_at_cell_centre_facing_open_neighbour() {
        let (g, _) = shooter(ROOM);
        assert_eq!(g.level.player.pos, vec2(1.5, 1.5));
        assert_eq!(g.level.player.yaw, Dir::E.yaw());
    }

    #[test]
    fn update_walks_player_by_tick_step() {
        let (mut g, mut ctx) = shooter(ROOM);
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        g.update(&mut ctx);
        assert_eq!(g.level.player.pos, vec2(1.5 + SPEED / TICK_HZ as f32, 1.5));
    }

    #[test]
    fn closed_door_holds_player() {
        let (mut g, mut ctx) = shooter("######\n#PD.X#\n#k...#\n######\n");
        g.level.player.yaw = Dir::E.yaw();
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        for _ in 0..120 {
            g.update(&mut ctx);
        }
        assert!(
            (g.level.player.pos.x - (2.0 - RADIUS)).abs() < 1e-5,
            "{:?}",
            g.level.player.pos
        );
    }

    #[test]
    fn update_consumes_mouse_and_draw_adds_rest() {
        let (mut g, mut ctx) = shooter(ROOM);
        let yaw = g.level.player.yaw;
        ctx.input.on_mouse_motion(vec2(100.0, -40.0));
        g.update(&mut ctx);
        ctx.input.end_tick();
        assert_eq!(g.level.player.yaw, yaw - 100.0 * SENSITIVITY);
        g.draw(&mut ctx, 1.0);
        assert_eq!(camera(&ctx).yaw, g.level.player.yaw);
        assert_eq!(camera(&ctx).pitch, 40.0 * SENSITIVITY);

        let before = g.level.player;
        ctx.input.on_mouse_motion(vec2(50.0, 0.0));
        g.draw(&mut ctx, 1.0);
        assert_eq!(camera(&ctx).yaw, before.yaw - 50.0 * SENSITIVITY);
        assert_eq!(g.level.player, before);
        assert_eq!(camera(&ctx).pos, vec3(1.5 * CELL, EYE, 1.5 * CELL));
    }

    #[test]
    fn click_grabs_cursor_only_when_focused() {
        let (mut g, mut ctx) = shooter(ROOM);
        ctx.input.on_button(MouseButton::Left, Pressed);
        g.update(&mut ctx);
        assert!(!ctx.cursor_grab, "окно без фокуса");
        ctx.input.on_button(MouseButton::Left, Released);
        ctx.input.end_tick();

        ctx.focused = true;
        ctx.input.on_button(MouseButton::Right, Pressed);
        g.update(&mut ctx);
        assert!(!ctx.cursor_grab, "не левая кнопка");
        ctx.input.end_tick();

        ctx.input.on_button(MouseButton::Left, Pressed);
        g.update(&mut ctx);
        assert!(ctx.cursor_grab);
        ctx.input.end_tick();
        g.update(&mut ctx);
        assert!(ctx.cursor_grab, "без Esc захват держится");
        ctx.input.on_key(KeyCode::Escape, Pressed, false);
        g.update(&mut ctx);
        assert!(!ctx.cursor_grab);
    }

    fn tick(g: &mut Shooter, ctx: &mut Context) {
        g.update(ctx);
        ctx.input.end_tick();
    }

    fn until(g: &mut Shooter, ctx: &mut Context, done: impl Fn(&Shooter) -> bool) {
        for _ in 0..TICK_HZ * 10 {
            if done(g) {
                return;
            }
            tick(g, ctx);
        }
        panic!("за 10 с не дождались");
    }

    fn walk_east(g: &mut Shooter, ctx: &mut Context) {
        g.level.player.yaw = Dir::E.yaw();
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
    }

    #[test]
    fn key_is_taken_once_by_touch() {
        let (mut g, mut ctx) = shooter("#######\n#P.k.X#\n#######\n");
        g.level.player.pos = vec2(3.5 - PICKUP, 1.5);
        tick(&mut g, &mut ctx);
        assert!(!g.level.key, "ровно на PICKUP не берётся");
        g.level.player.pos = vec2(1.5, 1.5);
        walk_east(&mut g, &mut ctx);
        until(&mut g, &mut ctx, |g| g.level.key);
        assert_eq!(g.level.keys, []);
        let d = g.level.player.pos.distance(vec2(3.5, 1.5));
        assert!(d < PICKUP && d >= PICKUP - SPEED / TICK_HZ as f32, "{d}");
        until(&mut g, &mut ctx, |g| g.level.won);
        assert!(g.level.key);
    }

    #[test]
    fn key_opens_door_and_door_holds_until_animation_ends() {
        let (mut g, mut ctx) = shooter("#######\n#kPD.X#\n#######\n");
        let door = ivec2(3, 1);
        walk_east(&mut g, &mut ctx);
        for _ in 0..TICK_HZ {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level.doors[0].opened, None, "без ключа");
        assert!((g.level.player.pos.x - (3.0 - RADIUS)).abs() < 1e-5);

        g.level.player.yaw = Dir::W.yaw();
        until(&mut g, &mut ctx, |g| g.level.key);
        assert_eq!(g.level.doors[0].opened, None, "ключ издалека не открывает");
        g.level.player.yaw = Dir::E.yaw();
        until(&mut g, &mut ctx, |g| g.level.doors[0].opened.is_some());
        assert!(g.level.player.pos.x <= 3.0 - RADIUS);
        let dt = 1.0 / TICK_HZ as f32;
        while g.level.obstacles.solid(door) {
            let t = g.level.doors[0].opened.unwrap();
            assert!(t < g.open_secs, "{t}");
            assert!(g.level.player.pos.x <= 3.0 - RADIUS + 1e-5);
            tick(&mut g, &mut ctx);
        }
        let t = g.level.doors[0].opened.unwrap();
        assert!(t >= g.open_secs && t - dt < g.open_secs, "{t}");
        until(&mut g, &mut ctx, |g| g.level.won);
    }

    #[test]
    fn medkit_heals_up_to_max_and_waits_at_full_health() {
        let (mut g, mut ctx) = shooter("#########\n#P+.+.+X#\n#########\n");
        walk_east(&mut g, &mut ctx);
        until(&mut g, &mut ctx, |g| g.level.player.pos.x > 3.0);
        assert_eq!(g.level.medkits.len(), 3, "при полном здоровье не берётся");
        g.level.player.hp = MAX_HP - 10;
        until(&mut g, &mut ctx, |g| g.level.medkits.len() == 2);
        assert_eq!(g.level.player.hp, MAX_HP);
        assert_eq!(g.level.medkits, [ivec2(2, 1), ivec2(6, 1)]);
        g.level.player.hp = 50;
        until(&mut g, &mut ctx, |g| g.level.medkits.len() == 1);
        assert_eq!(g.level.player.hp, 50 + MEDKIT);
    }

    #[test]
    fn dead_player_takes_nothing() {
        let (mut g, mut ctx) = shooter("#######\n#P+akX#\n#######\n");
        for cell in [2.5, 3.5, 4.5] {
            g.level.player.pos = vec2(cell, 1.5);
            g.level.player.hp = 0;
            tick(&mut g, &mut ctx);
        }
        assert_eq!(
            g.level,
            Level {
                player: g.level.player,
                ..Level::new(&g.map, &g.models)
            }
        );
    }

    #[test]
    fn ammo_pack_adds_to_both_weapons() {
        let (mut g, mut ctx) = shooter("######\n#Pa.X#\n######\n");
        let before = g.level.arsenal.weapons.map(|w| w.ammo);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level.ammo, [ivec2(2, 1)], "издалека не берётся");
        walk_east(&mut g, &mut ctx);
        until(&mut g, &mut ctx, |g| g.level.ammo.is_empty());
        let after = g.level.arsenal.weapons.map(|w| w.ammo);
        assert_eq!(after, [before[0] + AMMO_PACK[0], before[1] + AMMO_PACK[1]]);
    }

    #[test]
    fn exit_wins_and_enter_restarts_level() {
        let (mut g, mut ctx) = shooter("######\n#P..X#\n#..f.#\n######\n");
        walk_east(&mut g, &mut ctx);
        ctx.input.on_key(KeyCode::Enter, Pressed, false);
        tick(&mut g, &mut ctx);
        assert_ne!(
            g.level.player.pos,
            vec2(1.5, 1.5),
            "Enter до победы не рестартит"
        );
        ctx.input.on_key(KeyCode::Enter, Released, false);

        until(&mut g, &mut ctx, |g| g.level.won);
        assert_eq!(g.level.player.pos.floor().as_ivec2(), g.map.exit);
        assert_eq!(g.level.player.prev, g.level.player.pos);
        assert!(g.level.enemies.iter().all(|e| e.prev == e.pos));
        let won = g.level.clone();
        for _ in 0..10 {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level, won, "после победы мир стоит");

        ctx.input.on_key(KeyCode::Enter, Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level, Level::new(&g.map, &g.models));
    }

    #[test]
    fn dead_player_on_exit_does_not_win() {
        let (mut g, mut ctx) = shooter(ROOM);
        g.level.player.pos = vec2(3.5, 1.5);
        g.level.player.hp = 0;
        tick(&mut g, &mut ctx);
        assert!(!g.level.won);
    }

    #[test]
    fn death_waits_for_enter_and_restarts_full_level() {
        let (mut g, mut ctx) = shooter("########\n#Pm+a.X#\n########\n");
        ctx.grabbed = true;
        ctx.input.on_button(MouseButton::Left, Pressed);
        for _ in 0..TICK_HZ / 2 {
            tick(&mut g, &mut ctx);
        }
        ctx.input.on_button(MouseButton::Left, Released);
        assert!(g.level.enemies[0].hp < stats(EnemyKind::Brawler).hp);
        ctx.input.on_key(KeyCode::Enter, Pressed, false);
        tick(&mut g, &mut ctx);
        ctx.input.on_key(KeyCode::Enter, Released, false);
        assert!(g.level.player.hp < MAX_HP, "живого Enter не рестартит");

        until(&mut g, &mut ctx, |g| !g.level.player.alive());
        let ammo = g.level.arsenal.weapons[0].ammo;
        ctx.input.on_button(MouseButton::Left, Pressed);
        for _ in 0..TICK_HZ {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level.arsenal.weapons[0].ammo, ammo, "мёртвый не стреляет");
        assert!(!g.level.player.alive() && !g.level.won);

        ctx.input.on_key(KeyCode::Enter, Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level, Level::new(&g.map, &g.models));
    }

    #[test]
    fn hitscan_needs_grab_and_skips_dead_enemy() {
        let (mut g, mut ctx) = shooter("########\n#P.m.m.#\n#X.....#\n########\n");
        g.level.enemies[0].hurt(1000);
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        assert_eq!(
            g.level.arsenal.weapons[0].ammo, 40,
            "клик захвата — не выстрел"
        );
        ctx.grabbed = true;
        tick(&mut g, &mut ctx);
        let w = g.level.arsenal.weapons[0];
        assert_eq!(w.ammo, 39);
        assert_eq!(
            g.level.enemies[1].hp,
            stats(EnemyKind::Brawler).hp - w.damage
        );
        assert_eq!(g.level.enemies[1].state, State::Chase);
    }

    #[test]
    fn launcher_projectile_flies_past_dead_enemy_into_live_one() {
        let (mut g, mut ctx) = shooter("########\n#P.m.m.#\n#X.....#\n########\n");
        g.level.enemies[0].hurt(1000);
        ctx.grabbed = true;
        ctx.input.on_key(KeyCode::Digit2, Pressed, false);
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        ctx.input.on_button(MouseButton::Left, Released);
        assert_eq!(g.level.projectiles.len(), 1);
        until(&mut g, &mut ctx, |g| g.level.projectiles.is_empty());
        let w = g.level.arsenal.weapons[1];
        assert_eq!(
            g.level.enemies[1].hp,
            stats(EnemyKind::Brawler).hp - w.damage
        );
    }

    #[test]
    fn gunner_bolt_hurts_player_and_wall_stops_bolt() {
        let (mut g, mut ctx) = shooter("#######\n#P...s#\n#X....#\n#######\n");
        tick(&mut g, &mut ctx);
        assert_eq!(g.level.projectiles.len(), 1);
        until(&mut g, &mut ctx, |g| g.level.player.hp < MAX_HP);
        assert_eq!(g.level.player.hp, MAX_HP - stats(EnemyKind::Gunner).damage);
        assert_eq!(g.level.projectiles, []);
    }

    #[test]
    fn projectile_vanishes_at_wall() {
        let (mut g, mut ctx) = shooter(ROOM);
        g.level.player.yaw = Dir::N.yaw();
        ctx.grabbed = true;
        ctx.input.on_key(KeyCode::Digit2, Pressed, false);
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        ctx.input.on_button(MouseButton::Left, Released);
        assert_eq!(g.level.projectiles.len(), 1);
        until(&mut g, &mut ctx, |g| g.level.projectiles.is_empty());
    }
}
