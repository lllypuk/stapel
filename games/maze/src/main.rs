mod map;
mod player;

use std::f32::consts::FRAC_PI_2;
use std::fs;

use engine::{
    Assets, Camera3D, Color, Config, Context, Game, Handle, IVec2, KeyCode, Mat4, Model, ModelDraw,
    MouseButton, Quat, Scene,
};
use map::{Dir, Map, cell_center};
use player::Player;

const TICK_HZ: u32 = 60;
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

struct Maze {
    map: Map,
    models: Models,
    /// Стены, пол и баннеры: за уровень не меняются.
    statics: Vec<ModelDraw>,
    player: Player,
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
        let player = Player::new(map.player.as_vec2() + 0.5, map.facing.yaw());
        Self {
            map,
            models,
            statics,
            player,
        }
    }

    /// Дверь в модели поперёк Z: в коридоре вдоль X её поворачиваем.
    fn gate_yaw(&self, cell: IVec2) -> f32 {
        let along_z =
            self.map.wall(cell + Dir::E.offset()) || self.map.wall(cell + Dir::W.offset());
        if along_z { 0.0 } else { FRAC_PI_2 }
    }

    /// Закрытые ворота твёрдые; край карты тоже.
    fn solid(map: &Map, cell: IVec2) -> bool {
        !map.floor(cell) || map.gates.contains(&cell)
    }

    fn scene(&self, camera: Camera3D) -> Scene {
        let m = &self.models;
        let mut models = self.statics.clone();
        let items = [
            (m.coin, &self.map.coins),
            (m.key, &self.map.keys),
            (m.stairs, &self.map.exits),
            (m.orc, &self.map.orcs),
        ];
        for (model, cells) in items {
            models.extend(cells.iter().map(|&c| at(model, c, 0.0)));
        }
        models.extend(
            self.map
                .gates
                .iter()
                .map(|&c| at(m.gate, c, self.gate_yaw(c))),
        );
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
        let map = &self.map;
        self.player
            .update(&ctx.input, ctx.time.step.as_secs_f32(), |c| {
                Self::solid(map, c)
            });
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        let camera = self.player.camera(alpha, ctx.input.mouse_delta());
        ctx.gfx.scene = Some(self.scene(camera));
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

    use crate::player::{EYE, RADIUS, SENSITIVITY};

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
            g.scene(Camera3D {
                pos: vec3(p.x, EYE, p.z),
                yaw: g.map.facing.yaw(),
                ..Camera3D::default()
            })
        );
        assert_eq!(scene.ambient, AMBIENT);
    }

    #[test]
    fn camera_looks_at_open_neighbour() {
        let (g, _) = maze("###\n#P.\n###\n");
        let camera = g.player.camera(1.0, Vec2::ZERO);
        assert_eq!(camera.yaw, Dir::E.yaw());
        assert!(camera.forward().abs_diff_eq(Vec3::X, 1e-6));
    }

    #[test]
    fn walls_and_floor_cover_every_cell() {
        let (g, _) = maze("####\n#Po#\n#kE#\n#DX#\n####\n");
        let scene = g.scene(Camera3D::default());
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
        let (g, _) = maze("####\n#Po#\n#kE#\n#DX#\n####\n");
        let scene = g.scene(Camera3D::default());
        let m = &g.models;
        assert_eq!(placed(&scene, m.coin), centers([ivec2(2, 1)]));
        assert_eq!(placed(&scene, m.key), centers([ivec2(1, 2)]));
        assert_eq!(placed(&scene, m.orc), centers([ivec2(2, 2)]));
        assert_eq!(placed(&scene, m.gate), centers([ivec2(1, 3)]));
        assert_eq!(placed(&scene, m.stairs), centers([ivec2(2, 3)]));
        assert!(scene.models.iter().all(|d| d.pose.is_none()));
    }

    #[test]
    fn gate_spans_corridor() {
        let across = |text| {
            let (g, _) = maze(text);
            let d = draws(&g.scene(Camera3D::default()), g.models.gate)
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
            let (g, _) = maze(text);
            let scene = g.scene(Camera3D::default());
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
        let yaw = g.player.yaw;
        let camera = |ctx: &Context| ctx.gfx.scene.as_ref().unwrap().camera;

        ctx.input.on_mouse_motion(vec2(100.0, -40.0));
        assert_eq!(step_frame(&mut g, &mut ctx, &mut clock, step * 2), 2);
        assert_eq!(g.player.yaw, turned(yaw, 100.0));
        assert_eq!(g.player.pitch, 40.0 * SENSITIVITY);
        assert_eq!(camera(&ctx).yaw, g.player.yaw);

        let before = g.player;
        ctx.input.on_mouse_motion(vec2(50.0, 0.0));
        assert_eq!(step_frame(&mut g, &mut ctx, &mut clock, Duration::ZERO), 0);
        assert_eq!(camera(&ctx).yaw, turned(before.yaw, 50.0));
        assert_eq!(camera(&ctx).pitch, before.pitch);
        g.draw(&mut ctx, clock.alpha());
        assert_eq!(camera(&ctx).yaw, turned(before.yaw, 50.0));
        assert_eq!(g.player, before);

        assert_eq!(step_frame(&mut g, &mut ctx, &mut clock, step), 1);
        assert_eq!(g.player.yaw, turned(before.yaw, 50.0));
        assert_eq!(camera(&ctx).yaw, g.player.yaw);
    }

    #[test]
    fn draw_interpolates_position_without_changing_state() {
        let (mut g, mut ctx) = maze("###\n#P.\n#..\n");
        press(&mut ctx, KeyCode::KeyW);
        g.update(&mut ctx);
        let p = g.player;
        assert_ne!(p.prev, p.pos);
        g.draw(&mut ctx, 0.25);
        let pos = ctx.gfx.scene.as_ref().unwrap().camera.pos;
        let at = p.prev.lerp(p.pos, 0.25);
        assert!(pos.abs_diff_eq(vec3(at.x, EYE, at.y), 1e-6), "{pos:?}");
        assert_eq!(g.player, p);
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
            (g.player.pos.x - (2.0 - RADIUS)).abs() < 1e-5,
            "{:?}",
            g.player.pos
        );
        let (mut g, mut ctx) = maze("#####\n#P..#\n#####\n");
        press(&mut ctx, KeyCode::KeyW);
        for _ in 0..60 {
            g.update(&mut ctx);
        }
        assert!(
            (g.player.pos.x - (4.0 - RADIUS)).abs() < 1e-5,
            "без ворот — до стены"
        );
    }
}
