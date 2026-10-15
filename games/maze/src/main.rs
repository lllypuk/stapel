mod enemy;
mod map;
mod player;

use std::f32::consts::{FRAC_PI_2, TAU};
use std::{fs, mem};

use enemy::Orc;
use engine::{
    Assets, Camera3D, Color, Config, Context, Game, Handle, IVec2, KeyCode, Mat4, Model, ModelDraw,
    MouseButton, Pose, Quat, Scene, Vec2, vec3,
};
use map::{Dir, Map, cell_center};
use player::{Player, RADIUS};

const TICK_HZ: u32 = 60;
/// Тиков на оборот монеты.
const COIN_TURN: u64 = 2 * TICK_HZ as u64;
/// Касание предмета: центр игрока ближе к центру его клетки, м.
const PICKUP: f32 = 0.5;
/// Касание ворот: центр игрока ближе к их клетке, м.
const GATE_REACH: f32 = RADIUS + 0.05;
/// Свет без источников, пока факелы не светят.
const AMBIENT: Color = Color::WHITE;
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
const LEVEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/levels/level01.txt");
/// Замороженная копия первого уровня: тесты не зависят от правок `assets/levels/`.
#[cfg(test)]
const LEVEL01: &str = include_str!("testdata/level01.txt");

struct Models {
    wall: Handle<Model>,
    floor: Handle<Model>,
    gate: Handle<Model>,
    banner: Handle<Model>,
    coin: Handle<Model>,
    key: Handle<Model>,
    stairs: Handle<Model>,
    orc: Handle<Model>,
}

impl Models {
    fn load(assets: &mut Assets) -> Self {
        let mut load = |name: &str| assets.load_model(format!("{ASSETS}{name}.glb"));
        Self {
            wall: load("wall"),
            floor: load("floor"),
            gate: load("gate"),
            banner: load("banner"),
            coin: load("coin"),
            key: load("key"),
            stairs: load("stairs"),
            orc: load("character-orc"),
        }
    }
}

/// Модель в центре клетки, повёрнутая вокруг +Y на `yaw`.
fn at(model: Handle<Model>, cell: IVec2, yaw: f32) -> ModelDraw {
    ModelDraw {
        model,
        transform: Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), cell_center(cell)),
        pose: None,
    }
}

/// Состояние уровня, которое сбрасывает рестарт.
#[derive(Clone, Debug, PartialEq)]
struct Level {
    player: Player,
    /// Не взятые.
    coins: Vec<IVec2>,
    /// Не взятые.
    keys: Vec<IVec2>,
    /// Ключи в руках.
    held: usize,
    gates: Vec<Gate>,
    orcs: Vec<Orc>,
    won: bool,
}

