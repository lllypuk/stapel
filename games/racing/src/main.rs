#[cfg(test)]
mod autopilot;
mod camera;
mod car;
mod car_config;
mod layout;
mod pieces;
mod race;
mod track;

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use engine::{
    Assets, Camera2D, Color, Config, Context, DirectionalLight, Fog, Font, FontData, Game, Gfx,
    Handle, IVec2, Input, KeyCode, ModelDraw, Pose, Scene, StaticWorld, Vec2, Vec3, Watcher, egui,
    vec2, vec3,
};

use crate::camera::Chase;
use crate::car::{Car, Controls, SUBSTEP, SUBSTEPS};
use crate::car_config::CarConfig;
use crate::layout::{Layout, Models};
use crate::pieces::KIT_SCALE;
use crate::race::{Race, lap_time};
use crate::track::{Checkpoint, Track, cell_at};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
const TRACKS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tracks/");
/// Трасса без `TRACK=<имя>` в окружении.
const DEFAULT_TRACK: &str = "track01";
const CARS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/cars/");
const FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Regular.ttf"
);

const TICK_HZ: u32 = 60;
/// HUD в единицах окна 1280×720; иной аспект — letterbox.
const VIEW: Vec2 = vec2(1280.0, 720.0);
/// При `VIEW` в окне 1280×720 тексель атласа — пиксель экрана.
const HUD_PX: f32 = 24.0;
const HUD_MARGIN: f32 = 12.0;
const HUD_LAYER: i32 = 1;
/// Атлас шрифта — только ASCII, поэтому HUD по-английски.
const WRONG_WAY: &str = "WRONG WAY";

const GAS: &[KeyCode] = &[KeyCode::KeyW, KeyCode::ArrowUp];
const BRAKE: &[KeyCode] = &[KeyCode::KeyS, KeyCode::ArrowDown];
const LEFT: &[KeyCode] = &[KeyCode::KeyA, KeyCode::ArrowLeft];
const RIGHT: &[KeyCode] = &[KeyCode::KeyD, KeyCode::ArrowRight];

/// Ниже этой высоты центра масс — респаун, м.
const FALL_Y: f32 = -10.0;
/// Тиков колёсами вверх до респауна.
const UPSIDE_DOWN_TICKS: u32 = 2 * TICK_HZ;
/// Тиков вне клеток дороги своего уровня до респауна.
const OFF_ROAD_TICKS: u32 = 3 * TICK_HZ;
/// Машина ставится чуть выше дороги и падает на колёса, м.
const SPAWN_LIFT: f32 = 0.1;
/// Луч к асфальту респауна — от стольких метров над уровнем: ниже настила моста (1,5 м), выше середины рампы.
const GROUND_PROBE: f32 = 1.0;

/// Трасса `name` из `assets/tracks/`, без имени — `DEFAULT_TRACK`.
fn track_path(name: Option<&str>) -> PathBuf {
    PathBuf::from(format!(
        "{TRACKS_DIR}{}.toml",
        name.unwrap_or(DEFAULT_TRACK)
    ))
}

/// Характеристики машин из `dir` по имени файла без `.toml`: `race` раньше `race-future`.
fn load_cars(dir: &str) -> Vec<CarConfig> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .map(|e| e.unwrap_or_else(|e| panic!("{dir}: {e}")).path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort_by(|a, b| a.file_stem().cmp(&b.file_stem()));
    paths
        .iter()
        .map(|p| {
            let text = fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            CarConfig::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        })
        .collect()
}

/// Трасса с цепочкой и машиной из `cars`.
fn load_track(text: &str, cars: &[CarConfig]) -> Result<(Track, Vec<Checkpoint>), String> {
    let track = Track::parse(text).map_err(|e| e.to_string())?;
    let checkpoints = track.chain().map_err(|e| e.to_string())?;
    if !cars.iter().any(|c| c.model == track.car) {
        return Err(format!("машины {} нет в cars/", track.car));
    }
    Ok((track, checkpoints))
}

fn text_width(font: &FontData, text: &str) -> f32 {
    text.chars()
        .filter_map(|c| font.glyph(c).or_else(|| font.glyph('?')))
        .map(|g| g.advance)
        .sum()
}

