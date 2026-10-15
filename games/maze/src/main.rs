mod map;

use std::f32::consts::FRAC_PI_2;
use std::fs;

use engine::{
    Assets, Camera3D, Color, Config, Context, Game, Handle, IVec2, Mat4, Model, ModelDraw, Quat,
    Scene, Vec3,
};
use map::{Dir, Map, cell_center};

const TICK_HZ: u32 = 60;
/// Высота глаза над полом, м.
const EYE: f32 = 0.7;
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
    camera: Camera3D,
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
        let camera = Camera3D {
            pos: cell_center(map.player) + Vec3::Y * EYE,
            yaw: map.facing.yaw(),
            ..Camera3D::default()
        };
        Self {
            map,
            models,
            statics,
            camera,
        }
    }

    /// Дверь в модели поперёк Z: в коридоре вдоль X её поворачиваем.
    fn gate_yaw(&self, cell: IVec2) -> f32 {
        let along_z =
            self.map.wall(cell + Dir::E.offset()) || self.map.wall(cell + Dir::W.offset());
        if along_z { 0.0 } else { FRAC_PI_2 }
    }

    fn scene(&self) -> Scene {
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
            camera: self.camera,
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

    fn update(&mut self, _ctx: &mut Context) {}

    fn draw(&mut self, ctx: &mut Context, _alpha: f32) {
        ctx.gfx.scene = Some(self.scene());
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
    use engine::{ivec2, vec3};

    use super::*;

    fn maze(text: &str) -> (Maze, Context) {
        let mut ctx = Context::default();
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
        assert_eq!(*scene, g.scene());
        let p = cell_center(g.map.player);
        assert_eq!(scene.camera.pos, vec3(p.x, EYE, p.z));
        assert_eq!(scene.camera.yaw, g.map.facing.yaw());
        assert_eq!(scene.ambient, AMBIENT);
    }

    #[test]
    fn camera_looks_at_open_neighbour() {
        let (g, _) = maze("###\n#P.\n###\n");
        assert_eq!(g.camera.yaw, Dir::E.yaw());
        assert!(g.camera.forward().abs_diff_eq(Vec3::X, 1e-6));
    }

    #[test]
    fn walls_and_floor_cover_every_cell() {
        let (g, _) = maze("####\n#Po#\n#kE#\n#DX#\n####\n");
        let scene = g.scene();
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
        let scene = g.scene();
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
            let d = draws(&g.scene(), g.models.gate).next().unwrap().clone();
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
            let scene = g.scene();
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
}