impl Level {
    fn new(map: &Map) -> Self {
        Self {
            player: Player::new(map.player.as_vec2() + 0.5, map.facing.yaw()),
            coins: map.coins.clone(),
            keys: map.keys.clone(),
            held: 0,
            gates: map
                .gates
                .iter()
                .map(|&cell| Gate { cell, opened: None })
                .collect(),
            orcs: map.orcs.iter().map(|&cell| Orc::new(map, cell)).collect(),
            won: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Gate {
    cell: IVec2,
    /// Тик, в который ключ открыл ворота.
    opened: Option<u64>,
}

/// Секунды в `ticks` тиках.
fn secs(ticks: u64) -> f32 {
    ticks as f32 / TICK_HZ as f32
}

/// Поворот монеты на тике `tick`.
fn coin_yaw(tick: u64) -> f32 {
    (tick % COIN_TURN) as f32 / COIN_TURN as f32 * TAU
}

/// Центр игрока `pos` касается предмета в центре клетки `cell`.
fn touches_item(pos: Vec2, cell: IVec2) -> bool {
    pos.distance(cell.as_vec2() + 0.5) < PICKUP
}

/// Центр игрока `pos` у самой клетки `cell`: упор в твёрдую клетку — ровно `RADIUS`.
fn touches_cell(pos: Vec2, cell: IVec2) -> bool {
    let min = cell.as_vec2();
    pos.distance(pos.clamp(min, min + 1.0)) <= GATE_REACH
}

struct Maze {
    map: Map,
    models: Models,
    /// Стены, пол и баннеры: за уровень не меняются.
    statics: Vec<ModelDraw>,
    /// Анимация `open` ворот; без неё ворота открываются мгновенно.
    open: Option<usize>,
    open_secs: f32,
    /// Анимация `walk` орка; без неё орк скользит в покое.
    walk: Option<usize>,
    level: Level,
    /// Тиков с запуска; рестарт не сбрасывает — от него отсчитаны `Gate::opened`.
    tick: u64,
}

impl Maze {
    fn new(map: Map, assets: &mut Assets) -> Self {
        let models = Models::load(assets);
        let mut statics: Vec<ModelDraw> = map
            .cells()
            .map(|cell| {
                let model = if map.wall(cell) {
                    models.wall
                } else {
                    models.floor
                };
                at(model, cell, 0.0)
            })
            .collect();
        // Баннер в модели висит на грани −Z своей клетки: в клетке факела он ложится на стену.
        statics.extend(
            map.torches
                .iter()
                .map(|t| at(models.banner, t.cell, t.wall.yaw())),
        );
        let animations = &assets.model(models.gate).animations;
        let open = animations.iter().position(|a| a.name == "open");
        let open_secs = open.map_or(0.0, |i| animations[i].duration);
        let walk = (assets.model(models.orc).animations.iter()).position(|a| a.name == "walk");
        Self {
            level: Level::new(&map),
            map,
            models,
            statics,
            open,
            open_secs,
            walk,
            tick: 0,
        }
    }

    /// Дверь в модели поперёк Z: в коридоре вдоль X её поворачиваем.
    fn gate_yaw(&self, cell: IVec2) -> f32 {
        let along_z =
            self.map.wall(cell + Dir::E.offset()) || self.map.wall(cell + Dir::W.offset());
        if along_z { 0.0 } else { FRAC_PI_2 }
    }

    /// Ворота твёрдые, пока не доиграла `open`; край карты тоже твёрдый.
    fn solid(&self, cell: IVec2) -> bool {
        let shut = |g: &Gate| {
            g.opened
                .is_none_or(|t| secs(self.tick - t) < self.open_secs)
        };
        !self.map.floor(cell) || self.level.gates.iter().any(|g| g.cell == cell && shut(g))
    }

    /// Тик игры до победы: шаг, предметы, ворота, орки, выход; касание орка — рестарт.
    fn play(&mut self, ctx: &Context) {
        let dt = ctx.time.step.as_secs_f32();
        let mut player = self.level.player;
        player.update(&ctx.input, dt, |c| self.solid(c));
        let l = &mut self.level;
        l.player = player;
        let pos = player.pos;
        l.coins.retain(|&c| !touches_item(pos, c));
        let keys = l.keys.len();
        l.keys.retain(|&c| !touches_item(pos, c));
        l.held += keys - l.keys.len();
        for g in &mut l.gates {
            if l.held > 0 && g.opened.is_none() && touches_cell(pos, g.cell) {
                g.opened = Some(self.tick);
                l.held -= 1;
            }
        }
        let mut orcs = mem::take(&mut self.level.orcs);
        for o in &mut orcs {
            o.update(dt, |c| self.solid(c));
        }
        if orcs.iter().any(|o| o.touches(pos)) {
            self.level = Level::new(&self.map);
            return;
        }
        self.level.orcs = orcs;
        self.level.won = self.map.exits.contains(&pos.floor().as_ivec2());
    }

    fn scene(&self, camera: Camera3D, alpha: f32, assets: &Assets) -> Scene {
        let m = &self.models;
        let l = &self.level;
        let mut models = self.statics.clone();
        let spin = coin_yaw(self.tick);
        models.extend(l.coins.iter().map(|&c| at(m.coin, c, spin)));
        let items = [(m.key, &l.keys), (m.stairs, &self.map.exits)];
        for (model, cells) in items {
            models.extend(cells.iter().map(|&c| at(model, c, 0.0)));
        }
        models.extend(l.gates.iter().map(|g| {
            let pose = g.opened.zip(self.open).map(|(t, anim)| {
                let mut pose = Pose::default();
                assets
                    .model(m.gate)
                    .pose(anim, secs(self.tick - t), false, &mut pose);
                pose
            });
            ModelDraw {
                pose,
                ..at(m.gate, g.cell, self.gate_yaw(g.cell))
            }
        }));
        let orc = assets.model(m.orc);
        models.extend(l.orcs.iter().map(|o| {
            let pose = self.walk.map(|anim| {
                let mut pose = Pose::default();
                orc.pose(anim, o.walked_at(alpha) / enemy::SPEED, true, &mut pose);
                pose
            });
            let p = o.at(alpha);
            ModelDraw {
                model: m.orc,
                transform: Mat4::from_rotation_translation(
                    Quat::from_rotation_y(o.yaw()),
                    vec3(p.x, 0.0, p.y),
                ),
                pose,
            }
        }));
        Scene {
            camera,
            ambient: AMBIENT,
            models,
            ..Scene::default()
        }
    }
}

impl Game for Maze {
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
        if !self.level.won {
            self.play(ctx);
        } else if ctx.input.just_pressed(KeyCode::Enter) {
            self.level = Level::new(&self.map);
        }
        self.tick += 1;
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        let camera = self.level.player.camera(alpha, ctx.input.mouse_delta());
        ctx.gfx.scene = Some(self.scene(camera, alpha, &ctx.assets));
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Maze>(Config {
        title: "maze".into(),
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
        ElementState::{Pressed, Released},
        FixedClock, MAX_PITCH, Vec2, Vec3, ivec2, step_frame, vec2, vec3,
    };

    use crate::player::{EYE, SENSITIVITY, SPEED};

    use super::*;

    fn clock() -> FixedClock {
        FixedClock::new(TICK_HZ, Duration::from_millis(250))
    }

    fn maze(text: &str) -> (Maze, Context) {
        let mut ctx = Context::new(clock().step());
        let g = Maze::new(Map::parse(text).unwrap(), &mut ctx.assets);
        (g, ctx)
    }

    /// Позиции моделей `model` в сцене кадра, в порядке рисования.
    fn placed(scene: &Scene, model: Handle<Model>) -> Vec<Vec3> {
        draws(scene, model)
            .map(|d| d.transform.w_axis.truncate())
            .collect()
    }

    fn draws(scene: &Scene, model: Handle<Model>) -> impl Iterator<Item = &ModelDraw> {
        scene.models.iter().filter(move |d| d.model == model)
    }

    fn centers(cells: impl IntoIterator<Item = IVec2>) -> Vec<Vec3> {
        cells.into_iter().map(cell_center).collect()
    }

    #[test]
    fn init_loads_level_and_all_models() {
        let mut ctx = Context::default();
        let g = Maze::init(&mut ctx);
        let m = &g.models;
        for h in [
            m.wall, m.floor, m.gate, m.banner, m.coin, m.key, m.stairs, m.orc,
        ] {
            assert_ne!(ctx.assets.model(h).nodes.len(), 0);
        }
        let anims = |h| -> Vec<String> {
            let model: &Model = ctx.assets.model(h);
            model.animations.iter().map(|a| a.name.clone()).collect()
        };
        assert!(anims(m.gate).contains(&"open".into()));
        assert!(anims(m.orc).contains(&"walk".into()));
        let textures = ctx.assets.take_pending();
        assert_eq!(textures.len(), 2, "белая и общая colormap");
        assert_eq!((textures[1].width, textures[1].height), (512, 512));
    }

    #[test]
    fn draw_puts_scene_with_camera_at_player_eye() {
        let (mut g, mut ctx) = maze(LEVEL01);
        g.draw(&mut ctx, 0.5);
        let scene = ctx.gfx.scene.as_ref().unwrap();
        let p = cell_center(g.map.player);
        assert_eq!(
            *scene,
            g.scene(
                Camera3D {
                    pos: vec3(p.x, EYE, p.z),
                    yaw: g.map.facing.yaw(),
                    ..Camera3D::default()
                },
                0.5,
                &ctx.assets
            )
        );
        assert_eq!(scene.ambient, AMBIENT);
    }

    #[test]
    fn camera_looks_at_open_neighbour() {
        let (g, _) = maze("###\n#P.\n###\n");
        let camera = g.level.player.camera(1.0, Vec2::ZERO);
        assert_eq!(camera.yaw, Dir::E.yaw());
        assert!(camera.forward().abs_diff_eq(Vec3::X, 1e-6));
    }

    #[test]
    fn walls_and_floor_cover_every_cell() {
        let (g, ctx) = maze("####\n#Po#\n#kE#\n#DX#\n####\n");
        let scene = g.scene(Camera3D::default(), 0.0, &ctx.assets);
        let cells = || g.map.cells();
        assert_eq!(
            placed(&scene, g.models.wall),
            centers(cells().filter(|&c| g.map.wall(c)))
        );
        assert_eq!(
            placed(&scene, g.models.floor),
            centers(cells().filter(|&c| g.map.floor(c)))
        );
    }

    #[test]
    fn items_stand_on_their_cells() {
        let (g, ctx) = maze("####\n#Po#\n#kE#\n#DX#\n####\n");
        let scene = g.scene(Camera3D::default(), 0.0, &ctx.assets);
        let m = &g.models;
        assert_eq!(placed(&scene, m.coin), centers([ivec2(2, 1)]));
        assert_eq!(placed(&scene, m.key), centers([ivec2(1, 2)]));
        assert_eq!(placed(&scene, m.orc), centers([ivec2(2, 2)]));
        assert_eq!(placed(&scene, m.gate), centers([ivec2(1, 3)]));
        assert_eq!(placed(&scene, m.stairs), centers([ivec2(2, 3)]));
        let posed = |d: &ModelDraw| d.pose.is_some();
        assert!(
            scene
                .models
                .iter()
                .filter(|d| d.model != m.orc)
                .all(|d| !posed(d))
        );
        assert!(draws(&scene, m.orc).all(posed));
    }

    #[test]
    fn gate_spans_corridor() {
        let across = |text| {
            let (g, ctx) = maze(text);
            let d = draws(
                &g.scene(Camera3D::default(), 0.0, &ctx.assets),
                g.models.gate,
            )
            .next()
            .unwrap()
            .clone();
            d.transform.transform_vector3(Vec3::X)
        };
        assert!(across("..#\n.D#\n.P#\n").abs_diff_eq(Vec3::X, 1e-6));
        assert!(across("#..\n#D.\n#P.\n").abs_diff_eq(Vec3::X, 1e-6));
        assert!(across("###\n.DP\n###\n").abs().abs_diff_eq(Vec3::Z, 1e-6));
    }

    #[test]
    fn banner_hangs_on_torch_wall() {
        for (text, wall) in [
            ("P#.\n.T.\n...\n", Dir::N),
            (".P.\n.T#\n.#.\n", Dir::E),
            (".P.\n#T.\n.#.\n", Dir::S),
            (".P.\n#T.\n...\n", Dir::W),
        ] {
            let (g, ctx) = maze(text);
            let scene = g.scene(Camera3D::default(), 0.0, &ctx.assets);
            let banner = draws(&scene, g.models.banner).next().unwrap();
            let o = wall.offset().as_vec2();
            let toward_wall = banner.transform.transform_vector3(-Vec3::Z);
            assert!(
                toward_wall.abs_diff_eq(vec3(o.x, 0.0, o.y), 1e-6),
                "{wall:?}"
            );
            assert_eq!(banner.transform.w_axis.truncate(), cell_center(ivec2(1, 1)));
        }
    }

    fn press(ctx: &mut Context, key: KeyCode) {
        ctx.input.on_key(key, Pressed, false);
    }

    fn turned(yaw: f32, dx: f32) -> f32 {
        yaw - dx * SENSITIVITY
    }

    #[test]
    fn mouse_turns_once_in_update_and_draw_adds_unconsumed() {
        let (mut g, mut ctx) = maze(LEVEL01);
        let mut clock = clock();
        let step = clock.step();
        let yaw = g.level.player.yaw;
        let camera = |ctx: &Context| ctx.gfx.scene.as_ref().unwrap().camera;

        ctx.input.on_mouse_motion(vec2(100.0, -40.0));
        assert_eq!(step_frame(&mut g, &mut ctx, &mut clock, step * 2), 2);
        assert_eq!(g.level.player.yaw, turned(yaw, 100.0));
        assert_eq!(g.level.player.pitch, 40.0 * SENSITIVITY);
        assert_eq!(camera(&ctx).yaw, g.level.player.yaw);

        let before = g.level.player;
        ctx.input.on_mouse_motion(vec2(50.0, 0.0));
        assert_eq!(step_frame(&mut g, &mut ctx, &mut clock, Duration::ZERO), 0);
        assert_eq!(camera(&ctx).yaw, turned(before.yaw, 50.0));
        assert_eq!(camera(&ctx).pitch, before.pitch);
        g.draw(&mut ctx, clock.alpha());
        assert_eq!(camera(&ctx).yaw, turned(before.yaw, 50.0));
        assert_eq!(g.level.player, before);

        assert_eq!(step_frame(&mut g, &mut ctx, &mut clock, step), 1);
        assert_eq!(g.level.player.yaw, turned(before.yaw, 50.0));
        assert_eq!(camera(&ctx).yaw, g.level.player.yaw);
    }

    #[test]
    fn draw_interpolates_position_without_changing_state() {
        let (mut g, mut ctx) = maze("###\n#P.\n#..\n");
        press(&mut ctx, KeyCode::KeyW);
        g.update(&mut ctx);
        let p = g.level.player;
        assert_ne!(p.prev, p.pos);
        g.draw(&mut ctx, 0.25);
        let pos = ctx.gfx.scene.as_ref().unwrap().camera.pos;
        let at = p.prev.lerp(p.pos, 0.25);
        assert!(pos.abs_diff_eq(vec3(at.x, EYE, at.y), 1e-6), "{pos:?}");
        assert_eq!(g.level.player, p);
    }

    #[test]
    fn pitch_stays_clamped_in_draw() {
        let (mut g, mut ctx) = maze(LEVEL01);
        ctx.input.on_mouse_motion(vec2(0.0, -1e6));
        g.draw(&mut ctx, 0.0);
        assert_eq!(ctx.gfx.scene.as_ref().unwrap().camera.pitch, MAX_PITCH);
    }

    #[test]
    fn click_grabs_cursor_only_when_focused() {
        let (mut g, mut ctx) = maze(LEVEL01);
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

        g.update(&mut ctx);
        assert!(!ctx.cursor_grab, "кнопка зажата, но не нажата в этом тике");
        ctx.input.on_button(MouseButton::Left, Pressed);
        g.update(&mut ctx);
        assert!(ctx.cursor_grab);
    }

    #[test]
    fn click_before_focus_loss_does_not_grab() {
        let (mut g, mut ctx) = maze(LEVEL01);
        ctx.focused = true;
        ctx.input.on_button(MouseButton::Left, Pressed);
        ctx.focused = false;
        ctx.input.focus_lost();
        g.update(&mut ctx);
        assert!(!ctx.cursor_grab);
    }

    #[test]
    fn escape_releases_grab() {
        let (mut g, mut ctx) = maze(LEVEL01);
        ctx.cursor_grab = true;
        g.update(&mut ctx);
        assert!(ctx.cursor_grab, "без Esc захват держится");
        press(&mut ctx, KeyCode::Escape);
        g.update(&mut ctx);
        assert!(!ctx.cursor_grab);
    }

    #[test]
    fn closed_gate_is_solid() {
        let (mut g, mut ctx) = maze("#####\n#PD.#\n#####\n");
        press(&mut ctx, KeyCode::KeyW);
        for _ in 0..60 {
            g.update(&mut ctx);
        }
        assert!(
            (g.level.player.pos.x - (2.0 - RADIUS)).abs() < 1e-5,
            "{:?}",
            g.level.player.pos
        );
        assert_eq!(g.level.gates[0].opened, None, "без ключа ворота стоят");
        let (mut g, mut ctx) = maze("#####\n#P..#\n#####\n");
        press(&mut ctx, KeyCode::KeyW);
        for _ in 0..60 {
            g.update(&mut ctx);
        }
        assert!(
            (g.level.player.pos.x - (4.0 - RADIUS)).abs() < 1e-5,
            "без ворот — до стены"
        );
    }

    /// Тик, как в `step_frame`: `update`, затем `end_tick`.
    fn tick(g: &mut Maze, ctx: &mut Context) {
        g.update(ctx);
        ctx.input.end_tick();
    }

    /// Тики, пока не выполнится `done`; дольше секунды — паника.
    fn until(g: &mut Maze, ctx: &mut Context, done: impl Fn(&Maze) -> bool) {
        for _ in 0..TICK_HZ {
            if done(g) {
                return;
            }
            tick(g, ctx);
        }
        panic!("за секунду не дождались");
    }

    #[test]
    fn coin_and_key_are_taken_once_by_touch() {
        let (mut g, mut ctx) = maze("######\n#Pok.#\n######\n");
        press(&mut ctx, KeyCode::KeyW);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level.coins, [ivec2(2, 1)], "ещё не дошли");
        assert_eq!(g.level.keys, [ivec2(3, 1)]);
        until(&mut g, &mut ctx, |g| g.level.coins.is_empty());
        let x = g.level.player.pos.x;
        assert!(x > 2.0 && x <= 2.0 + 1e-5 + SPEED / TICK_HZ as f32, "{x}");
        until(&mut g, &mut ctx, |g| g.level.keys.is_empty());
        assert_eq!(g.level.held, 1);
        for _ in 0..TICK_HZ {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level.held, 1);
        assert!((g.level.player.pos.x - (5.0 - RADIUS)).abs() < 1e-5);
    }

    #[test]
    fn key_opens_gate_passable_after_animation() {
        let (mut g, mut ctx) = maze("##########\n#Pk.D.D..#\n##########\n");
        let first = ivec2(4, 1);
        press(&mut ctx, KeyCode::KeyW);
        until(&mut g, &mut ctx, |g| g.level.held == 1);
        assert_eq!(g.level.gates[0].opened, None, "ворота далеко");
        until(&mut g, &mut ctx, |g| g.level.gates[0].opened.is_some());
        assert_eq!(g.level.held, 0, "ключ потрачен");
        let x = g.level.player.pos.x;
        assert!(x >= 4.0 - GATE_REACH, "{x}");
        assert!(g.open_secs > 0.5, "{}", g.open_secs);
        let t0 = g.level.gates[0].opened.unwrap();
        for _ in 0..2 * TICK_HZ {
            if secs(g.tick - t0) >= g.open_secs {
                break;
            }
            assert!(g.solid(first));
            tick(&mut g, &mut ctx);
            let x = g.level.player.pos.x;
            assert!(x <= 4.0 - RADIUS + 1e-5, "{x}");
        }
        assert!(!g.solid(first));
        until(&mut g, &mut ctx, |g| g.level.player.pos.x > 5.0);
        for _ in 0..TICK_HZ {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level.gates[1].opened, None, "второму ключа нет");
        assert!((g.level.player.pos.x - (6.0 - RADIUS)).abs() < 1e-5);
    }

    #[test]
    fn exit_wins_and_enter_restarts_level() {
        let (mut g, mut ctx) = maze("######\n#PoX.#\n######\n");
        press(&mut ctx, KeyCode::Enter);
        tick(&mut g, &mut ctx);
        assert_eq!(
            g.level,
            Level::new(&g.map),
            "Enter до победы ничего не делает"
        );
        ctx.input.on_key(KeyCode::Enter, Released, false);

        press(&mut ctx, KeyCode::KeyW);
        until(&mut g, &mut ctx, |g| g.level.won);
        assert_eq!(g.level.player.pos.floor().as_ivec2(), ivec2(3, 1));
        assert_eq!(g.level.coins, []);
        let won = g.level.clone();
        for _ in 0..10 {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level, won, "после победы игрок стоит");

        press(&mut ctx, KeyCode::Enter);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level, Level::new(&g.map));
        assert_eq!(g.level.player.prev, g.level.player.pos);
    }

    #[test]
    fn coins_spin_with_ticks_not_alpha() {
        let (mut g, mut ctx) = maze("####\n#Po#\n####\n");
        let coin = |g: &mut Maze, ctx: &mut Context, alpha| {
            g.draw(ctx, alpha);
            let scene = ctx.gfx.scene.as_ref().unwrap();
            draws(scene, g.models.coin).next().unwrap().transform
        };
        let handle = g.models.coin;
        let turned = |yaw| at(handle, ivec2(2, 1), yaw).transform;
        assert_eq!(coin(&mut g, &mut ctx, 0.5), turned(0.0));
        for _ in 0..COIN_TURN / 4 {
            tick(&mut g, &mut ctx);
        }
        let quarter = coin(&mut g, &mut ctx, 0.1);
        assert!(quarter.abs_diff_eq(turned(FRAC_PI_2), 1e-6), "{quarter:?}");
        assert_eq!(coin(&mut g, &mut ctx, 0.9), quarter);
        for _ in COIN_TURN / 4..COIN_TURN {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(coin(&mut g, &mut ctx, 0.5), turned(0.0));
    }

    #[test]
    fn open_gate_drawn_in_animation_pose() {
        let (mut g, mut ctx) = maze("#######\n#PkD..#\n#######\n");
        press(&mut ctx, KeyCode::KeyW);
        until(&mut g, &mut ctx, |g| g.level.gates[0].opened.is_some());
        for _ in 0..20 {
            tick(&mut g, &mut ctx);
        }
        g.draw(&mut ctx, 0.5);
        let model = ctx.assets.model(g.models.gate);
        let mut want = Pose::default();
        model.pose(g.open.unwrap(), 21.0 / 60.0, false, &mut want);
        assert_ne!(want.globals, model.rest.globals);
        let scene = ctx.gfx.scene.as_ref().unwrap();
        let gate = draws(scene, g.models.gate).next().unwrap();
        assert_eq!(gate.pose.as_ref(), Some(&want));
        assert_eq!(gate.transform.w_axis.truncate(), cell_center(ivec2(3, 1)));
    }

    #[test]
    fn taken_items_are_not_drawn() {
        let (mut g, mut ctx) = maze("#####\n#Pok#\n#####\n");
        press(&mut ctx, KeyCode::KeyW);
        for _ in 0..TICK_HZ {
            tick(&mut g, &mut ctx);
        }
        g.draw(&mut ctx, 0.5);
        let scene = ctx.gfx.scene.as_ref().unwrap();
        assert_eq!(draws(scene, g.models.coin).count(), 0);
        assert_eq!(draws(scene, g.models.key).count(), 0);
    }

    #[test]
    fn orc_turns_at_closed_gate() {
        let (mut g, mut ctx) = maze("########\n#P#E.D.#\n########\n");
        let mut xs = Vec::new();
        for _ in 0..3 * TICK_HZ {
            tick(&mut g, &mut ctx);
            xs.push(g.level.orcs[0].pos.x);
        }
        let max = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let min = xs.iter().copied().fold(f32::INFINITY, f32::min);
        assert!((max - (5.0 - enemy::RADIUS)).abs() < 1e-5, "{max}");
        assert!((min - (3.0 + enemy::RADIUS)).abs() < 1e-5, "{min}");
    }

    #[test]
    fn orc_drawn_between_ticks_facing_heading_in_walk_pose() {
        let (mut g, mut ctx) = maze("#####\n#P#E#\n###.#\n###.#\n#####\n");
        for _ in 0..10 {
            tick(&mut g, &mut ctx);
        }
        let o = g.level.orcs[0];
        assert_eq!(o.dir, vec2(0.0, 1.0));
        g.draw(&mut ctx, 0.25);
        let scene = ctx.gfx.scene.as_ref().unwrap();
        let d = draws(scene, g.models.orc).next().unwrap();
        let at = o.prev.lerp(o.pos, 0.25);
        assert!(
            d.transform
                .w_axis
                .truncate()
                .abs_diff_eq(vec3(at.x, 0.0, at.y), 1e-6)
        );
        let front = d.transform.transform_vector3(Vec3::Z);
        assert!(front.abs_diff_eq(Vec3::Z, 1e-6), "{front:?}");
        let model = ctx.assets.model(g.models.orc);
        let mut want = Pose::default();
        let secs = (o.walked - 0.75 * o.prev.distance(o.pos)) / enemy::SPEED;
        model.pose(g.walk.unwrap(), secs, true, &mut want);
        assert_eq!(d.pose.as_ref(), Some(&want));
        let mut later = Pose::default();
        model.pose(g.walk.unwrap(), o.walked / enemy::SPEED, true, &mut later);
        assert_ne!(want, later, "фаза следует за путём");
    }

    #[test]
    fn orc_touch_restarts_level() {
        let (mut g, mut ctx) = maze("#########\n#Pok..E.#\n#########\n");
        press(&mut ctx, KeyCode::KeyW);
        until(&mut g, &mut ctx, |g| g.level.held == 1);
        assert_eq!(g.level.coins, []);
        for _ in 0..2 * TICK_HZ {
            if g.level.held == 0 {
                break;
            }
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level, Level::new(&g.map));
        assert_eq!(g.level.coins.len(), 1);
        assert_eq!(g.level.player.prev, g.level.player.pos);
        assert!(g.level.orcs.iter().all(|o| o.prev == o.pos));
    }
}