fn controls(input: &Input) -> Controls {
    let down = |keys: &[KeyCode]| f32::from(u8::from(keys.iter().any(|&k| input.is_down(k))));
    Controls {
        throttle: down(GAS),
        brake: down(BRAKE),
        steer: down(LEFT) - down(RIGHT),
        handbrake: input.is_down(KeyCode::Space),
    }
}

/// Куда ставится машина: начало модели и направление носа.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spawn {
    pos: Vec3,
    dir: Vec3,
}

impl Spawn {
    /// Посреди клетки за чекпоинтом, на асфальте: на линии выхода `roadRamp` зад машины — под кромкой трамплина.
    fn at(c: &Checkpoint, world: &StaticWorld) -> Self {
        let pos = c.origin + c.normal * KIT_SCALE / 2.0;
        let ground = world.raycast(pos + Vec3::Y * GROUND_PROBE, Vec3::NEG_Y, KIT_SCALE, |_| {
            true
        });
        Self {
            pos: ground.map_or(pos, |h| h.point),
            dir: c.normal,
        }
    }

    /// Посреди последней клетки `roadStart`: первый чекпоинт — на его выходе.
    fn start(checkpoints: &[Checkpoint]) -> Self {
        let first = &checkpoints[0];
        Self {
            pos: first.origin - first.normal * KIT_SCALE / 2.0,
            dir: first.normal,
        }
    }
}

struct Racing {
    layout: Layout,
    models: Models,
    sky: Color,
    cars: Vec<CarConfig>,
    car_index: usize,
    car: Car,
    pose: Pose,
    chase: Chase,
    race: Race,
    road_cells: HashSet<(IVec2, u32)>,
    /// Посреди последней клетки `roadStart` носом по цепочке.
    start: Spawn,
    /// Последний пройденный чекпоинт, до первого — старт.
    respawn: Spawn,
    upside_down: u32,
    off_road: u32,
    font: Handle<Font>,
    /// Ошибка разбора трассы; кириллицы нет в атласе, поэтому — в egui.
    error: Option<String>,
    path: PathBuf,
    watch: Watcher,
}

impl Racing {
    /// Трасса `text`, прочитанная из `path`; дальше за файлом следит `Watcher`.
    ///
    /// # Panics
    /// Если `load_track` не принял трассу.
    fn new(path: PathBuf, text: String, cars: Vec<CarConfig>, assets: &mut Assets) -> Self {
        let (track, checkpoints) =
            load_track(&text, &cars).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let models = Models::load(assets);
        let car_index = (cars.iter().position(|c| c.model == track.car)).expect("load_track");
        let start = Spawn::start(&checkpoints);
        let car = Self::make_car(&cars[car_index], &models, assets);
        let mut racing = Self {
            layout: Layout::new(&track, &models, assets),
            models,
            sky: Color::from_srgb8(170, 205, 235),
            chase: Chase::new(car.body.pos, car.body.rot),
            cars,
            car_index,
            car,
            pose: Pose::default(),
            road_cells: track.road_cells(),
            race: Race::new(&track, checkpoints),
            start,
            respawn: start,
            upside_down: 0,
            off_road: 0,
            font: assets.load_font(FONT_PATH, HUD_PX),
            error: None,
            path,
            watch: Watcher::new(text),
        };
        racing.restart();
        racing
    }

