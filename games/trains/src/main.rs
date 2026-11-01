#[cfg_attr(not(test), expect(dead_code, reason = "бот ездит только в тестах"))]
mod bot;
mod consist;
mod depot;
mod piece;
mod switch;
mod track;
mod train;

use std::collections::BTreeMap;
use std::f32::consts::FRAC_PI_4;

use stapel::{
    Assets, Camera2D, Camera3D, Color, Config, Context, DirectionalLight, Font, Game, Gfx, Handle,
    Input, KeyCode, Mat4, Mesh, MeshDraw, Model, ModelDraw, MouseButton, Quat, Scene, Vec2, Vec3,
    Vertex, egui, vec2, vec3,
};

use consist::Car;
use depot::Depot;
use piece::RAIL_TOP;
use track::{Building, CELL, LEVEL01, MARGIN, Road, Track};
use train::{Coupler, Placed, Train};

const TICK_HZ: u32 = 60;
const DT: f32 = 1.0 / TICK_HZ as f32;
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
const FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Regular.ttf"
);
/// HUD в единицах окна 1280×720; иной аспект — letterbox.
const VIEW: Vec2 = vec2(1280.0, 720.0);
/// При `VIEW` в окне 1280×720 тексель атласа — пиксель экрана.
const HUD_PX: f32 = 24.0;
const HUD_MARGIN: f32 = 12.0;
const HUD_LAYER: i32 = 1;
/// Атлас шрифта — только ASCII, поэтому HUD по-английски.
const BUSY: &str = "SWITCH BUSY";
const GAS: &[KeyCode] = &[KeyCode::KeyW, KeyCode::ArrowUp];
const BACK: &[KeyCode] = &[KeyCode::KeyS, KeyCode::ArrowDown];
/// Состав в депо на старте.
const START: [&str; 3] = [
    "train/train-locomotive-a",
    "train/train-carriage-box",
    "train/train-carriage-coal",
];
/// Подкаталог набора и имя GLB без расширения.
const MODELS: [&str; 34] = [
    "train/railroad-straight",
    "train/railroad-corner-large",
    "train/railroad-corner-small",
    "train/railroad-curve",
    "train/train-connector",
    "train/train-locomotive-a",
    "train/train-diesel-a",
    "train/train-diesel-box-a",
    "train/train-electric-double-a",
    "train/train-electric-bullet-a",
    "train/train-carriage-box",
    "train/train-carriage-coal",
    "train/train-carriage-container-blue",
    "train/train-carriage-container-red",
    "train/train-carriage-flatbed-wood",
    "train/train-carriage-lumber",
    "train/train-carriage-tank",
    "train/train-carriage-wood",
    "road/road-straight",
    "road/road-corner",
    "road/road-crossing",
    "road/road-end",
    "road/grass",
    "road/pond",
    "road/pool",
    "road/plaza",
    "city/building-a",
    "city/building-b",
    "city/building-c",
    "city/building-d",
    "city/building-e",
    "city/building-f",
    "city/building-skyscraper-a",
    "city/building-skyscraper-b",
];
/// Изометрия: взгляд по диагонали клетки.
const YAW: f32 = FRAC_PI_4;
/// Наклон изометрии, `−atan(1/√2)`.
const PITCH: f32 = -35.264_f32.to_radians();
/// Видимая высота кадра ортокамеры, м.
const VIEW_HEIGHT: f32 = 16.0;
/// Отвод камеры от цели вдоль взгляда: сцена должна быть перед `near`; кадр от него не зависит, туман зависел бы.
const CAMERA_BACK: f32 = 50.0;
/// Подъём активной ветки стрелки над неактивной: иначе перекрытые детали мерцают.
const SWITCH_LIFT: f32 = 0.01;
const INACTIVE: Color = Color {
    r: 0.45,
    g: 0.45,
    b: 0.45,
    a: 1.0,
};
/// Глубина `train-connector` по z: сцепка растягивается на зазор.
const CONNECTOR_DEPTH: f32 = 0.2;
/// Плитка Road Tiles 3×3 — в клетку пути.
const TILE_SCALE: f32 = CELL / 3.0;
/// Верх травы плитки в её осях.
const GRASS_TOP: f32 = 0.55;
/// Здания City Kit мельче вагона.
const CITY_SCALE: f32 = 2.2;
/// Земля ниже травы плиток: иначе они мерцают.
const GROUND_DROP: f32 = 0.02;
/// Поле земли за краем пути, м.
const GROUND_MARGIN: f32 = 20.0;
/// Цвет травы Road Tiles.
const GROUND: Color = Color {
    r: 0.541,
    g: 0.71,
    b: 0.286,
    a: 1.0,
};
const SUN: DirectionalLight = DirectionalLight {
    dir: vec3(-0.4, -1.0, -0.6),
    color: Color::WHITE,
};
const AMBIENT: Color = Color {
    r: 0.35,
    g: 0.35,
    b: 0.4,
    a: 1.0,
};
const SKY: Color = Color {
    r: 0.45,
    g: 0.62,
    b: 0.8,
    a: 1.0,
};

struct Trains {
    models: BTreeMap<&'static str, Handle<Model>>,
    /// Локомотивы и вагоны по имени модели.
    cars: BTreeMap<&'static str, Car>,
    track: Track,
    train: Train,
    /// Состав прошлого тика: сцена — между ним и `train`.
    prev: Train,
    ground: Handle<Mesh>,
    depot: Depot,
    font: Handle<Font>,
    /// Тиков до конца надписи об отказе стрелки.
    busy: u32,
}

