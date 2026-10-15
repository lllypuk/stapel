mod enemy;
mod map;
mod minimap;
mod panel;
mod player;

use std::f32::consts::{FRAC_PI_2, TAU};
use std::{fs, mem};

use enemy::Orc;
use engine::{
    Assets, Camera2D, Camera3D, Color, Config, Context, Fog, Font, FontData, Game, Gfx, Handle,
    IVec2, KeyCode, Mat4, Model, ModelDraw, MouseButton, PointLight, Pose, Quat, Scene, Texture,
    Vec2, egui, vec2, vec3,
};
use map::{Dir, Map, Torch, cell_center};
use minimap::Minimap;
use panel::Cmd;
use player::{Player, RADIUS};

const TICK_HZ: u32 = 60;
/// Тиков на оборот монеты.
const COIN_TURN: u64 = 2 * TICK_HZ as u64;
/// Касание предмета: центр игрока ближе к центру его клетки, м.
const PICKUP: f32 = 0.5;
/// Касание ворот: центр игрока ближе к их клетке, м.
const GATE_REACH: f32 = RADIUS + 0.05;
/// Свет без источников.
const AMBIENT: Color = Color {
    r: 0.08,
    g: 0.08,
    b: 0.11,
    a: 1.0,
};
/// Им же очищается 3D-проход: дальше `end` сцена сливается с фоном.
const FOG: Fog = Fog {
    color: Color {
        r: 0.01,
        g: 0.008,
        b: 0.015,
        a: 1.0,
    },
    start: 2.0,
    end: 8.0,
};
/// Линейный цвет факела в полную силу.
const TORCH: Color = Color {
    r: 1.0,
    g: 0.55,
    b: 0.2,
    a: 1.0,
};
const TORCH_RANGE: f32 = 4.0;
/// Высота факела над полом, м.
const TORCH_HEIGHT: f32 = 0.8;
/// Отступ факела от стены с баннером, м.
const TORCH_GAP: f32 = 0.25;
/// Доля яркости, которую мерцание гасит в худший тик.
const FLICKER: f32 = 0.3;
/// HUD в единицах окна 1280×720; иной аспект — letterbox.
const VIEW: Vec2 = vec2(1280.0, 720.0);
/// При `VIEW` в окне 1280×720 тексель атласа — пиксель экрана.
const HUD_PX: f32 = 24.0;
const HUD_MARGIN: f32 = 12.0;
/// Поверх миникарты.
const HUD_LAYER: i32 = 3;
const WIN: &str = "EXIT FOUND";
const RESTART: &str = "PRESS ENTER";
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
const FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Regular.ttf"
);
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
    /// Тиков игры на уровне; после победы стоит.
    ticks: u64,
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
            ticks: 0,
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

/// Яркость факела `i` на тике `tick` в `[1 - FLICKER, 1]`; сдвиг фазы по `i` не даёт факелам мигать хором.
fn flicker(tick: u64, i: usize) -> f32 {
    let t = tick as f32;
    let phase = i as f32 * 1.7;
    let wave = (t * 0.31 + phase).sin() + (t * 0.83 + 2.0 * phase).sin();
    1.0 - FLICKER * (0.5 + 0.25 * wave)
}

/// `color` с RGB, умноженными на `k`.
fn scaled(color: Color, k: f32) -> Color {
    let k = f64::from(k);
    Color {
        r: color.r * k,
        g: color.g * k,
        b: color.b * k,
        a: color.a,
    }
}

/// Свет факела `torch` с яркостью `bright` у его стены.
fn torch_light(torch: &Torch, bright: f32) -> PointLight {
    let o = torch.wall.offset().as_vec2() * (0.5 - TORCH_GAP);
    PointLight {
        pos: cell_center(torch.cell) + vec3(o.x, TORCH_HEIGHT, o.y),
        color: scaled(TORCH, bright),
        range: TORCH_RANGE,
    }
}