    /// Изменённая на диске трасса — заново с прежними моделями и машиной; битая — прежняя и ошибка.
    fn reload_changed(&mut self, assets: &Assets) {
        let text = match self.watch.tick(&self.path) {
            Ok(Some(text)) => text,
            Ok(None) => return,
            Err(e) => {
                log::warn!("{e}");
                return;
            }
        };
        let (track, checkpoints) = match load_track(&text, &self.cars) {
            Ok(t) => t,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.error = None;
        self.layout = Layout::new(&track, &self.models, assets);
        self.road_cells = track.road_cells();
        self.start = Spawn::start(&checkpoints);
        self.race = Race::new(&track, checkpoints);
        self.restart();
    }

    fn make_car(config: &CarConfig, models: &Models, assets: &Assets) -> Car {
        let model = assets.model(models.get(&config.model));
        Car::new(config.clone(), model, Vec3::ZERO, 0.0)
            .unwrap_or_else(|e| panic!("{}: {e}", config.model))
    }

    fn spawn(&mut self, at: Spawn) {
        self.car
            .place(at.pos + Vec3::Y * SPAWN_LIFT, at.dir.x.atan2(at.dir.z));
        self.chase = Chase::new(self.car.body.pos, self.car.body.rot);
        self.upside_down = 0;
        self.off_road = 0;
    }

    fn restart(&mut self) {
        self.race.restart();
        self.respawn = self.start;
        self.spawn(self.start);
    }

    fn next_car(&mut self, assets: &Assets) {
        self.car_index = (self.car_index + 1) % self.cars.len();
        self.car = Self::make_car(&self.cars[self.car_index], &self.models, assets);
        self.restart();
    }

    fn pass_checkpoint(&mut self) {
        let body = &self.car.body;
        if let Some(i) = self.race.tick(self.car.prev.0, body.pos, body.vel) {
            self.respawn = Spawn::at(&self.race.checkpoints[i], &self.layout.world);
        }
    }

    fn status(&self) -> String {
        let r = &self.race;
        let best = r.best.map_or_else(|| "-:--.--".into(), lap_time);
        format!("LAP {} {}\nBEST  {best}", r.laps + 1, lap_time(r.ticks))
    }

    /// Круг и лучший — слева сверху, скорость — справа снизу, «не туда» — по центру.
    fn draw_hud(&self, gfx: &mut Gfx, assets: &Assets) {
        gfx.camera = Camera2D {
            center: Vec2::ZERO,
            size: VIEW,
        };
        let half = VIEW / 2.0;
        let font = assets.font(self.font);
        let speed = format!("{:3.0} KM/H", self.car.body.vel.length() * 3.6);
        let mut lines = vec![
            (
                self.status(),
                vec2(-half.x + HUD_MARGIN, half.y - HUD_MARGIN - HUD_PX),
            ),
            (
                speed.clone(),
                vec2(
                    half.x - HUD_MARGIN - text_width(font, &speed),
                    HUD_MARGIN - half.y,
                ),
            ),
        ];
        if self.race.wrong_way() {
            let pos = vec2(-text_width(font, WRONG_WAY) / 2.0, half.y / 2.0);
            lines.push((WRONG_WAY.into(), pos));
        }
        for (s, pos) in lines {
            gfx.draw_text(assets, self.font, &s, pos, 1.0, Color::WHITE, HUD_LAYER);
        }
    }

    /// Тик машины с управлением `controls`: чекпоинты, респаун, камера.
    fn drive(&mut self, controls: Controls) {
        self.car.tick(&self.layout.world, controls);
        self.pass_checkpoint();
        self.auto_respawn();
        let tick = SUBSTEPS as f32 * SUBSTEP;
        self.chase.tick(self.car.body.pos, self.car.body.rot, tick);
    }

    /// Респаун после падения, переворота или съезда с дороги своего уровня.
    fn auto_respawn(&mut self) {
        let body = &self.car.body;
        let on_road = cell_at(body.pos).is_some_and(|c| self.road_cells.contains(&c));
        self.off_road = if on_road { 0 } else { self.off_road + 1 };
        let upside_down = (body.rot * Vec3::Y).y < 0.0;
        self.upside_down = if upside_down { self.upside_down + 1 } else { 0 };
        if body.pos.y < FALL_Y
            || self.upside_down >= UPSIDE_DOWN_TICKS
            || self.off_road >= OFF_ROAD_TICKS
        {
            self.spawn(self.respawn);
        }
    }
}

impl Game for Racing {
    fn init(ctx: &mut Context) -> Self {
        let path = track_path(std::env::var("TRACK").ok().as_deref());
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Self::new(path, text, load_cars(CARS_DIR), &mut ctx.assets)
    }

    fn update(&mut self, ctx: &mut Context) {
        self.reload_changed(&ctx.assets);
        if ctx.input.just_pressed(KeyCode::Tab) && !self.race.started {
            self.next_car(&ctx.assets);
        }
        if ctx.input.just_pressed(KeyCode::KeyR) {
            self.spawn(self.respawn);
        }
        self.drive(controls(&ctx.input));
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        let model = self.models.get(&self.car.config.model);
        self.car.pose(ctx.assets.model(model), &mut self.pose);
        let (pos, rot) = self.car.lerp(alpha);
        let mut models = self.layout.draws.clone();
        models.push(ModelDraw {
            model,
            transform: self.car.transform(alpha),
            pose: Some(self.pose.clone()),
            tint: Color::WHITE,
        });
        ctx.gfx.clear_color = self.sky;
        ctx.gfx.scene = Some(Scene {
            camera: self.chase.camera(alpha, pos, rot),
            ambient: Color {
                r: 0.35,
                g: 0.37,
                b: 0.42,
                a: 1.0,
            },
            fog: Fog {
                color: self.sky,
                start: 80.0,
                end: 320.0,
            },
            sun: Some(DirectionalLight {
                dir: vec3(0.4, -1.0, 0.3),
                color: Color {
                    r: 1.0,
                    g: 0.95,
                    b: 0.85,
                    a: 1.0,
                },
            }),
            lights: Vec::new(),
            meshes: Vec::new(),
            models,
        });
        self.draw_hud(&mut ctx.gfx, &ctx.assets);
    }