fn load_models(assets: &mut Assets) -> BTreeMap<&'static str, Handle<Model>> {
    MODELS
        .iter()
        .map(|&name| (name, assets.load_model(format!("{ASSETS}{name}.glb"))))
        .collect()
}

/// Модели с тележками из `MODELS`: локомотивы и вагоны.
fn catalog(
    models: &BTreeMap<&'static str, Handle<Model>>,
    assets: &Assets,
) -> BTreeMap<&'static str, Car> {
    (models.iter())
        .filter_map(|(&n, &h)| Some((n, Car::new(h, assets.model(h))?)))
        .collect()
}

/// Состав по именам в начале депо: хвост у тупика.
fn in_depot(track: &Track, cars: &BTreeMap<&'static str, Car>, consist: &[&str]) -> Train {
    let cars = (consist.iter())
        .map(|n| *cars.get(n).unwrap_or_else(|| panic!("{n}: нет в каталоге")))
        .collect();
    Train::new(track, track.depot[0], MARGIN, cars).expect("депо вмещает состав")
}

fn pressed(input: &Input, keys: &[KeyCode]) -> f32 {
    f32::from(u8::from(keys.iter().any(|&k| input.is_down(k))))
}

/// Квад 1×1 в плоскости y = 0 от начала по +X и +Z.
fn quad() -> (Vec<Vertex>, Vec<u32>) {
    let vertex = |x: f32, z: f32| Vertex {
        pos: [x, 0.0, z],
        normal: [0.0, 1.0, 0.0],
        ..Vertex::default()
    };
    let vertices = vec![
        vertex(0.0, 0.0),
        vertex(0.0, 1.0),
        vertex(1.0, 1.0),
        vertex(1.0, 0.0),
    ];
    (vertices, vec![0, 1, 2, 0, 2, 3])
}

/// Поворот вокруг Y, уводящий +Z в `dir` на плоскости (x, z).
fn yaw(dir: Vec2) -> f32 {
    dir.x.atan2(dir.y)
}

/// Низ колёс Train Kit — y = 0 модели: машина и сцепка встают на верх рельса.
fn on_rails(p: Vec2) -> Mat4 {
    Mat4::from_translation(vec3(p.x, RAIL_TOP, p.y))
}

fn lerp_dir(a: Vec2, b: Vec2, t: f32) -> Vec2 {
    a.lerp(b, t).normalize()
}

fn lerp_car(a: &Placed, b: &Placed, t: f32) -> Placed {
    Placed {
        bogies: [0, 1].map(|i| {
            let ((pa, da), (pb, db)) = (a.bogies[i], b.bogies[i]);
            (pa.lerp(pb, t), lerp_dir(da, db, t))
        }),
        pos: a.pos.lerp(b.pos, t),
        dir: lerp_dir(a.dir, b.dir, t),
    }
}

/// Середина от переднего края головной машины до заднего края последней.
fn middle(placed: &[Placed], cars: &[Car]) -> Vec2 {
    let (first, last) = (placed[0], placed[placed.len() - 1]);
    let (head, tail) = (cars[0], cars[cars.len() - 1]);
    let front = first.pos + first.dir * head.front;
    let back = last.pos + last.dir * (tail.front - tail.length);
    front.midpoint(back)
}

/// Машина на рельсах; тележки — по касательной, колёса паровоза без тележки — нет.
fn car_draw(car: &Car, placed: &Placed, model: &Model) -> ModelDraw {
    let mut pose = model.rest.clone();
    for (name, (_, tangent)) in ["wheels-front", "wheels-back"]
        .into_iter()
        .zip(placed.bogies)
    {
        if let Some(node) = model.nodes.iter().position(|n| n.name == name) {
            let local = pose.local_mut(node);
            local.rotation = Quat::from_rotation_y(yaw(tangent) - yaw(placed.dir)) * local.rotation;
        }
    }
    model.recompose(&mut pose);
    ModelDraw {
        model: car.model,
        transform: on_rails(placed.pos) * Mat4::from_rotation_y(yaw(placed.dir)),
        pose: Some(pose),
        tint: Color::WHITE,
    }
}

/// `train-connector` по хорде сцепки на всю её длину.
fn coupler_transform([a, b]: Coupler) -> Mat4 {
    let d = b - a;
    on_rails(a.midpoint(b))
        * Mat4::from_rotation_y(yaw(d))
        * Mat4::from_scale(vec3(1.0, 1.0, d.length() / CONNECTOR_DEPTH))
}

/// Плитка на клетку `cell` с травой на y = 0, поворот — вокруг центра клетки.
fn road_transform(road: &Road) -> Mat4 {
    let c = (road.cell.as_vec2() + 0.5) * CELL;
    Mat4::from_translation(vec3(c.x, -GRASS_TOP * TILE_SCALE, c.y))
        * Mat4::from_rotation_y(road.rot.radians())
        * Mat4::from_scale(Vec3::splat(TILE_SCALE))
        * Mat4::from_translation(vec3(-1.5, 0.0, 1.5))
}

fn building_transform(b: &Building) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::splat(CITY_SCALE * b.scale),
        Quat::from_rotation_y(b.rot.to_radians()),
        vec3(b.pos.x, 0.0, b.pos.y),
    )
}