fn text_width(font: &FontData, text: &str) -> f32 {
    text.chars()
        .filter_map(|c| font.glyph(c).or_else(|| font.glyph('?')))
        .map(|g| g.advance)
        .sum()
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
    font: Handle<Font>,
    arrow: Handle<Texture>,
    /// Стены, пол и баннеры: за уровень не меняются.
    statics: Vec<ModelDraw>,
    /// Анимация `open` ворот.
    open: usize,
    open_secs: f32,
    /// Анимация `walk` орка.
    walk: usize,
    level: Level,
    /// Тиков с запуска; рестарт не сбрасывает — от него отсчитаны `Gate::opened`.
    tick: u64,
    /// Рестарт не сбрасывает: пройденное остаётся известным.
    minimap: Minimap,
    /// Панель отладки по F1.
    panel: bool,
    queue: Vec<Cmd>,
    /// Множитель `AMBIENT`.
    ambient: f32,
    fog_end: f32,
    /// FPS для панели: `ui` не видит `Context`.
    fps: u32,
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
        let animation = |model, name| {
            let animations = &assets.model(model).animations;
            let i = (animations.iter().position(|a| a.name == name))
                .unwrap_or_else(|| panic!("нет анимации {name}"));
            (i, animations[i].duration)
        };
        let (open, open_secs) = animation(models.gate, "open");
        let (walk, _) = animation(models.orc, "walk");
        Self {
            level: Level::new(&map),
            minimap: Minimap::new(&map),
            map,
            models,
            font: assets.load_font(FONT_PATH, HUD_PX),
            arrow: assets.load_texture(format!("{ASSETS}arrow.png")),
            statics,
            open,
            open_secs,
            walk,
            tick: 0,
            panel: false,
            queue: Vec::new(),
            ambient: 1.0,
            fog_end: FOG.end,
            fps: 0,
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
        let pos = player.pos;
        self.minimap.reveal(pos);
        let l = &mut self.level;
        l.player = player;
        l.ticks += 1;
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
        if self.level.won {
            // `play` больше не позовётся, и `draw` качал бы тела между `prev` и `pos` каждый тик.
            let l = &mut self.level;
            l.player.prev = l.player.pos;
            for o in &mut l.orcs {
                o.prev = o.pos;
            }
        }
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
            let pose = g.opened.map(|t| {
                let mut pose = Pose::default();
                assets
                    .model(m.gate)
                    .pose(self.open, secs(self.tick - t), false, &mut pose);
                pose
            });
            ModelDraw {
                pose,
                ..at(m.gate, g.cell, self.gate_yaw(g.cell))
            }
        }));
        let orc = assets.model(m.orc);
        models.extend(l.orcs.iter().map(|o| {
            let mut pose = Pose::default();
            let t = o.walked_at(alpha) / enemy::SPEED;
            orc.pose(self.walk, t, true, &mut pose);
            let p = o.at(alpha);
            ModelDraw {
                model: m.orc,
                transform: Mat4::from_rotation_translation(
                    Quat::from_rotation_y(o.yaw()),
                    vec3(p.x, 0.0, p.y),
                ),
                pose: Some(pose),
            }
        }));
        let lights = (self.map.torches.iter().enumerate())
            .map(|(i, t)| torch_light(t, flicker(self.tick, i)))
            .collect();
        Scene {
            camera,
            ambient: scaled(AMBIENT, self.ambient),
            fog: Fog {
                end: self.fog_end,
                ..FOG
            },
            lights,
            models,
            meshes: Vec::new(),
        }
    }

    fn status(&self) -> String {
        let total = self.map.coins.len();
        let l = &self.level;
        format!(
            "COINS {}/{total}\nKEYS {}\nTIME {:.1}",
            total - l.coins.len(),
            l.held,
            secs(l.ticks)
        )
    }

    /// Миникарта в правом верхнем углу, счёт в левом, победа по центру.
    fn draw_hud(&self, gfx: &mut Gfx, assets: &Assets, camera: Camera3D) {
        let half = VIEW / 2.0;
        let width = self.map.size.x as f32 * minimap::CELL;
        let origin = vec2(half.x - HUD_MARGIN - width, half.y - HUD_MARGIN);
        let pos = vec2(camera.pos.x, camera.pos.z);
        (self.minimap).draw(gfx, &self.map, origin, self.arrow, pos, camera.yaw);
        let mut text = |s: &str, pos| {
            gfx.draw_text(assets, self.font, s, pos, 1.0, Color::WHITE, HUD_LAYER);
        };
        text(
            &self.status(),
            vec2(-half.x + HUD_MARGIN, half.y - HUD_MARGIN - HUD_PX),
        );
        if self.level.won {
            let font = assets.font(self.font);
            for (s, y) in [(WIN, HUD_PX), (RESTART, -HUD_PX)] {
                text(s, vec2(-text_width(font, s) / 2.0, y));
            }
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
        for cmd in mem::take(&mut self.queue) {
            match cmd {
                Cmd::Ambient(k) => self.ambient = k,
                Cmd::FogEnd(end) => self.fog_end = end,
            }
        }
        if ctx.input.just_pressed(KeyCode::F1) {
            self.panel = !self.panel;
            // И при закрытии: курсор остаётся у панели до клика в мир.
            ctx.cursor_grab = false;
        }
        // Клик, пришедший до потери фокуса, захватил бы курсор в фоновом окне.
        if ctx.input.mouse_just_pressed(MouseButton::Left) && ctx.focused && !self.panel {
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
        self.fps = ctx.time.fps;
        // После выхода `update` не крутит игрока, и непотреблённый сдвиг дёргал бы камеру до тика.
        let delta = if self.level.won {
            Vec2::ZERO
        } else {
            ctx.input.mouse_delta()
        };
        let camera = self.level.player.camera(alpha, delta);
        let scene = self.scene(camera, alpha, &ctx.assets);
        ctx.gfx.clear_color = scene.fog.color;
        ctx.gfx.scene = Some(scene);
        ctx.gfx.camera = Camera2D {
            center: Vec2::ZERO,
            size: VIEW,
        };
        self.draw_hud(&mut ctx.gfx, &ctx.assets, camera);
    }

    fn ui(&mut self, ui: &egui::Context) {
        engine::ui::set_font(ui, FONT_PATH);
        if !self.panel {
            return;
        }
        egui::Window::new("Отладка")
            .resizable(false)
            .show(ui, |ui| {
                panel::show(ui, self.fps, self.ambient, self.fog_end, &mut self.queue);
            });
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

    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use engine::{
        ElementState::{Pressed, Released},
        FixedClock, MAX_PITCH, Sprite, Vec2, Vec3,
        egui::accesskit::{Action, ActionData, ActionRequest, Role},
        ivec2, step_frame, text, vec2, vec3,
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
        assert_eq!(
            textures.len(),
            4,
            "белая, общая colormap, атлас шрифта, стрелка"
        );
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

    #[test]
    fn torch_light_hangs_by_banner_wall() {
        for (text, wall) in [
            ("P#.\n.T.\n...\n", Dir::N),
            (".P.\n.T#\n.#.\n", Dir::E),
            (".P.\n#T.\n.#.\n", Dir::S),
            (".P.\n#T.\n...\n", Dir::W),
        ] {
            let (g, ctx) = maze(text);
            let scene = g.scene(Camera3D::default(), 0.0, &ctx.assets);
            let o = wall.offset().as_vec2() * 0.25;
            let want = cell_center(ivec2(1, 1)) + vec3(o.x, TORCH_HEIGHT, o.y);
            assert_eq!(scene.lights.len(), 1, "{wall:?}");
            assert!(scene.lights[0].pos.abs_diff_eq(want, 1e-6), "{wall:?}");
            assert_eq!(scene.lights[0].range, TORCH_RANGE);
        }
    }

    #[test]
    fn every_torch_has_banner_and_light() {
        let (g, ctx) = maze(LEVEL01);
        let scene = g.scene(Camera3D::default(), 0.0, &ctx.assets);
        let torches: Vec<_> = g.map.torches.iter().map(|t| t.cell).collect();
        assert!(torches.len() > 1);
        assert_eq!(placed(&scene, g.models.banner), centers(torches.clone()));
        let cells: Vec<_> = (scene.lights.iter())
            .map(|l| vec2(l.pos.x, l.pos.z).floor().as_ivec2())
            .collect();
        assert_eq!(cells, torches);
    }

    #[test]
    fn flicker_stays_in_bounds_and_differs_between_torches() {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for i in 0..engine::MAX_LIGHTS {
            for tick in 0..10_000 {
                let b = flicker(tick, i);
                assert!((1.0 - FLICKER..=1.0).contains(&b), "{tick} {i}: {b}");
                lo = lo.min(b);
                hi = hi.max(b);
            }
        }
        assert!(
            lo < 1.0 - 0.9 * FLICKER && hi > 1.0 - 0.1 * FLICKER,
            "{lo}..{hi}"
        );
        assert!((0..60).any(|t| (flicker(t, 0) - flicker(t, 1)).abs() > 0.01));
        assert_ne!(flicker(0, 0), flicker(1, 0));
    }

    #[test]
    fn torch_light_flickers_with_ticks_not_alpha() {
        let (mut g, mut ctx) = maze(LEVEL01);
        let (mut twin, _) = maze(LEVEL01);
        let lights = |g: &mut Maze, ctx: &mut Context, alpha| {
            g.draw(ctx, alpha);
            ctx.gfx.scene.as_ref().unwrap().lights.clone()
        };
        let first = lights(&mut g, &mut ctx, 0.0);
        assert_eq!(lights(&mut g, &mut ctx, 0.9), first);
        let k = f64::from(flicker(0, 0));
        assert_eq!(
            (first[0].color.r, first[0].color.g, first[0].color.b),
            (TORCH.r * k, TORCH.g * k, TORCH.b * k)
        );
        for _ in 0..7 {
            tick(&mut g, &mut ctx);
            twin.tick += 1;
        }
        let later = lights(&mut g, &mut ctx, 0.0);
        assert_ne!(later, first);
        let twin = twin.scene(Camera3D::default(), 0.0, &ctx.assets);
        assert_eq!(twin.lights, later, "мерцание — функция тика");
        let k = f64::from(flicker(7, 1));
        assert_eq!(later[1].color.g, TORCH.g * k);
    }

    #[test]
    fn scene_has_ambient_and_fog_and_clears_with_fog_color() {
        let (mut g, mut ctx) = maze(LEVEL01);
        g.draw(&mut ctx, 0.0);
        let scene = ctx.gfx.scene.as_ref().unwrap();
        assert_eq!((scene.ambient, scene.fog), (AMBIENT, FOG));
        assert_eq!(ctx.gfx.clear_color, FOG.color);
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
    fn mouse_does_not_turn_camera_after_win() {
        let (mut g, mut ctx) = maze(LEVEL01);
        g.level.won = true;
        ctx.input.on_mouse_motion(vec2(100.0, -40.0));
        g.draw(&mut ctx, 0.0);
        let camera = ctx.gfx.scene.as_ref().unwrap().camera;
        assert_eq!((camera.yaw, camera.pitch), (g.level.player.yaw, 0.0));
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
        let (mut g, mut ctx) = maze("######\n#PoX.#\n######\n#E...#\n######\n");
        press(&mut ctx, KeyCode::Enter);
        tick(&mut g, &mut ctx);
        assert_eq!(
            g.level,
            Level {
                ticks: 1,
                orcs: g.level.orcs.clone(),
                ..Level::new(&g.map)
            },
            "Enter до победы ничего не делает"
        );
        ctx.input.on_key(KeyCode::Enter, Released, false);

        press(&mut ctx, KeyCode::KeyW);
        until(&mut g, &mut ctx, |g| g.level.won);
        assert_eq!(g.level.player.pos.floor().as_ivec2(), ivec2(3, 1));
        assert_eq!(g.level.coins, []);
        assert_eq!(g.level.player.prev, g.level.player.pos);
        assert!(g.level.orcs.iter().all(|o| o.prev == o.pos));
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
        model.pose(g.open, 21.0 / 60.0, false, &mut want);
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
        model.pose(g.walk, secs, true, &mut want);
        assert_eq!(d.pose.as_ref(), Some(&want));
        let mut later = Pose::default();
        model.pose(g.walk, o.walked / enemy::SPEED, true, &mut later);
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

    fn hud(gfx: &Gfx) -> Vec<Sprite> {
        (gfx.sprites.iter())
            .filter(|s| s.layer == HUD_LAYER)
            .copied()
            .collect()
    }

    #[test]
    fn hud_counts_coins_keys_and_time_and_centers_win() {
        let (mut g, mut ctx) = maze("#######\n#Pooo.#\n#######\n");
        g.level.coins.remove(1);
        (g.level.held, g.level.ticks) = (2, 150);
        g.draw(&mut ctx, 0.0);
        assert_eq!(
            ctx.gfx.camera,
            Camera2D {
                center: Vec2::ZERO,
                size: VIEW
            }
        );
        let playing = hud(&ctx.gfx);
        ctx.gfx.sprites.clear();
        g.level.won = true;
        g.draw(&mut ctx, 0.0);
        let won = hud(&ctx.gfx);

        let font = ctx.assets.font(g.font);
        let text = |s, pos| text::layout(font, s, pos, 1.0, Color::WHITE, HUD_LAYER);
        let status: Vec<_> = text("COINS 1/3\nKEYS 2\nTIME 2.5", vec2(-628.0, 324.0)).collect();
        assert_eq!(status.len(), "COINS1/3KEYS2TIME2.5".len());
        assert_eq!(playing, status);
        let centered = |s, y| text(s, vec2(-text_width(font, s) / 2.0, y));
        let want: Vec<_> = (status.into_iter())
            .chain(centered(WIN, HUD_PX))
            .chain(centered(RESTART, -HUD_PX))
            .collect();
        assert_eq!(won, want);
    }

    #[test]
    fn text_width_sums_advances() {
        let mut assets = Assets::default();
        let font = assets.load_font(FONT_PATH, HUD_PX);
        let font = assets.font(font);
        let one = text_width(font, "0");
        assert_eq!(one, font.glyph('0').unwrap().advance);
        assert!(one > 0.0);
        assert_eq!(text_width(font, "0\u{e9}0"), 3.0 * one);
    }

    #[test]
    fn play_reveals_minimap_and_draw_puts_arrow_at_camera() {
        let (mut g, mut ctx) = maze(LEVEL01);
        assert!(!g.minimap.seen(g.map.player));
        press(&mut ctx, KeyCode::KeyW);
        tick(&mut g, &mut ctx);
        assert!(g.minimap.seen(g.map.player));
        ctx.input.on_mouse_motion(vec2(30.0, 0.0));
        g.draw(&mut ctx, 0.5);
        let camera = ctx.gfx.scene.as_ref().unwrap().camera;
        let arrow: Vec<_> = (ctx.gfx.sprites.iter())
            .filter(|s| s.layer == minimap::ARROW_LAYER)
            .collect();
        let width = g.map.size.x as f32 * minimap::CELL;
        let origin = vec2(640.0 - HUD_MARGIN - width, 360.0 - HUD_MARGIN);
        assert_eq!(arrow.len(), 1);
        assert_eq!(
            arrow[0].pos,
            origin + vec2(camera.pos.x, -camera.pos.z) * minimap::CELL
        );
        assert_eq!(arrow[0].rotation, camera.yaw);
        assert_ne!(camera.yaw, g.level.player.yaw, "с непотреблённым сдвигом");
        assert_eq!(arrow[0].texture, g.arrow);
    }

    #[test]
    fn restart_keeps_minimap_and_resets_time() {
        let (mut g, mut ctx) = maze("######\n#PoX.#\n######\n");
        press(&mut ctx, KeyCode::KeyW);
        until(&mut g, &mut ctx, |g| g.level.won);
        let ticks = g.level.ticks;
        assert!(ticks > 0);
        for _ in 0..5 {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level.ticks, ticks, "после победы время стоит");
        let seen = g.minimap.clone();
        press(&mut ctx, KeyCode::Enter);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level.ticks, 0);
        assert_eq!(g.minimap, seen);
    }

    #[test]
    fn f1_toggles_panel_and_releases_grab_until_click() {
        let (mut g, mut ctx) = maze(LEVEL01);
        ctx.focused = true;
        ctx.cursor_grab = true;
        press(&mut ctx, KeyCode::F1);
        tick(&mut g, &mut ctx);
        assert!(g.panel);
        assert!(!ctx.cursor_grab);
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        assert!(!ctx.cursor_grab, "клик при панели не захватывает");
        ctx.input.on_button(MouseButton::Left, Released);

        ctx.input.on_key(KeyCode::F1, Released, false);
        press(&mut ctx, KeyCode::F1);
        tick(&mut g, &mut ctx);
        assert!(!g.panel);
        assert!(!ctx.cursor_grab, "закрытие без клика не захватывает");
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        assert!(ctx.cursor_grab);
    }

    #[test]
    fn closing_panel_releases_grab_taken_meanwhile() {
        let (mut g, mut ctx) = maze(LEVEL01);
        g.panel = true;
        ctx.cursor_grab = true;
        press(&mut ctx, KeyCode::F1);
        tick(&mut g, &mut ctx);
        assert!(!g.panel);
        assert!(!ctx.cursor_grab);
    }

    fn harness(g: Maze) -> Harness<'static, Maze> {
        let mut h = Harness::builder()
            .with_size(egui::vec2(500.0, 400.0))
            .build_ui_state(|ui, g: &mut Maze| g.ui(ui.ctx()), g);
        h.run();
        h
    }

    fn set_value(h: &Harness<'_, Maze>, label: &str, value: f64) {
        let slider = h.get_by_role_and_label(Role::Slider, label);
        let (target_node, target_tree) = slider.accesskit_node().locate();
        h.event(egui::Event::AccessKitActionRequest(ActionRequest {
            target_node,
            target_tree,
            action: Action::SetValue,
            data: Some(ActionData::NumericValue(value)),
        }));
    }

    #[test]
    fn panel_shows_fps_only_when_open() {
        let (mut g, _) = maze(LEVEL01);
        g.fps = 57;
        let h = harness(g);
        assert!(h.query_by_label("FPS 57").is_none());
        let (mut g, _) = maze(LEVEL01);
        (g.fps, g.panel) = (57, true);
        let h = harness(g);
        h.get_by_label("FPS 57");
        assert!(
            h.ctx
                .fonts(|f| f.definitions().font_data.contains_key(FONT_PATH))
        );
    }

    #[test]
    fn sliders_queue_commands_applied_in_update() {
        let (mut g, mut ctx) = maze(LEVEL01);
        g.panel = true;
        let mut h = harness(g);
        set_value(&h, "ambient ×", 3.0);
        h.run();
        set_value(&h, "туман, м", 12.0);
        h.run();
        let mut g = h.into_state();
        assert_eq!(g.queue, [Cmd::Ambient(3.0), Cmd::FogEnd(12.0)]);
        assert_eq!(
            (g.ambient, g.fog_end),
            (1.0, FOG.end),
            "до update не применены"
        );
        tick(&mut g, &mut ctx);
        assert_eq!(g.queue, []);
        g.draw(&mut ctx, 0.0);
        let scene = ctx.gfx.scene.as_ref().unwrap();
        assert_eq!(scene.ambient, scaled(AMBIENT, 3.0));
        assert_eq!(scene.fog, Fog { end: 12.0, ..FOG });
        assert_eq!(ctx.gfx.clear_color, FOG.color);
    }

    #[test]
    fn fog_slider_stops_short_of_fog_start() {
        let (mut g, _) = maze(LEVEL01);
        g.panel = true;
        let mut h = harness(g);
        set_value(&h, "туман, м", 0.0);
        h.run();
        let end = FOG.start + 0.5;
        assert_eq!(h.into_state().queue, [Cmd::FogEnd(end)]);
    }

    #[test]
    fn untouched_sliders_queue_nothing() {
        let (mut g, _) = maze(LEVEL01);
        g.panel = true;
        let mut h = harness(g);
        h.run();
        assert_eq!(h.into_state().queue, []);
    }

    #[test]
    fn scaled_multiplies_rgb_keeps_alpha() {
        let c = Color {
            r: 0.5,
            g: 0.25,
            b: 1.0,
            a: 0.5,
        };
        assert_eq!(
            scaled(c, 2.0),
            Color {
                r: 1.0,
                g: 0.5,
                b: 2.0,
                a: 0.5
            }
        );
    }
}