    fn ui(&mut self, ui: &egui::Context) {
        engine::ui::set_font(ui, FONT_PATH);
        let Some(error) = &self.error else {
            return;
        };
        egui::Window::new("Ошибка трассы")
            .collapsible(false)
            .resizable(false)
            .anchor(
                egui::Align2::LEFT_BOTTOM,
                egui::vec2(HUD_MARGIN, -HUD_MARGIN),
            )
            .show(ui, |ui| ui.label(error));
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Racing>(Config {
        title: "racing".into(),
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
    use engine::ElementState::{Pressed, Released};
    use engine::{Quat, Sprite, Triangle, text};

    use super::*;
    use crate::layout::{self, ROAD};
    use crate::pieces::LEVEL;

    const OVAL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/testdata/oval.toml");

    fn open(path: PathBuf, ctx: &mut Context) -> Racing {
        let text = fs::read_to_string(&path).expect("трасса");
        Racing::new(path, text, load_cars(CARS_DIR), &mut ctx.assets)
    }

    fn racing(ctx: &mut Context) -> Racing {
        open(OVAL.into(), ctx)
    }

    fn tick(g: &mut Racing, ctx: &mut Context) {
        g.update(ctx);
        ctx.input.end_tick();
    }

    fn ticks(g: &mut Racing, ctx: &mut Context, n: u32) {
        for _ in 0..n {
            tick(g, ctx);
        }
    }

    fn tap(g: &mut Racing, ctx: &mut Context, key: KeyCode) {
        ctx.input.on_key(key, Pressed, false);
        tick(g, ctx);
        ctx.input.on_key(key, Released, false);
    }

    /// Последний тик начат с машины, поставленной в `at`.
    fn placed_at(g: &Racing, at: Spawn) -> bool {
        let origin = g.car.transform(0.0).transform_point3(Vec3::ZERO);
        let lift = Vec3::Y * (SPAWN_LIFT + g.car.config.travel);
        let nose = g.car.prev.1 * Vec3::Z;
        origin.abs_diff_eq(at.pos + lift, 1e-4) && nose.abs_diff_eq(at.dir, 1e-5)
    }

    /// Расстояние по горизонтали.
    fn across(a: Vec3, b: Vec3) -> f32 {
        (a - b).with_y(0.0).length()
    }

    /// Плоскость дороги на высоте `y` вместо мира трассы.
    fn flat(g: &mut Racing, y: f32) {
        let half = 1000.0;
        let corner = [(-1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0)]
            .map(|(x, z)| vec3(x * half, y, z * half));
        let tri = |[a, b, c]: [usize; 3]| Triangle {
            verts: [corner[a], corner[b], corner[c]],
            tag: ROAD,
        };
        g.layout.world = StaticWorld::new(KIT_SCALE, vec![tri([0, 1, 2]), tri([0, 2, 3])]);
    }

    #[test]
    fn keys_map_to_controls() {
        let mut input = Input::default();
        for k in [KeyCode::KeyW, KeyCode::KeyA, KeyCode::Space] {
            input.on_key(k, Pressed, false);
        }
        let want = Controls {
            throttle: 1.0,
            brake: 0.0,
            steer: 1.0,
            handbrake: true,
        };
        assert_eq!(controls(&input), want);
        let mut input = Input::default();
        for k in [KeyCode::ArrowDown, KeyCode::ArrowRight] {
            input.on_key(k, Pressed, false);
        }
        let want = Controls {
            brake: 1.0,
            steer: -1.0,
            ..Controls::default()
        };
        assert_eq!(controls(&input), want);
        for k in [
            KeyCode::ArrowUp,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::ArrowLeft,
        ] {
            input.on_key(k, Pressed, false);
        }
        let both = controls(&input);
        assert_eq!((both.throttle, both.brake, both.steer), (1.0, 1.0, 0.0));
    }

    #[test]
    fn load_cars_reads_every_file_sorted() {
        let names: Vec<String> = load_cars(CARS_DIR).into_iter().map(|c| c.model).collect();
        assert_eq!(
            names,
            ["hatchback-sports", "race", "race-future", "sedan-sports"]
        );
    }

    #[test]
    fn starts_mid_last_start_cell_facing_chain_and_stays_on_road() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        // Старт овала — клетки [2..3, 0], цепочка едет к −X: выход у x = 8.
        let start = Spawn {
            pos: vec3(10.0, 0.0, 2.0),
            dir: Vec3::NEG_X,
        };
        assert!(g.start.pos.abs_diff_eq(start.pos, 1e-5), "{:?}", g.start);
        assert!(g.start.dir.abs_diff_eq(start.dir, 1e-6));
        assert_eq!(g.respawn, g.start);
        assert_eq!(g.car.config.model, "race");
        ticks(&mut g, &mut ctx, 2 * 60);
        assert!(g.car.body.vel.length() < 0.05, "{}", g.car.body.vel);
        let origin = g.car.transform(1.0).transform_point3(Vec3::ZERO);
        assert!(across(origin, start.pos) < 0.05, "{origin}");
        assert!(origin.y.abs() < 0.2, "{origin}");
        assert_eq!((g.off_road, g.upside_down, g.race.next), (0, 0, 0));
    }

    /// Газ до пересечения первого чекпоинта; дольше 3 с — паника.
    fn cross_start_line(g: &mut Racing, ctx: &mut Context) {
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        for _ in 0..3 * 60 {
            tick(g, ctx);
            if g.race.next == 1 {
                ctx.input.on_key(KeyCode::KeyW, Released, false);
                return;
            }
        }
        panic!("не пересекла старт: {}", g.car.body.pos);
    }

    #[test]
    fn r_returns_to_last_checkpoint_in_its_direction() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ticks(&mut g, &mut ctx, 10);
        ctx.input.on_key(KeyCode::KeyW, Released, false);
        assert!(g.car.body.vel.length() > 1.0);
        tap(&mut g, &mut ctx, KeyCode::KeyR);
        assert!(placed_at(&g, g.start));

        cross_start_line(&mut g, &mut ctx);
        let first = Spawn::at(&g.race.checkpoints[0], &g.layout.world);
        assert_eq!(g.respawn, first);
        ticks(&mut g, &mut ctx, 30);
        tap(&mut g, &mut ctx, KeyCode::KeyR);
        assert!(placed_at(&g, first));
        assert_eq!(g.race.next, 1);
        let eye = g
            .chase
            .eye
            .distance(camera::target(g.car.body.pos, g.car.body.rot));
        assert!(eye < 0.5, "{eye}");
    }

    #[test]
    fn tab_switches_car_only_before_start_line() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ticks(&mut g, &mut ctx, 10);
        ctx.input.on_key(KeyCode::KeyW, Released, false);
        tap(&mut g, &mut ctx, KeyCode::Tab);
        assert_eq!(g.car.config.model, "race-future");
        assert!(placed_at(&g, g.start));
        assert_eq!(g.car.body.vel.with_y(0.0).length(), 0.0);
        for _ in 0..3 {
            tap(&mut g, &mut ctx, KeyCode::Tab);
        }
        assert_eq!(g.car.config.model, "race");

        cross_start_line(&mut g, &mut ctx);
        assert!(g.race.started);
        tap(&mut g, &mut ctx, KeyCode::Tab);
        assert_eq!(g.car.config.model, "race");
        assert!(!placed_at(&g, g.start), "рестартовала");
        assert_eq!(g.race.next, 1);
    }