/// Квад земли под путём с полем `GROUND_MARGIN`: дороги карты — внутри петли.
fn ground_transform(track: &Track) -> Mat4 {
    let points = track.ports.iter().map(|p| p.pos);
    let (min, max) = points.fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(lo, hi), p| {
        (lo.min(p), hi.max(p))
    });
    let (min, size) = (min - GROUND_MARGIN, max - min + 2.0 * GROUND_MARGIN);
    Mat4::from_translation(vec3(min.x, -GROUND_DROP, min.y))
        * Mat4::from_scale(vec3(size.x, 1.0, size.y))
}

/// Ветка стрелки: `Some(true)` — активная, `Some(false)` — неактивная хоть у одной стрелки.
fn branch(track: &Track, edge: usize) -> Option<bool> {
    let mut active = None;
    for s in &track.switches {
        if let Some(i) = s.branches.iter().position(|&b| b == edge) {
            if i != s.active {
                return Some(false);
            }
            active = Some(true);
        }
    }
    active
}

/// Изометрическая ортокамера, смотрящая на `target`.
fn camera(target: Vec3) -> Camera3D {
    let mut camera = Camera3D {
        yaw: YAW,
        pitch: PITCH,
        ortho: Some(VIEW_HEIGHT),
        ..Camera3D::default()
    };
    camera.pos = target - camera.forward() * CAMERA_BACK;
    camera
}

impl Game for Trains {
    fn init(ctx: &mut Context) -> Self {
        let models = load_models(&mut ctx.assets);
        let cars = catalog(&models, &ctx.assets);
        let all: Vec<_> = cars.values().copied().collect();
        let track = Track::parse(LEVEL01, consist::max_len(&all)).expect("level01");
        let depot = Depot::new(START.to_vec());
        let train = in_depot(&track, &cars, &depot.consist);
        let (vertices, indices) = quad();
        Self {
            models,
            cars,
            track,
            prev: train.clone(),
            train,
            ground: ctx.assets.add_mesh(vertices, indices),
            depot,
            font: ctx.assets.load_font(FONT_PATH, HUD_PX),
            busy: 0,
        }
    }

    fn update(&mut self, ctx: &mut Context) {
        let input = &ctx.input;
        if self.depot.apply() {
            self.train = in_depot(&self.track, &self.cars, &self.depot.consist);
        }
        self.prev.clone_from(&self.train);
        if input.just_pressed(KeyCode::Tab) {
            self.depot.open = !self.depot.open && depot::can_open(&self.track, &self.train);
        }
        let ahead = (input.just_pressed(KeyCode::KeyE))
            .then(|| switch::ahead(&self.track, &self.train))
            .flatten();
        let clicked = (input.mouse_just_pressed(MouseButton::Left))
            .then(|| switch::pick(&self.track, ctx.cursor_ray()?))
            .flatten()
            .filter(|&c| Some(c) != ahead);
        self.busy = self.busy.saturating_sub(1);
        for s in ahead.into_iter().chain(clicked) {
            if !switch::toggle(&mut self.track, &self.train, s) {
                self.busy = TICK_HZ;
            }
        }
        let pedal = if self.depot.open {
            0.0
        } else {
            pressed(input, GAS) - pressed(input, BACK)
        };
        self.train.drive(pedal, DT);
        self.train.advance(DT, &self.track);
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        ctx.gfx.clear_color = SKY;
        ctx.gfx.scene = Some(self.scene(alpha, &ctx.assets));
        self.draw_hud(&mut ctx.gfx, &ctx.assets);
    }

    fn ui(&mut self, ui: &egui::Context) {
        stapel::ui::set_font(ui, FONT_PATH);
        if self.depot.open {
            let catalog: Vec<_> = self.cars.keys().copied().collect();
            self.depot.ui(ui, &catalog);
        }
    }
}

impl Trains {
    /// Путь, состав между тиками, окружение; камера — за серединой состава.
    fn scene(&self, alpha: f32, assets: &Assets) -> Scene {
        let (was, was_couplers) = self.prev.cars_at(&self.track);
        let (now, couplers) = self.train.cars_at(&self.track);
        let placed: Vec<_> = (was.iter().zip(&now))
            .map(|(a, b)| lerp_car(a, b, alpha))
            .collect();
        let target = middle(&placed, &self.train.cars);
        let rails = self.track.edges.iter().enumerate().map(|(i, e)| {
            let branch = branch(&self.track, i);
            let lift = if branch == Some(true) {
                SWITCH_LIFT
            } else {
                0.0
            };
            ModelDraw {
                model: self.models[e.rail.piece.model()],
                transform: Mat4::from_translation(Vec3::Y * lift) * e.rail.transform(),
                pose: None,
                tint: if branch == Some(false) {
                    INACTIVE
                } else {
                    Color::WHITE
                },
            }
        });
        let cars = (self.train.cars.iter().zip(&placed))
            .map(|(car, p)| car_draw(car, p, assets.model(car.model)));
        let still = |model, transform| ModelDraw {
            model,
            transform,
            pose: None,
            tint: Color::WHITE,
        };
        let connector = self.models["train/train-connector"];
        let couplers = (was_couplers.iter().zip(&couplers)).map(|(a, b)| {
            still(
                connector,
                coupler_transform([0, 1].map(|i| a[i].lerp(b[i], alpha))),
            )
        });
        let roads =
            (self.track.roads.iter()).map(|r| still(self.models[r.tile], road_transform(r)));
        let buildings = (self.track.buildings.iter())
            .map(|b| still(self.models[b.model], building_transform(b)));
        Scene {
            camera: camera(vec3(target.x, 0.0, target.y)),
            ambient: AMBIENT,
            sun: Some(SUN),
            meshes: vec![MeshDraw {
                mesh: self.ground,
                texture: Handle::WHITE,
                color: GROUND,
                transform: ground_transform(&self.track),
            }],
            models: rails
                .chain(cars)
                .chain(couplers)
                .chain(roads)
                .chain(buildings)
                .collect(),
            ..Scene::default()
        }
    }