    #[test]
    fn respawn_at_every_checkpoint_of_every_track_stands_on_road() {
        let mut ctx = Context::default();
        for entry in fs::read_dir(TRACKS_DIR).expect("трассы") {
            let path = entry.expect("трасса").path();
            let mut g = open(path.clone(), &mut ctx);
            for i in 0..g.race.checkpoints.len() {
                let at = Spawn::at(&g.race.checkpoints[i], &g.layout.world);
                g.spawn(at);
                ctx.input.on_key(KeyCode::KeyS, Pressed, false);
                for _ in 0..120 {
                    tick(&mut g, &mut ctx);
                    let body = &g.car.body;
                    let upright = (body.rot * Vec3::Y).y > 0.5;
                    assert!(
                        upright && body.pos.y > at.pos.y - 0.5,
                        "{} чекпоинт {i}: {}",
                        path.display(),
                        body.pos
                    );
                }
                ctx.input.on_key(KeyCode::KeyS, Released, false);
            }
        }
    }

    #[test]
    fn falling_below_ten_meters_respawns() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        ticks(&mut g, &mut ctx, 30);
        g.car.place(vec3(14.0, -9.0, 10.0), 0.0);
        tick(&mut g, &mut ctx);
        assert!(g.car.body.pos.y < -8.0, "{}", g.car.body.pos);
        g.car.place(vec3(14.0, -10.6, 10.0), 0.0);
        tick(&mut g, &mut ctx);
        assert!(placed_at(&g, g.start), "{}", g.car.body.pos);
    }