    /// Скорость — слева сверху, отказ стрелки — по центру сверху.
    fn draw_hud(&self, gfx: &mut Gfx, assets: &Assets) {
        gfx.camera = Camera2D {
            center: Vec2::ZERO,
            size: VIEW,
        };
        let top = VIEW.y / 2.0 - HUD_MARGIN - HUD_PX;
        let speed = format!("{:3.0} KM/H", self.train.speed.abs() * 3.6);
        let pos = vec2(HUD_MARGIN - VIEW.x / 2.0, top);
        gfx.draw_text(assets, self.font, &speed, pos, 1.0, Color::WHITE, HUD_LAYER);
        if self.busy > 0 {
            let x = -assets.font(self.font).width(BUSY) / 2.0;
            gfx.draw_text(
                assets,
                self.font,
                BUSY,
                vec2(x, top),
                1.0,
                Color::WHITE,
                HUD_LAYER,
            );
        }
    }
}

fn main() -> Result<(), stapel::EventLoopError> {
    stapel::run::<Trains>(Config {
        title: "trains".into(),
        size: (1280, 720),
        tick_hz: TICK_HZ,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: false,
    })
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use stapel::{ElementState, Fog, Shown, ShownScene, Sprite, Texture, UVec2, Viewport, text};

    use super::*;
    use crate::depot::DepotCmd;
    use crate::piece::Piece;
    use crate::track::fixture::{self, edge};
    use crate::track::level01::{A, D};
    use crate::train::MAX_SPEED;

    fn texture(
        assets: &Assets,
        name: &str,
        models: &BTreeMap<&str, Handle<Model>>,
    ) -> Handle<Texture> {
        assets.model(models[name]).nodes[0].primitives[0].texture
    }

    #[test]
    fn every_model_loads_with_volume() {
        let mut ctx = Context::default();
        let g = Trains::init(&mut ctx);
        assert_eq!(g.models.len(), MODELS.len());
        let mut seen = Vec::new();
        for (name, &h) in &g.models {
            let b = ctx
                .assets
                .model(h)
                .bounds()
                .unwrap_or_else(|| panic!("{name}"));
            assert!(b.max.cmpgt(b.min).all(), "{name}: {b:?}");
            assert!(!seen.contains(&h), "{name} — повтор");
            seen.push(h);
        }
    }

    #[test]
    fn sets_keep_their_own_textures() {
        let mut ctx = Context::default();
        let m = load_models(&mut ctx.assets);
        let tex = |name| texture(&ctx.assets, name, &m);
        let train = tex("train/train-diesel-a");
        let city = tex("city/building-a");
        assert_ne!(train, Handle::WHITE);
        assert_ne!(city, Handle::WHITE);
        assert_ne!(train, city);
        assert_eq!(tex("train/railroad-straight"), train);
        assert_eq!(tex("city/building-skyscraper-a"), city);
        assert_eq!(tex("road/road-straight"), Handle::WHITE);
    }

    fn scene(g: &mut Trains, ctx: &mut Context, alpha: f32) -> Scene {
        g.draw(ctx, alpha);
        ctx.gfx.scene.take().expect("сцена")
    }

    fn target(s: &Scene) -> Vec3 {
        s.camera.pos + s.camera.forward() * CAMERA_BACK
    }

    fn near(a: Vec3, b: Vec3) {
        assert!(a.abs_diff_eq(b, 1e-4), "{a} ≠ {b}");
    }

    fn flat(p: Vec2, y: f32) -> Vec3 {
        vec3(p.x, y, p.y)
    }

    fn xz(v: Vec3) -> Vec2 {
        vec2(v.x, v.z)
    }

    #[test]
    fn camera_is_isometric_and_follows_consist_middle_between_ticks() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let s = scene(&mut g, &mut ctx, 0.0);
        assert_eq!(s.camera.ortho, Some(VIEW_HEIGHT));
        let look = s.camera.forward();
        assert!((look.x - look.z).abs() < 1e-5 && look.x < 0.0, "{look}");
        assert!((look.y + 1.0 / 3f32.sqrt()).abs() < 1e-4, "{look}");
        let (edge, forward) = g.track.depot[0];
        let (rail, len) = (g.track.edges[edge].rail, g.track.edges[edge].len());
        let (tail, dir) = if forward {
            rail.point_at(MARGIN)
        } else {
            let (p, d) = rail.point_at(len - MARGIN);
            (p, -d)
        };
        let middle = tail + dir * consist::len(&g.train.cars) / 2.0;
        near(target(&s), flat(middle, 0.0));
        g.depot.open = false;
        g.train.speed = MAX_SPEED;
        tick(&mut g, &mut ctx);
        let moved = g.train.tail - MARGIN;
        assert!(moved > 0.05, "{moved}");
        near(target(&scene(&mut g, &mut ctx, 0.0)), flat(middle, 0.0));
        near(
            target(&scene(&mut g, &mut ctx, 1.0)),
            flat(middle + dir * moved, 0.0),
        );
        let half = scene(&mut g, &mut ctx, 0.5);
        near(target(&half), flat(middle + dir * moved / 2.0, 0.0));
        let n = g.track.edges.len();
        let (was, _) = g.prev.cars_at(&g.track);
        let (now, _) = g.train.cars_at(&g.track);
        for (i, d) in half.models[n..n + now.len()].iter().enumerate() {
            let at = d.transform.transform_point3(Vec3::ZERO);
            near(at, flat(was[i].pos.midpoint(now[i].pos), RAIL_TOP));
        }
    }

    #[test]
    fn active_branch_is_lifted_and_inactive_dimmed() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        const { assert!(SWITCH_LIFT > 0.0 && INACTIVE.r < 1.0 && INACTIVE.a == 1.0) };
        for active in [0, 1] {
            for s in &mut g.track.switches {
                s.active = active;
            }
            let s = scene(&mut g, &mut ctx, 0.0);
            let lifted = Mat4::from_translation(Vec3::Y * SWITCH_LIFT);
            for (i, e) in g.track.edges.iter().enumerate() {
                let d = &s.models[i];
                assert_eq!(d.model, g.models[e.rail.piece.model()]);
                let sw = g.track.switches.iter().find(|s| s.branches.contains(&i));
                let (transform, tint) = match sw {
                    Some(s) if s.branches[s.active] == i => {
                        (lifted * e.rail.transform(), Color::WHITE)
                    }
                    Some(_) => (e.rail.transform(), INACTIVE),
                    None => (e.rail.transform(), Color::WHITE),
                };
                assert_eq!(
                    (d.transform, d.tint, d.pose.as_ref()),
                    (transform, tint, None),
                    "{i}"
                );
            }
        }
    }

    #[test]
    fn branch_of_two_switches_is_inactive_if_inactive_at_either() {
        let mut t = fixture::map(&[
            ("straight", [0, -2], 0, false),
            ("straight", [0, 0], 0, false),
            ("curve", [0, 0], 0, false),
            ("straight", [-1, 0], 0, false),
            ("straight", [-1, 2], 0, false),
        ]);
        let curve = 2;
        assert_eq!(t.switches.len(), 2);
        assert!(t.switches.iter().all(|s| s.branches.contains(&curve)));
        for (on, want) in [
            ([true, true], Some(true)),
            ([true, false], Some(false)),
            ([false, true], Some(false)),
        ] {
            for (s, on) in t.switches.iter_mut().zip(on) {
                let i = s.branches.iter().position(|&b| b == curve).expect("ветка");
                s.active = if on { i } else { 1 - i };
            }
            assert_eq!(branch(&t, curve), want, "{on:?}");
        }
        assert_eq!(branch(&t, 0), None);
    }

    #[test]
    fn bogies_turn_along_tangent_locomotive_wheels_do_not() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let cars: Vec<_> = g.cars.values().copied().collect();
        let start = edge(&g.track, Piece::Straight, [14, 0]);
        g.train = Train::new(&g.track, (start, true), 0.0, cars.clone()).expect("состав");
        g.prev = g.train.clone();
        let s = scene(&mut g, &mut ctx, 1.0);
        let n = g.track.edges.len();
        let (placed, couplers) = g.train.cars_at(&g.track);
        let (mut turned, mut wheels) = (0, 0);
        for ((car, p), d) in cars.iter().zip(&placed).zip(&s.models[n..]) {
            assert_eq!(d.model, car.model);
            let model = ctx.assets.model(car.model);
            let pose = d.pose.as_ref().expect("поза");
            near(
                d.transform.transform_point3(Vec3::ZERO),
                flat(p.pos, RAIL_TOP),
            );
            near(d.transform.transform_vector3(Vec3::Z), flat(p.dir, 0.0));
            let node = |name| model.nodes.iter().position(|n| n.name == name);
            for (name, (point, tangent)) in
                ["wheels-front", "wheels-back"].into_iter().zip(p.bogies)
            {
                let Some(bogie) = node(name) else {
                    assert_eq!(name, "wheels-back");
                    continue;
                };
                let world = d.transform * pose.globals[bogie];
                near(world.transform_vector3(Vec3::Z), flat(tangent, 0.0));
                if name == "wheels-front" {
                    let at = world.transform_point3(Vec3::ZERO);
                    near(vec3(at.x, 0.0, at.z), flat(point, 0.0));
                }
                turned += usize::from(tangent.angle_to(p.dir).abs() > 0.05);
            }
            for (i, n) in model.nodes.iter().enumerate() {
                if n.name == "wheel" {
                    assert_eq!(pose.globals[i], model.rest.globals[i]);
                    wheels += 1;
                }
            }
        }
        assert!(turned > 4 && wheels > 0, "{turned} {wheels}");
        let connector = g.models["train/train-connector"];
        let b = ctx.assets.model(connector).bounds().expect("сцепка");
        assert_eq!(couplers.len(), cars.len() - 1);
        for (c, d) in couplers.iter().zip(&s.models[n + cars.len()..]) {
            assert_eq!(d.model, connector);
            near(
                d.transform.transform_point3(vec3(0.0, 0.0, b.min.z)),
                flat(c[0], RAIL_TOP),
            );
            near(
                d.transform.transform_point3(vec3(0.0, 0.0, b.max.z)),
                flat(c[1], RAIL_TOP),
            );
            near(d.transform.transform_vector3(Vec3::Y), Vec3::Y);
            let across = (c[1] - c[0]).perp().normalize();
            near(
                d.transform.transform_vector3(Vec3::X).abs(),
                flat(across, 0.0).abs(),
            );
        }
    }

    #[test]
    fn roads_and_buildings_stand_by_map() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let s = scene(&mut g, &mut ctx, 0.0);
        let n = g.track.edges.len() + 2 * g.train.cars.len() - 1;
        let (roads, buildings) = s.models[n..].split_at(g.track.roads.len());
        assert_eq!(buildings.len(), g.track.buildings.len());
        let bounds = |d: &ModelDraw| {
            (ctx.assets.model(d.model).bounds().expect("габариты")).transformed(d.transform)
        };
        let grass = ctx
            .assets
            .model(g.models["road/grass"])
            .bounds()
            .expect("трава");
        let cells: Vec<_> = g.track.roads.iter().map(|r| r.cell).collect();
        for (r, d) in g.track.roads.iter().zip(roads) {
            assert_eq!(d.model, g.models[r.tile]);
            let b = bounds(d);
            let cell = r.cell.as_vec2() * CELL;
            near(b.min, flat(cell, -GRASS_TOP * TILE_SCALE));
            assert!(xz(b.max).abs_diff_eq(cell + CELL, 1e-4), "{r:?}: {b:?}");
            assert!(grass.transformed(d.transform).max.y.abs() < 1e-5, "{r:?}");
            if r.tile == "road/road-end" {
                let exit = xz(d.transform.transform_vector3(Vec3::X).normalize()).round();
                assert!(
                    cells.contains(&(r.cell + exit.as_ivec2())),
                    "{r:?}: выезд {exit}"
                );
            }
        }
        let rects: Vec<_> = (g.track.roads.iter())
            .filter(|r| r.tile.starts_with("road/road-"))
            .map(|r| (r.cell.as_vec2() * CELL, (r.cell.as_vec2() + 1.0) * CELL))
            .chain(buildings.iter().map(|d| {
                let b = bounds(d);
                (xz(b.min), xz(b.max))
            }))
            .collect();
        let overlap = |(a0, a1): (Vec2, Vec2), (b0, b1): (Vec2, Vec2)| {
            a0.cmplt(b1).all() && b0.cmplt(a1).all()
        };
        let first = rects.len() - buildings.len();
        for (i, (b, d)) in g.track.buildings.iter().zip(buildings).enumerate() {
            assert_eq!(d.model, g.models[b.model]);
            let bb = bounds(d);
            let height = ctx.assets.model(d.model).bounds().expect("здание").max.y;
            assert!(bb.min.y.abs() < 1e-5, "{b:?}");
            assert!(
                (bb.max.y - height * CITY_SCALE * b.scale).abs() < 1e-4,
                "{b:?}"
            );
            let twice = building_transform(&Building {
                scale: 2.0 * b.scale,
                ..*b
            });
            let top = twice.transform_point3(Vec3::Y * height).y;
            assert!(
                (top - 2.0 * height * CITY_SCALE * b.scale).abs() < 1e-4,
                "{b:?}"
            );
            assert!(
                xz(bb.min).midpoint(xz(bb.max)).abs_diff_eq(b.pos, 1e-3),
                "{b:?}"
            );
            let me = rects[first + i];
            for (j, &other) in rects.iter().enumerate() {
                assert!(
                    j == first + i || !overlap(me, other),
                    "{b:?} задевает {other:?}"
                );
            }
        }
    }

    #[test]
    fn ground_below_grass_sun_lights_sides_facing_camera() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let s = scene(&mut g, &mut ctx, 0.0);
        assert_eq!(
            (s.ambient, s.sun, s.fog),
            (AMBIENT, Some(SUN), Fog::default())
        );
        for side in [Vec3::X, Vec3::Y, Vec3::Z] {
            assert!(side.dot(s.camera.forward()) < 0.0, "{side} к камере");
            assert!(side.dot(-SUN.dir) > 0.0, "{side} в тени");
        }
        let ground = s.meshes[0];
        assert_eq!(s.meshes.len(), 1);
        assert_eq!(
            (ground.mesh, ground.texture, ground.color),
            (g.ground, Handle::WHITE, GROUND)
        );
        let corners: Vec<_> = (ctx.assets.mesh_geometry(g.ground).positions.iter())
            .map(|&p| ground.transform.transform_point3(p))
            .collect();
        assert!(corners.iter().all(|c| c.y == -GROUND_DROP), "{corners:?}");
        let points = g.track.ports.iter().map(|p| p.pos);
        let (lo, hi) = points.fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p), hi.max(p))
        });
        let (c0, c1) = corners
            .iter()
            .fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(xz(*p)), hi.max(xz(*p)))
            });
        assert_eq!((c0, c1), (lo - GROUND_MARGIN, hi + GROUND_MARGIN));
        const { assert!(GROUND_DROP > 0.0) };
    }

    #[test]
    fn quad_faces_up() {
        let (vertices, indices) = quad();
        for t in indices.chunks(3) {
            let p = |i: u32| Vec3::from(vertices[i as usize].pos);
            let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            assert!(n.normalize().abs_diff_eq(Vec3::Y, 1e-6), "{t:?}");
        }
        assert!(vertices.iter().all(|v| Vec3::from(v.normal) == Vec3::Y));
    }

    #[test]
    fn hud_shows_speed_top_left_both_ways() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        for speed in [2.5, -2.5] {
            g.train.speed = speed;
            assert_eq!(
                hud(&mut g, &mut ctx),
                layout(&g, &ctx, "  9 KM/H", vec2(-628.0, 324.0))
            );
        }
    }

    pub(crate) fn tick(g: &mut Trains, ctx: &mut Context) {
        g.update(ctx);
        ctx.input.end_tick();
    }

    /// Кадр показан ортокамерой на `target`, клик в середине экрана.
    fn click(ctx: &mut Context, target: Vec2) {
        let size = UVec2::new(1280, 720);
        ctx.shown = Some(Shown {
            camera: Camera2D {
                center: Vec2::ZERO,
                size: VIEW,
            },
            viewport: Viewport {
                x: 0.0,
                y: 0.0,
                w: VIEW.x,
                h: VIEW.y,
            },
            scene: Some(ShownScene {
                camera: camera(vec3(target.x, 0.0, target.y)),
                size,
            }),
        });
        ctx.input.on_cursor(Some(size.as_vec2() / 2.0));
        ctx.input
            .on_button(MouseButton::Left, ElementState::Released);
        ctx.input
            .on_button(MouseButton::Left, ElementState::Pressed);
    }

    fn layout(g: &Trains, ctx: &Context, text: &str, pos: Vec2) -> Vec<Sprite> {
        let font = ctx.assets.font(g.font);
        assert!(font.atlas.is_some(), "шрифт не загрузился");
        text::layout(font, text, pos, 1.0, Color::WHITE, HUD_LAYER).collect()
    }

    /// Стоящий состав: скорость 0.
    fn standing(g: &Trains, ctx: &Context) -> Vec<Sprite> {
        layout(g, ctx, "  0 KM/H", vec2(-628.0, 324.0))
    }

    fn hud(g: &mut Trains, ctx: &mut Context) -> Vec<Sprite> {
        ctx.gfx.sprites.clear();
        g.draw(ctx, 0.0);
        (ctx.gfx.sprites.iter())
            .filter(|s| s.layer == HUD_LAYER)
            .copied()
            .collect()
    }

    #[test]
    fn catalog_is_every_locomotive_and_wagon() {
        let mut ctx = Context::default();
        let g = Trains::init(&mut ctx);
        let cars = catalog(&g.models, &ctx.assets);
        let mut want: Vec<_> = (MODELS.iter().copied())
            .filter(|n| n.starts_with("train/train-") && *n != "train/train-connector")
            .collect();
        want.sort_unstable();
        assert_eq!(cars.keys().copied().collect::<Vec<_>>(), want);
    }

    #[test]
    fn starts_in_depot_with_start_consist() {
        let mut ctx = Context::default();
        let g = Trains::init(&mut ctx);
        assert_eq!(g.train.route[0], g.track.depot[0]);
        assert_eq!(g.train.tail, MARGIN);
        let models: Vec<_> = g.train.cars.iter().map(|c| c.model).collect();
        assert_eq!(models, START.map(|n| g.models[n]));
    }

    #[test]
    fn e_throws_switch_ahead_click_throws_switch_under_cursor() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let (a, d) = (g.track.switch_by_pos(A), g.track.switch_by_pos(D));
        ctx.input
            .on_key(KeyCode::KeyE, ElementState::Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(
            (g.track.switches[d].active, g.track.switches[a].active),
            (1, 0)
        );
        tick(&mut g, &mut ctx);
        assert_eq!(
            g.track.switches[d].active, 1,
            "держать E — не переводить снова"
        );
        click(&mut ctx, A);
        tick(&mut g, &mut ctx);
        assert_eq!(
            (g.track.switches[d].active, g.track.switches[a].active),
            (1, 1)
        );
        click(&mut ctx, D);
        ctx.input
            .on_key(KeyCode::KeyE, ElementState::Released, false);
        ctx.input
            .on_key(KeyCode::KeyE, ElementState::Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(
            g.track.switches[d].active, 0,
            "клик и E по одной стрелке — один перевод"
        );
        assert_eq!(g.busy, 0);
        assert_eq!(hud(&mut g, &mut ctx), standing(&g, &ctx));
    }

    #[test]
    fn switch_under_train_refuses_and_hud_shows_busy_for_a_second() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let bottom = edge(&g.track, Piece::Straight, [2, 0]);
        g.train = Train::new(&g.track, (bottom, true), 2.0, g.train.cars.clone()).expect("состав");
        let a = g.track.switch_by_pos(A);
        click(&mut ctx, A);
        tick(&mut g, &mut ctx);
        assert_eq!(g.track.switches[a].active, 0);
        let x = -ctx.assets.font(g.font).width(BUSY) / 2.0;
        let mut want = standing(&g, &ctx);
        want.extend(layout(&g, &ctx, BUSY, vec2(x, 324.0)));
        assert_eq!(hud(&mut g, &mut ctx), want);
        assert_eq!(
            ctx.gfx.camera,
            Camera2D {
                center: Vec2::ZERO,
                size: VIEW
            }
        );
        for _ in 1..TICK_HZ {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(hud(&mut g, &mut ctx), want);
        tick(&mut g, &mut ctx);
        assert_eq!(hud(&mut g, &mut ctx), standing(&g, &ctx));
    }

    #[test]
    fn w_s_and_arrows_drive_and_brake() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        g.depot.open = false;
        let start = g.train.clone();
        for key in GAS {
            ctx.input.on_key(*key, ElementState::Pressed, false);
            tick(&mut g, &mut ctx);
            let mut want = start.clone();
            want.drive(1.0, DT);
            assert_eq!(g.train.speed, want.speed, "{key:?}");
            assert!(g.train.head > start.head, "{key:?}");
            ctx.input.on_key(*key, ElementState::Released, false);
            g.train = start.clone();
        }
        for key in BACK {
            g.train.speed = 1.0;
            ctx.input.on_key(*key, ElementState::Pressed, false);
            tick(&mut g, &mut ctx);
            assert!(g.train.speed < 0.99, "тормоз, а не накат: {key:?}");
            ctx.input.on_key(*key, ElementState::Released, false);
        }
    }

    fn press(ctx: &mut Context, key: KeyCode) {
        ctx.input.on_key(key, ElementState::Pressed, false);
        ctx.input.on_key(key, ElementState::Released, false);
    }

    #[test]
    fn depot_is_open_at_start_and_blocks_gas() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        assert!(g.depot.open);
        let start = g.train.clone();
        ctx.input
            .on_key(KeyCode::KeyW, ElementState::Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(g.train, start);
    }

    #[test]
    fn go_closes_depot_and_train_drives_off_from_where_it_stands() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        g.train.speed = 1.0;
        g.train.advance(1.0, &g.track);
        g.train.speed = 0.0;
        let stood = g.train.clone();
        g.depot.queue.push(DepotCmd::Go);
        tick(&mut g, &mut ctx);
        assert!(!g.depot.open);
        assert_eq!(g.train, stood, "без пересборки состав на месте");
        ctx.input
            .on_key(KeyCode::KeyW, ElementState::Pressed, false);
        tick(&mut g, &mut ctx);
        assert!(g.train.speed > 0.0);
    }

    #[test]
    fn tab_opens_depot_only_for_standing_train_on_depot_path() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        press(&mut ctx, KeyCode::Tab);
        tick(&mut g, &mut ctx);
        assert!(!g.depot.open, "Tab закрывает");
        g.train.speed = 1.0;
        press(&mut ctx, KeyCode::Tab);
        tick(&mut g, &mut ctx);
        assert!(!g.depot.open, "состав едет");
        g.train.speed = 0.0;
        tick(&mut g, &mut ctx);
        assert!(!g.depot.open, "без Tab");
        press(&mut ctx, KeyCode::Tab);
        tick(&mut g, &mut ctx);
        assert!(g.depot.open);
        press(&mut ctx, KeyCode::Tab);
        tick(&mut g, &mut ctx);
        let last = g.track.depot[g.track.depot.len() - 1];
        let out = g.track.next(last).expect("за стрелкой D");
        assert!(!g.track.depot.iter().any(|&(e, _)| e == out.0));
        let cars = g.train.cars.clone();
        g.train = Train::new(&g.track, last, 0.0, cars.clone()).expect("за D");
        assert!(g.train.route.contains(&out), "{:?}", g.train.route);
        press(&mut ctx, KeyCode::Tab);
        tick(&mut g, &mut ctx);
        assert!(!g.depot.open, "голова за D");
        g.train = Train::new(&g.track, out, 0.0, cars).expect("за D");
        press(&mut ctx, KeyCode::Tab);
        tick(&mut g, &mut ctx);
        assert!(!g.depot.open, "состав на петле");
    }

    #[test]
    fn rebuild_puts_consist_at_depot_start() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let start = g.train.clone();
        g.depot.open = false;
        ctx.input
            .on_key(KeyCode::KeyW, ElementState::Pressed, false);
        for _ in 0..60 {
            tick(&mut g, &mut ctx);
        }
        ctx.input
            .on_key(KeyCode::KeyW, ElementState::Released, false);
        g.train.speed = 0.0;
        g.train.reverse = true;
        assert_ne!(
            (g.train.route[0], g.train.tail),
            (start.route[0], start.tail)
        );
        press(&mut ctx, KeyCode::Tab);
        tick(&mut g, &mut ctx);
        assert!(g.depot.open);
        let tank = "train/train-carriage-tank";
        g.depot.queue.push(DepotCmd::Add(tank));
        tick(&mut g, &mut ctx);
        let mut names = START.to_vec();
        names.push(tank);
        let cars: Vec<_> = names.iter().map(|n| g.cars[n]).collect();
        let want = Train::new(&g.track, g.track.depot[0], MARGIN, cars).expect("депо");
        assert_eq!(g.train, want);
        assert_eq!(
            g.prev, want,
            "кадр после пересборки не тянется от старого места"
        );
        assert_eq!((want.route[0], want.tail), (start.route[0], start.tail));
        assert!(g.depot.open, "пересборка депо не закрывает");
    }

    fn harness(g: Trains) -> Harness<'static, Trains> {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 720.0))
            .build_ui_state(|ui, g: &mut Trains| g.ui(ui.ctx()), g);
        h.run();
        h
    }

    #[test]
    fn depot_window_only_while_open_with_cyrillic_font_and_whole_catalog() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        g.depot.open = false;
        let h = harness(g);
        assert!(h.query_by_label("Депо").is_none());
        let mut h = harness(Trains::init(&mut ctx));
        assert!(
            h.ctx
                .fonts(|f| f.definitions().font_data.contains_key(FONT_PATH))
        );
        let names: Vec<_> = h.state().cars.keys().copied().collect();
        for name in names {
            let label = name.strip_prefix("train/train-").expect(name);
            let label = if depot::is_wagon(name) {
                format!("+ {label}")
            } else {
                label.to_owned()
            };
            assert!(h.query_by_label(&label).is_some(), "{label}");
        }
        h.get_by_label("Ехать").click();
        h.run();
        assert_eq!(h.state().depot.queue, [DepotCmd::Go]);
    }
}