    #[test]
    fn upside_down_for_two_seconds_respawns() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        flat(&mut g, 0.0);
        g.car.body.rot = Quat::from_rotation_z(std::f32::consts::PI) * g.car.body.rot;
        g.car.body.pos.y += 0.5;
        for t in 1..2 * 60 {
            tick(&mut g, &mut ctx);
            assert!((g.car.body.rot * Vec3::Y).y < 0.0, "{t}");
            assert_eq!(g.upside_down, t);
        }
        tick(&mut g, &mut ctx);
        assert!(placed_at(&g, g.start));
        assert_eq!(g.upside_down, 0);
    }

    #[test]
    fn off_road_of_own_level_for_three_seconds_respawns() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        flat(&mut g, 0.0);
        // [3, 2] — внутри овала.
        let inside = vec3(14.0, 0.0, 10.0);
        g.car.place(inside, 0.0);
        for t in 1..3 * 60 {
            tick(&mut g, &mut ctx);
            assert_eq!(g.off_road, t);
        }
        assert!(across(g.car.body.pos, inside) < 0.1);
        tick(&mut g, &mut ctx);
        assert_eq!(g.off_road, 0);
        let origin = g.car.transform(1.0).transform_point3(Vec3::ZERO);
        assert!(across(origin, g.start.pos) < 0.01, "{origin}");

        // Над клеткой старта, но уровнем выше.
        flat(&mut g, LEVEL_UP);
        g.car.place(g.start.pos + Vec3::Y * LEVEL_UP, 0.0);
        ticks(&mut g, &mut ctx, 2);
        assert_eq!(g.off_road, 2);
        flat(&mut g, 0.0);
        g.car.place(g.start.pos, 0.0);
        tick(&mut g, &mut ctx);
        assert_eq!(g.off_road, 0);
    }

    const LEVEL_UP: f32 = 2.2;

    #[test]
    fn draw_shows_track_and_posed_car_from_chase_camera() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        ticks(&mut g, &mut ctx, 5);
        g.draw(&mut ctx, 0.5);
        let scene = ctx.gfx.scene.expect("сцена");
        let (car, track) = scene.models.split_last().expect("машина");
        assert_eq!(track, g.layout.draws.as_slice());
        assert_eq!(car.model, g.models.get("race"));
        assert_eq!(car.transform, g.car.transform(0.5));
        assert_eq!(car.pose.as_ref(), Some(&g.pose));
        assert_ne!(g.pose, Pose::default());
        let (pos, rot) = g.car.lerp(0.5);
        assert_eq!(scene.camera, g.chase.camera(0.5, pos, rot));
        assert!(scene.sun.is_some_and(|s| s.dir.y < 0.0));
        assert!(scene.fog.end <= scene.camera.far);
        assert_eq!(ctx.gfx.clear_color, scene.fog.color);
    }

    fn hud(ctx: &Context) -> Vec<Sprite> {
        (ctx.gfx.sprites.iter())
            .filter(|s| s.layer == HUD_LAYER)
            .copied()
            .collect()
    }

    #[test]
    fn hud_shows_lap_best_speed_and_wrong_way() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        (g.race.laps, g.race.ticks, g.race.best) = (1, 150, Some(61 * 60 + 30));
        g.car.body.vel = vec3(0.0, 0.0, -25.0);
        g.draw(&mut ctx, 1.0);
        assert_eq!(
            ctx.gfx.camera,
            Camera2D {
                center: Vec2::ZERO,
                size: VIEW
            }
        );
        let driving = hud(&ctx);
        ctx.gfx.sprites.clear();
        g.race.best = None;
        g.race.wrong = 2 * TICK_HZ;
        g.draw(&mut ctx, 1.0);
        let wrong = hud(&ctx);

        let font = ctx.assets.font(g.font);
        assert!(font.atlas.is_some(), "шрифт не загрузился");
        let text = |s, pos| text::layout(font, s, pos, 1.0, Color::WHITE, HUD_LAYER);
        let speed = " 90 KM/H";
        let right = vec2(628.0 - text_width(font, speed), -348.0);
        let want: Vec<_> = text("LAP 2 0:02.50\nBEST  1:01.50", vec2(-628.0, 324.0))
            .chain(text(speed, right))
            .collect();
        assert_eq!(driving, want);
        let want: Vec<_> = text("LAP 2 0:02.50\nBEST  -:--.--", vec2(-628.0, 324.0))
            .chain(text(speed, right))
            .chain(text(
                WRONG_WAY,
                vec2(-text_width(font, WRONG_WAY) / 2.0, 180.0),
            ))
            .collect();
        assert_eq!(wrong, want);
    }

    #[test]
    fn crossing_start_line_starts_lap_clock() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        tick(&mut g, &mut ctx);
        assert_eq!(g.race.ticks, 0);
        cross_start_line(&mut g, &mut ctx);
        ticks(&mut g, &mut ctx, 10);
        assert_eq!(g.race.ticks, 10);
    }

    fn harness(g: Racing) -> Harness<'static, Racing> {
        let mut h = Harness::builder()
            .with_size(egui::vec2(800.0, 400.0))
            .build_ui_state(|ui, g: &mut Racing| g.ui(ui.ctx()), g);
        h.run();
        h
    }

    #[test]
    fn track_error_shows_in_window_with_cyrillic_font() {
        let mut ctx = Context::default();
        let h = harness(racing(&mut ctx));
        assert!(h.query_by_label("Ошибка трассы").is_none());
        let mut g = racing(&mut ctx);
        let error = "строка 3: неизвестная деталь \"x\"";
        g.error = Some(error.into());
        let h = harness(g);
        let bottom = h.get_by_label(error).rect().max.y;
        assert!(bottom < 400.0 - HUD_MARGIN, "{bottom}");
        assert!(
            h.ctx
                .fonts(|f| f.definitions().font_data.contains_key(FONT_PATH))
        );
    }

    /// Овал в пустом каталоге теста в `target/` копии дерева: файл можно переписывать.
    fn temp_oval(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/test-dirs"
        ))
        .join(format!("racing-{name}"));
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("старый каталог");
        }
        fs::create_dir_all(&dir).expect("каталог");
        let path = dir.join("track.toml");
        fs::copy(OVAL, &path).expect("копия");
        path
    }

    /// Отъехала от старта и смотрит на файл трассы ещё раз через `WATCH_TICKS`.
    fn drive_off(g: &mut Racing, ctx: &mut Context) {
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ticks(g, ctx, engine::WATCH_TICKS - 1);
        ctx.input.on_key(KeyCode::KeyW, Released, false);
        assert!(!placed_at(g, g.start));
    }

    /// Высота мира под точкой `(x, z)`.
    fn ground(g: &Racing, x: f32, z: f32) -> Option<f32> {
        let hit = g
            .layout
            .world
            .raycast(vec3(x, 10.0, z), Vec3::NEG_Y, 20.0, |_| true);
        hit.map(|h| h.point.y)
    }

    #[test]
    fn changed_track_rebuilds_world_with_same_models_and_restarts() {
        let mut ctx = Context::default();
        let path = temp_oval("changed");
        let mut g = open(path.clone(), &mut ctx);
        let straight = g.models.get("roadStraight");
        assert!(g.layout.draws.iter().any(|d| d.model == straight));
        // Внешний угол поворота [0, 0] пуст.
        assert_eq!(ground(&g, 2.0, 2.0), None);
        g.error = Some("прежняя".into());
        g.race.best = Some(100);
        drive_off(&mut g, &mut ctx);
        let text = include_str!("testdata/bridge8.toml");
        fs::write(&path, text).expect("запись");
        tick(&mut g, &mut ctx);

        let track = Track::parse(text).expect("восьмёрка");
        let start = Spawn::start(&track.chain().expect("цепочка"));
        assert_eq!(g.start, start);
        assert!(placed_at(&g, start));
        assert_eq!((g.respawn, g.race.next, g.race.best), (start, 0, None));
        assert_eq!(g.road_cells, track.road_cells());
        assert_eq!(g.error, None);
        assert_eq!(g.layout.draws.len(), layout::place(&track).len());
        assert!(g.layout.draws.iter().any(|d| d.model == straight));
        let mid = g.models.get("roadStraightBridgeMid");
        assert!(g.layout.draws.iter().any(|d| d.model == mid));
        // Под мостом [0, 0] — его настил на уровне 1.
        let deck = ground(&g, 2.0, 2.0).expect("мост");
        assert!((deck - LEVEL * KIT_SCALE).abs() < 0.1, "{deck}");
    }

    #[test]
    fn broken_track_keeps_old_one_and_shows_error() {
        let mut ctx = Context::default();
        let path = temp_oval("broken");
        let mut g = open(path.clone(), &mut ctx);
        let draws = g.layout.draws.clone();
        for (text, error) in [
            (
                "car = \"race\"\n[[road]]\npiece = \"x\"\ncell = [0, 0]\n",
                "строка 3: неизвестная деталь \"x\"",
            ),
            ("car = \"truck\"\n", "нужен один roadStart, а их 0"),
        ] {
            drive_off(&mut g, &mut ctx);
            let pos = g.car.body.pos;
            fs::write(&path, text).expect("запись");
            tick(&mut g, &mut ctx);
            assert_eq!(g.error.as_deref(), Some(error));
            assert_eq!(g.layout.draws, draws);
            assert!(g.car.body.pos.distance(pos) < 1.0, "рестартовала");
            assert_eq!(ground(&g, 2.0, 2.0), None);
            g.restart();
        }
        let oval = fs::read_to_string(OVAL).expect("овал");
        drive_off(&mut g, &mut ctx);
        fs::write(&path, oval.replace("car = \"race\"", "car = \"truck\"")).expect("запись");
        tick(&mut g, &mut ctx);
        assert_eq!(g.error.as_deref(), Some("машины truck нет в cars/"));

        drive_off(&mut g, &mut ctx);
        fs::write(&path, &oval).expect("запись");
        tick(&mut g, &mut ctx);
        assert_eq!(g.error, None);
        assert!(placed_at(&g, g.start));
    }

    #[test]
    fn same_text_or_missing_file_does_not_rebuild() {
        let mut ctx = Context::default();
        let path = temp_oval("same");
        let mut g = open(path.clone(), &mut ctx);
        drive_off(&mut g, &mut ctx);
        fs::write(&path, fs::read_to_string(OVAL).expect("овал")).expect("запись");
        tick(&mut g, &mut ctx);
        assert!(!placed_at(&g, g.start), "рестартовала");

        drive_off(&mut g, &mut ctx);
        fs::remove_file(&path).expect("удаление");
        tick(&mut g, &mut ctx);
        assert!(!placed_at(&g, g.start), "рестартовала");
        assert_eq!(g.error, None);
    }

    #[test]
    fn init_opens_track_from_env_or_default() {
        assert_eq!(
            track_path(None),
            PathBuf::from(format!("{TRACKS_DIR}track01.toml"))
        );
        assert_eq!(
            track_path(Some("track02")),
            PathBuf::from(format!("{TRACKS_DIR}track02.toml"))
        );
        let mut ctx = Context::default();
        let g = Racing::init(&mut ctx);
        let path = track_path(std::env::var("TRACK").ok().as_deref());
        let text = fs::read_to_string(&path).expect("трасса");
        let (track, checkpoints) = load_track(&text, &g.cars).expect("трасса");
        assert_eq!(g.path, path);
        assert_eq!(g.start, Spawn::start(&checkpoints));
        assert_eq!(g.layout.draws.len(), layout::place(&track).len());
    }

    #[test]
    fn text_width_sums_advances() {
        let mut assets = Assets::default();
        let font = assets.load_font(FONT_PATH, HUD_PX);
        let font = assets.font(font);
        let one = text_width(font, "0");
        assert_eq!(one, font.glyph('0').expect("0").advance);
        assert!(one > 1.0);
        assert_eq!(text_width(font, "0\u{e9}0"), 3.0 * one);
    }
}
