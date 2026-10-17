#[cfg(test)]
mod autopilot;
mod camera;
mod car;
mod car_config;
mod editor;
mod editor_panel;
mod engine_sound;
mod garage;
mod ghost;
mod layout;
mod pieces;
mod race;
mod track;
mod tuning_panel;

use std::collections::HashSet;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};

use engine::{
    Assets, Camera2D, Camera3D, Color, Config, Context, DirectionalLight, Fog, Font, FontData,
    Game, Gfx, Handle, IVec2, Input, KeyCode, LoopId, Mesh, MeshDraw, ModelDraw, MouseButton, Pose,
    Scene, Sound, StaticWorld, Vec2, Vec3, Watcher, egui, vec2, vec3,
};

use crate::camera::Chase;
use crate::car::{Car, Controls, SUBSTEP, SUBSTEPS};
use crate::car_config::{CarConfig, TUNING};
use crate::editor::{Editor, EditorCmd};
use crate::editor_panel::{EditorPanel, PanelCmd, Switch};
use crate::engine_sound::{Rpm, SoundCmd};
use crate::garage::{Garage, GarageCmd};
use crate::ghost::{Ghost, Recorder};
use crate::layout::{Layout, Models};
use crate::pieces::KIT_SCALE;
use crate::race::{Pass, Race, lap_time};
use crate::track::{Checkpoint, TrackDraft, cell_at};
use crate::tuning_panel::{TuneCmd, TuningPanel};

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
/// Квад клетки под курсором в редакторе.
const CELL: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.4,
};
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

/// Каталоги трасс и машин; тесты, которые пишут файлы, подменяют их временными.
#[derive(Clone, Debug, PartialEq)]
struct Dirs {
    tracks: PathBuf,
    cars: PathBuf,
}

impl Dirs {
    fn assets() -> Self {
        Self {
            tracks: TRACKS_DIR.into(),
            cars: CARS_DIR.into(),
        }
    }

    /// Трасса `name`, без имени — `DEFAULT_TRACK`.
    fn track(&self, name: Option<&str>) -> PathBuf {
        (self.tracks).join(format!("{}.toml", name.unwrap_or(DEFAULT_TRACK)))
    }

    fn car(&self, model: &str) -> PathBuf {
        self.cars.join(format!("{model}.toml"))
    }
}

/// Характеристики машин из `dir` по имени файла без `.toml`: `race` раньше `race-future`.
fn load_cars(dir: &Path) -> Vec<CarConfig> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| {
            e.unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
                .path()
        })
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

/// Файлы `*.toml` каталога по имени.
fn list_tracks(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let error = |e: std::io::Error| format!("{}: {e}", dir.display());
    let mut tracks = Vec::new();
    for entry in fs::read_dir(dir).map_err(error)? {
        let path = entry.map_err(error)?.path();
        if path.extension().is_some_and(|e| e == "toml") {
            tracks.push(path);
        }
    }
    tracks.sort();
    Ok(tracks)
}

/// `trackNN.toml` после наибольшего NN списка; без таких — `track01.toml`.
fn new_track_name(tracks: &[PathBuf]) -> String {
    let nn = (tracks.iter())
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?;
            name.strip_prefix("track")?
                .strip_suffix(".toml")?
                .parse::<u32>()
                .ok()
        })
        .max()
        .map_or(1, |n| n + 1);
    format!("track{nn:02}.toml")
}

/// Трасса с цепочкой и машиной из `cars`.
fn load_track(text: &str, cars: &[CarConfig]) -> Result<(TrackDraft, Vec<Checkpoint>), String> {
    let (track, lines) = TrackDraft::parse_lines(text).map_err(|e| e.to_string())?;
    let checkpoints = (track.chain()).map_err(|e| lines.locate(e).to_string())?;
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
        let ground = world.raycast(pos + GROUND_PROBE * Vec3::Y, Vec3::NEG_Y, KIT_SCALE, |_| {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Garage,
    Race,
    Editor,
}

struct Racing {
    mode: Mode,
    garage: Garage,
    editor: Editor,
    panel: EditorPanel,
    /// Квад клетки под курсором в редакторе.
    quad: Handle<Mesh>,
    tuning: TuningPanel,
    /// Трасса заезда: последняя цельная с диска или черновик, с которым вышли из редактора.
    track: TrackDraft,
    layout: Layout,
    models: Models,
    sky: Color,
    cars: Vec<CarConfig>,
    car: Car,
    pose: Pose,
    chase: Chase,
    race: Race,
    recorder: Recorder,
    ghost: Ghost,
    motor: Handle<Sound>,
    rpm: Rpm,
    voice: Option<LoopId>,
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
    dirs: Dirs,
}

impl Racing {
    /// Гараж с предвыбором машины трассы `text`, прочитанной из `path`; дальше за файлом следит `Watcher`.
    ///
    /// # Panics
    /// Если `load_track` не принял трассу.
    fn new(path: PathBuf, text: String, dirs: Dirs, assets: &mut Assets) -> Self {
        let cars = load_cars(&dirs.cars);
        let (track, checkpoints) =
            load_track(&text, &cars).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let models = Models::load(assets);
        let selected = (cars.iter().position(|c| c.model == track.car)).expect("load_track");
        let start = Spawn::start(&checkpoints);
        let car = Self::make_car(&cars[selected], &models, assets);
        let (vertices, indices) = editor::quad();
        let tracks = list_tracks(&dirs.tracks).unwrap_or_else(|e| {
            log::warn!("{e}");
            Vec::new()
        });
        let mut racing = Self {
            mode: Mode::Garage,
            garage: Garage::new(selected),
            editor: Editor::new(track.clone(), &models, assets),
            panel: EditorPanel {
                tracks,
                ..EditorPanel::default()
            },
            quad: assets.add_mesh(vertices, indices),
            tuning: TuningPanel::default(),
            layout: Layout::new(&track, &models, assets),
            models,
            sky: Color::from_srgb8(170, 205, 235),
            chase: Chase::new(car.body.pos, car.body.rot),
            cars,
            car,
            pose: Pose::default(),
            road_cells: track.road_cells(),
            race: Race::new(&track, checkpoints),
            recorder: Recorder::default(),
            ghost: Ghost::default(),
            motor: assets.add_sound(engine_sound::synth(assets.sample_rate())),
            rpm: Rpm::default(),
            voice: None,
            start,
            respawn: start,
            upside_down: 0,
            off_road: 0,
            font: assets.load_font(FONT_PATH, HUD_PX),
            error: None,
            path,
            watch: Watcher::new(text),
            dirs,
            track,
        };
        racing.restart();
        racing
    }

    /// Изменённая на диске трасса; при `dirty` черновик не трогается, панель показывает флаг.
    fn reload_changed(&mut self, assets: &Assets) {
        match self.watch.tick(&self.path) {
            Ok(Some(_)) if self.editor.dirty() => self.panel.changed_on_disk = true,
            Ok(Some(text)) => self.load_text(&text, assets),
            Ok(None) => {}
            Err(e) => log::warn!("{e}"),
        }
    }

    /// Текст файла трассы — в черновик, цельный — и в заезд; битый — ошибка, заезд прежний.
    fn load_text(&mut self, text: &str, assets: &Assets) {
        self.panel.changed_on_disk = false;
        match load_track(text, &self.cars) {
            Ok((track, checkpoints)) => {
                self.error = None;
                self.editor.replace(track.clone(), &self.models, assets);
                self.set_race(track, checkpoints, assets);
            }
            Err(e) => {
                self.error = Some(e);
                if let Ok(draft) = TrackDraft::parse(text) {
                    self.editor.replace(draft, &self.models, assets);
                }
            }
        }
    }

    /// Заезд по `track` с прежними моделями и машиной, со старта и без лучшего круга.
    fn set_race(&mut self, track: TrackDraft, checkpoints: Vec<Checkpoint>, assets: &Assets) {
        self.layout = Layout::new(&track, &self.models, assets);
        self.road_cells = track.road_cells();
        self.start = Spawn::start(&checkpoints);
        self.race = Race::new(&track, checkpoints);
        self.ghost = Ghost::default();
        self.track = track;
        self.restart();
    }

    fn file_error(&mut self, path: &Path, e: impl std::fmt::Display) {
        self.panel.error = Some(format!("{}: {e}", path.display()));
    }

    /// Черновик — в файл трассы целиком.
    fn save(&mut self) {
        let text = self.editor.draft.to_toml();
        match fs::write(&self.path, &text) {
            Ok(()) => {
                self.editor.saved();
                self.watch.saved(text);
                self.panel.changed_on_disk = false;
                self.panel.error = None;
            }
            Err(e) => self.file_error(&self.path.clone(), e),
        }
    }

    /// Черновик — файл с диска; правки теряются.
    fn load_from_disk(&mut self, assets: &Assets) {
        match fs::read_to_string(&self.path) {
            Ok(text) => {
                self.load_text(&text, assets);
                self.watch.saved(text);
                self.panel.error = None;
            }
            Err(e) => self.file_error(&self.path.clone(), e),
        }
    }

    /// Трасса `path` в редакторе с камерой над ней; цельная — и в заезде.
    fn open_track(&mut self, path: PathBuf, assets: &Assets) {
        let read = fs::read_to_string(&path).map_err(|e| e.to_string());
        let parsed = read.and_then(|text| {
            let draft = TrackDraft::parse(&text).map_err(|e| e.to_string())?;
            Ok((text, draft))
        });
        let (text, draft) = match parsed {
            Ok(t) => t,
            Err(e) => return self.file_error(&path, e),
        };
        self.editor = Editor::new(draft, &self.models, assets);
        self.path = path;
        self.watch = Watcher::new(text.clone());
        self.error = None;
        self.panel.error = None;
        self.panel.changed_on_disk = false;
        if let Ok((track, checkpoints)) = load_track(&text, &self.cars) {
            self.set_race(track, checkpoints, assets);
        }
    }

    /// Пустая трасса с машиной гаража — новым файлом в каталог трасс и в редактор.
    fn new_track(&mut self, assets: &Assets) {
        let draft = TrackDraft {
            car: self.cars[self.garage.selected].model.clone(),
            roads: Vec::new(),
            decor: Vec::new(),
        };
        let dir = &self.dirs.tracks;
        let created = list_tracks(dir).and_then(|tracks| {
            let path = dir.join(new_track_name(&tracks));
            fs::write(&path, draft.to_toml()).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((path, list_tracks(dir)?))
        });
        match created {
            Ok((path, tracks)) => {
                self.panel.tracks = tracks;
                self.open_track(path, assets);
            }
            Err(e) => self.panel.error = Some(e),
        }
    }

    fn switch(&mut self, to: Switch, assets: &Assets) {
        match to {
            Switch::Open(path) => self.open_track(path, assets),
            Switch::New => self.new_track(assets),
        }
    }

    fn apply_panel(&mut self, cmd: PanelCmd, assets: &Assets) {
        match cmd {
            PanelCmd::Edit(cmd) => self.editor.apply(cmd, &self.models, assets),
            PanelCmd::Switch(to) if self.editor.dirty() => self.panel.pending = Some(to),
            PanelCmd::Switch(to) => self.switch(to, assets),
            PanelCmd::Save => self.save(),
            PanelCmd::LoadFromDisk => self.load_from_disk(assets),
            PanelCmd::Discard => {
                if let Some(to) = self.panel.pending.take() {
                    self.switch(to, assets);
                }
            }
            PanelCmd::Keep => self.panel.pending = None,
        }
    }

    /// Цельный черновик — заезд по нему машиной гаража; изменённый — заново, без лучшего круга.
    fn leave_editor(&mut self, assets: &Assets) {
        let Ok(checkpoints) = self.editor.draft.chain() else {
            return;
        };
        if self.editor.draft != self.track {
            self.set_race(self.editor.draft.clone(), checkpoints, assets);
        }
        self.go(assets);
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
        self.recorder.cut();
        self.upside_down = 0;
        self.off_road = 0;
    }

    fn restart(&mut self) {
        self.race.restart();
        self.recorder.restart();
        self.respawn = self.start;
        self.spawn(self.start);
    }

    /// Заезд машиной, выбранной в гараже, со старта.
    fn go(&mut self, assets: &Assets) {
        self.car = Self::make_car(&self.cars[self.garage.selected], &self.models, assets);
        self.restart();
        self.mode = Mode::Race;
    }

    /// Команды панели — правка выбранной машины; в заезде `Restart`-поле ставит её заново на последний чекпоинт.
    fn update_tuning(&mut self, assets: &Assets) {
        let config = &mut self.cars[self.garage.selected];
        for cmd in mem::take(&mut self.tuning.queue) {
            let path = self.dirs.car(&config.model);
            let done = match cmd {
                TuneCmd::Set(i, value) => {
                    *(TUNING[i].field)(config) = value;
                    continue;
                }
                TuneCmd::Save => fs::write(&path, config.to_toml()).map_err(|e| e.to_string()),
                TuneCmd::Reset => fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|text| CarConfig::parse(&text).map_err(|e| e.to_string()))
                    .map(|c| *config = c),
            };
            self.tuning.error = done.err().map(|e| format!("{}: {e}", path.display()));
        }
        if self.mode != Mode::Race || self.car.config == *config {
            return;
        }
        self.car.retune(config);
        if self.car.config != *config {
            self.car = Self::make_car(config, &self.models, assets);
            self.spawn(self.respawn);
        }
    }

    fn update_garage(&mut self, ctx: &Context) {
        let mut go = ctx.input.just_pressed(KeyCode::Enter);
        for cmd in mem::take(&mut self.garage.queue) {
            match cmd {
                GarageCmd::Select(i) => self.garage.selected = i,
                GarageCmd::Go => go = true,
            }
        }
        self.garage.tick();
        if go {
            self.go(&ctx.assets);
        }
    }

    fn update_race(&mut self, ctx: &Context) {
        if ctx.input.just_pressed(KeyCode::KeyG) {
            self.restart();
            self.mode = Mode::Garage;
            return;
        }
        if ctx.input.just_pressed(KeyCode::KeyR) {
            self.spawn(self.respawn);
        }
        if ctx.input.just_pressed(KeyCode::F2) {
            self.tuning.open = !self.tuning.open;
        }
        self.drive(controls(&ctx.input));
    }

    /// F1 — в заезд, если цепочка цела; Ctrl+S — сохранить, Ctrl+Z — отменить.
    fn update_editor(&mut self, ctx: &Context) {
        let input = &ctx.input;
        let ctrl = input.is_down(KeyCode::ControlLeft) || input.is_down(KeyCode::ControlRight);
        if ctrl && input.just_pressed(KeyCode::KeyS) {
            self.panel.queue.push(PanelCmd::Save);
        }
        if ctrl && input.just_pressed(KeyCode::KeyZ) {
            self.panel.queue.push(PanelCmd::Edit(EditorCmd::Undo));
        }
        for cmd in mem::take(&mut self.panel.queue) {
            self.apply_panel(cmd, &ctx.assets);
        }
        if input.just_pressed(KeyCode::F1) {
            self.leave_editor(&ctx.assets);
            return;
        }
        self.editor.control(&ctx.input);
        if ctx.input.mouse_just_pressed(MouseButton::Left)
            && let Some(ray) = ctx.cursor_ray()
        {
            self.editor.click(ray, &self.models, &ctx.assets);
        }
    }

    fn update_sound(&mut self, ctx: &mut Context) {
        match (
            engine_sound::command(self.mode, &self.rpm, self.voice.is_some()),
            self.voice,
        ) {
            // Голос стартует с `rate = 1`: тишина до первого `Set` в следующем тике.
            (SoundCmd::Start, _) => {
                self.voice = ctx.audio.start_loop(ctx.assets.sound(self.motor), 0.0);
            }
            (SoundCmd::Set(rate, volume), Some(id)) => ctx.audio.set_loop(id, rate, volume),
            (SoundCmd::Stop, Some(id)) => {
                ctx.audio.stop_loop(id);
                self.voice = None;
            }
            _ => {}
        }
    }

    /// Свет и туман общие у заезда и гаража.
    fn scene(&self, camera: Camera3D, models: Vec<ModelDraw>) -> Scene {
        Scene {
            camera,
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
        }
    }

    /// Выбранная машина в позе покоя на подиуме.
    fn draw_garage(&self, alpha: f32) -> Scene {
        let model = self.models.get(&self.cars[self.garage.selected].model);
        let models = vec![
            ModelDraw {
                model: self.models.get(garage::PODIUM),
                transform: self.garage.podium(alpha),
                pose: None,
                tint: Color::WHITE,
            },
            ModelDraw {
                model,
                transform: self.garage.transform(alpha),
                pose: None,
                tint: Color::WHITE,
            },
        ];
        self.scene(Garage::camera(), models)
    }

    fn draw_race(&mut self, ctx: &mut Context, alpha: f32) -> Scene {
        let model = self.models.get(&self.car.config.model);
        self.car.pose(ctx.assets.model(model), &mut self.pose);
        let (pos, rot) = self.car.lerp(alpha);
        let mut models = self.layout.draws.clone();
        if self.race.started {
            models.extend(self.ghost.draw(self.race.ticks, alpha, &self.models));
        }
        models.push(ModelDraw {
            model,
            transform: self.car.transform(alpha),
            pose: Some(self.pose.clone()),
            tint: Color::WHITE,
        });
        self.draw_hud(&mut ctx.gfx, &ctx.assets);
        self.scene(self.chase.camera(alpha, pos, rot), models)
    }

    /// Черновик, деталь инструмента и клетка под курсором.
    fn draw_editor(&self, ctx: &Context, alpha: f32) -> Scene {
        let e = &self.editor;
        let mut models = e.draws();
        let mut meshes = Vec::new();
        if let Some(ray) = ctx.cursor_ray() {
            models.extend(e.preview(ray, &self.models));
            meshes.extend(e.cell_quad(ray).map(|transform| MeshDraw {
                mesh: self.quad,
                texture: Handle::WHITE,
                color: CELL,
                transform,
            }));
        }
        Scene {
            meshes,
            ..self.scene(e.camera(alpha), models)
        }
    }

    fn pass_checkpoint(&mut self) -> Option<Pass> {
        let body = &self.car.body;
        let pass = self.race.tick(self.car.prev.0, body.pos, body.vel);
        if let Some(p) = pass {
            self.respawn = Spawn::at(&self.race.checkpoints[p.checkpoint], &self.layout.world);
        }
        pass
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

    /// Тик машины с управлением `controls`: чекпоинты, респаун, запись круга, камера.
    fn drive(&mut self, controls: Controls) {
        self.car.tick(&self.layout.world, controls);
        let speed = self.car.forward_speed().abs() / self.car.config.max_speed;
        let grounded = self.car.wheels.iter().any(|w| w.ground.is_some());
        self.rpm.tick(speed, controls.throttle, grounded);
        let pass = self.pass_checkpoint();
        self.auto_respawn();
        // Сэмпл — после респауна: в кадре машина уже на новом месте.
        let origin = self.car.transform(1.0);
        if let Some(lap) = self.recorder.tick(&self.car.config.model, origin, pass) {
            self.ghost.offer(lap);
        }
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
        let dirs = Dirs::assets();
        let path = dirs.track(std::env::var("TRACK").ok().as_deref());
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Self::new(path, text, dirs, &mut ctx.assets)
    }

    fn update(&mut self, ctx: &mut Context) {
        self.reload_changed(&ctx.assets);
        self.update_tuning(&ctx.assets);
        match self.mode {
            Mode::Editor => self.update_editor(ctx),
            _ if ctx.input.just_pressed(KeyCode::F1) => self.mode = Mode::Editor,
            Mode::Garage => self.update_garage(ctx),
            Mode::Race => self.update_race(ctx),
        }
        self.update_sound(ctx);
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        ctx.gfx.clear_color = self.sky;
        let scene = match self.mode {
            Mode::Garage => self.draw_garage(alpha),
            Mode::Race => self.draw_race(ctx, alpha),
            Mode::Editor => self.draw_editor(ctx, alpha),
        };
        ctx.gfx.scene = Some(scene);
    }

    fn ui(&mut self, ui: &egui::Context) {
        engine::ui::set_font(ui, FONT_PATH);
        let config = &self.cars[self.garage.selected];
        match self.mode {
            Mode::Garage => {
                self.garage.ui(ui, &self.cars);
                self.tuning.ui(ui, config, egui::Align2::LEFT_TOP);
            }
            Mode::Race if self.tuning.open => {
                self.tuning.ui(ui, config, egui::Align2::RIGHT_TOP);
            }
            Mode::Editor => self.panel.ui(ui, &self.editor, &self.path),
            Mode::Race => {}
        }
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
    use engine::egui::accesskit::Role;
    use engine::{Mat4, Quat, Ray, Sprite, Triangle, text};

    use super::*;
    use crate::editor::Tool;
    use crate::ghost::{Lap, Sample};
    use crate::layout::{self, ROAD};
    use crate::pieces::{LEVEL, decor_piece};
    use crate::track::{DraftError, Entry};

    const OVAL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/testdata/oval.toml");

    /// Сразу в заезде машиной трассы.
    fn open(path: PathBuf, ctx: &mut Context) -> Racing {
        let text = fs::read_to_string(&path).expect("трасса");
        let mut g = Racing::new(path, text, Dirs::assets(), &mut ctx.assets);
        g.go(&ctx.assets);
        g
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
        let names: Vec<String> = load_cars(CARS_DIR.as_ref())
            .into_iter()
            .map(|c| c.model)
            .collect();
        assert_eq!(
            names,
            ["hatchback-sports", "race", "race-future", "sedan-sports"]
        );
    }

    #[test]
    fn motor_revs_with_gas_and_mute_start_is_retried() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        ticks(&mut g, &mut ctx, 30);
        let idle = g.rpm.rate();
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ticks(&mut g, &mut ctx, 60);
        assert!(g.rpm.rate() > 2.0 * idle, "{} vs {idle}", g.rpm.rate());
        assert!(g.rpm.rate() < engine::audio::MAX_RATE, "{}", g.rpm.rate());
        assert_eq!(g.voice, None);
        let sound = engine_sound::command(g.mode, &g.rpm, g.voice.is_some());
        assert_eq!(sound, SoundCmd::Start);
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

    /// Законченный записью круг; конец — подставной.
    fn finish_lap(g: &mut Racing) -> Lap {
        let end = Some(Pass {
            checkpoint: 0,
            lap: Some(0),
        });
        (g.recorder.tick("", Mat4::IDENTITY, end)).expect("круг пишется")
    }

    #[test]
    fn ghost_replays_model_origin_like_car_transform() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        cross_start_line(&mut g, &mut ctx);
        let at_start = g.car.transform(1.0);
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ctx.input.on_key(KeyCode::KeyD, Pressed, false);
        let mut want = Vec::new();
        for _ in 0..60 {
            tick(&mut g, &mut ctx);
            want.push((g.race.ticks, g.car.transform(0.3)));
        }
        let lap = finish_lap(&mut g);
        assert_eq!(lap.car, "race");
        assert_eq!(lap.transform(0, 0.3), Some(at_start));
        assert!(
            (g.car.body.rot * Vec3::Z).dot(Vec3::Z).abs() < 0.99,
            "не повернула"
        );
        for (t, want) in want {
            let got = lap.transform(t, 0.3).expect("сэмпл");
            assert!(got.abs_diff_eq(want, 1e-3), "тик {t}: {got} != {want}");
        }
    }

    #[test]
    fn respawn_cuts_ghost_interpolation() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        cross_start_line(&mut g, &mut ctx);
        ticks(&mut g, &mut ctx, 10);
        tap(&mut g, &mut ctx, KeyCode::KeyR);
        tick(&mut g, &mut ctx);
        let cuts: Vec<usize> = (finish_lap(&mut g).samples.iter())
            .enumerate()
            .filter_map(|(k, s)| s.cut.then_some(k))
            .collect();
        assert_eq!(cuts, [11]);
    }

    /// Круг из трёх одинаковых сэмплов `transform` машины `car`.
    fn still_lap(car: &str, transform: Mat4) -> Lap {
        let sample = Sample {
            transform,
            cut: false,
        };
        Lap {
            car: car.into(),
            samples: vec![sample; 3],
        }
    }

    fn ghosts(ctx: &mut Context) -> Vec<ModelDraw> {
        let scene = ctx.gfx.scene.take().expect("сцена");
        scene
            .models
            .into_iter()
            .filter(|m| m.tint == ghost::TINT)
            .collect()
    }

    #[test]
    fn ghost_of_other_car_drawn_by_its_model_during_lap_only() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        let at = Mat4::from_translation(vec3(1.0, 2.0, 3.0));
        g.ghost.best = Some(still_lap("sedan-sports", at));
        g.draw(&mut ctx, 0.5);
        assert_eq!(ghosts(&mut ctx), [], "до старта");
        cross_start_line(&mut g, &mut ctx);
        tick(&mut g, &mut ctx);
        g.draw(&mut ctx, 0.5);
        let want = ModelDraw {
            model: g.models.get("sedan-sports"),
            transform: at,
            pose: None,
            tint: ghost::TINT,
        };
        assert_eq!(ghosts(&mut ctx), [want]);
        assert_eq!(g.car.config.model, "race");
        ticks(&mut g, &mut ctx, 2);
        g.draw(&mut ctx, 0.5);
        assert_eq!(ghosts(&mut ctx), [], "после конца круга");
        g.race.ticks = 1;
        g.mode = Mode::Garage;
        g.draw(&mut ctx, 0.5);
        assert_eq!(ghosts(&mut ctx), [], "в гараже");
    }

    fn car_index(g: &Racing, model: &str) -> usize {
        (g.cars.iter().position(|c| c.model == model)).expect("машина")
    }

    #[test]
    fn starts_in_garage_with_track_car_preselected() {
        let mut ctx = Context::default();
        let path = temp_oval("garage");
        let oval = fs::read_to_string(&path).expect("овал");
        let text = oval.replace("car = \"race\"", "car = \"sedan-sports\"");
        fs::write(&path, &text).expect("запись");
        let mut g = Racing::new(path, text, Dirs::assets(), &mut ctx.assets);
        assert_eq!(g.mode, Mode::Garage);
        assert_eq!(g.garage.selected, car_index(&g, "sedan-sports"));
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ticks(&mut g, &mut ctx, 30);
        assert!(placed_at(&g, g.start), "машина ехала в гараже");
        assert_eq!(g.race.ticks, 0);
    }

    #[test]
    fn enter_starts_race_with_selected_car() {
        let mut ctx = Context::default();
        let path = PathBuf::from(OVAL);
        let text = fs::read_to_string(&path).expect("овал");
        let mut g = Racing::new(path, text, Dirs::assets(), &mut ctx.assets);
        g.garage
            .queue
            .push(GarageCmd::Select(car_index(&g, "race-future")));
        tick(&mut g, &mut ctx);
        assert_eq!(
            (g.mode, g.car.config.model.as_str()),
            (Mode::Garage, "race")
        );
        tap(&mut g, &mut ctx, KeyCode::Enter);
        assert_eq!(g.mode, Mode::Race);
        assert_eq!(g.car.config.model, "race-future");
        assert!(placed_at(&g, g.start));
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ticks(&mut g, &mut ctx, 10);
        assert!(g.car.body.vel.length() > 1.0, "заезд не идёт");

        g.garage
            .queue
            .extend([GarageCmd::Select(car_index(&g, "race")), GarageCmd::Go]);
        g.mode = Mode::Garage;
        tick(&mut g, &mut ctx);
        assert_eq!((g.mode, g.car.config.model.as_str()), (Mode::Race, "race"));
    }

    #[test]
    fn g_returns_to_garage_and_restarts_race() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        cross_start_line(&mut g, &mut ctx);
        ticks(&mut g, &mut ctx, 20);
        g.race.best = Some(500);
        g.ghost.best = Some(still_lap("race", Mat4::IDENTITY));
        tap(&mut g, &mut ctx, KeyCode::KeyG);
        assert_eq!(g.mode, Mode::Garage);
        assert!(placed_at(&g, g.start));
        assert_eq!((g.respawn, g.race.next, g.race.ticks), (g.start, 0, 0));
        assert_eq!(g.race.best, Some(500), "лучший круг трассы не сбрасывается");
        assert!(g.ghost.best.is_some());
        assert_eq!(g.garage.selected, car_index(&g, "race"));
        tap(&mut g, &mut ctx, KeyCode::Enter);
        assert_eq!(g.mode, Mode::Race);
    }

    #[test]
    fn tab_no_longer_changes_car() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ticks(&mut g, &mut ctx, 10);
        tap(&mut g, &mut ctx, KeyCode::Tab);
        assert_eq!(g.car.config.model, "race");
        assert!(!placed_at(&g, g.start), "рестартовала");
        assert_eq!(g.mode, Mode::Race);
    }

    #[test]
    fn garage_draws_selected_car_on_podium_without_hud() {
        let mut ctx = Context::default();
        let path = PathBuf::from(OVAL);
        let text = fs::read_to_string(&path).expect("овал");
        let mut g = Racing::new(path, text, Dirs::assets(), &mut ctx.assets);
        g.garage.selected = car_index(&g, "hatchback-sports");
        ticks(&mut g, &mut ctx, 3);
        g.draw(&mut ctx, 0.5);
        let scene = ctx.gfx.scene.take().expect("сцена");
        assert_eq!(scene.camera, Garage::camera());
        let [podium, car] = scene.models.as_slice() else {
            panic!("{} моделей", scene.models.len());
        };
        assert_eq!(podium.model, g.models.get(garage::PODIUM));
        assert_eq!(podium.transform, g.garage.podium(0.5));
        assert_eq!(car.model, g.models.get("hatchback-sports"));
        assert_eq!(car.transform, g.garage.transform(0.5));
        assert_ne!(car.transform, Mat4::IDENTITY);
        assert_eq!(car.pose, None);
        assert!(scene.sun.is_some());
        assert_eq!(ctx.gfx.clear_color, g.sky);
        assert_eq!(hud(&ctx), []);
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
    fn spawn_stands_mid_cell_past_checkpoint_on_ground_below() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        flat(&mut g, 0.0);
        let c = Checkpoint {
            road: 0,
            origin: vec3(8.0, 0.5, 2.0),
            normal: Vec3::NEG_X,
            height: f32::INFINITY,
        };
        let at = Spawn::at(&c, &g.layout.world);
        assert!(at.pos.abs_diff_eq(vec3(6.0, 0.0, 2.0), 1e-5), "{at:?}");
        assert_eq!(at.dir, Vec3::NEG_X);
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
        let scene = ctx.gfx.scene.take().expect("сцена");
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

    /// Кадр показан камерой редактора на поверхности `size`, курсор в её середине.
    fn show_editor(g: &Racing, ctx: &mut Context) {
        let size = engine::UVec2::new(1280, 720);
        ctx.shown = Some(engine::Shown {
            camera: Camera2D {
                center: Vec2::ZERO,
                size: VIEW,
            },
            viewport: engine::Viewport {
                x: 0.0,
                y: 0.0,
                w: VIEW.x,
                h: VIEW.y,
            },
            scene: Some(engine::ShownScene {
                camera: g.editor.camera(1.0),
                size,
            }),
        });
        ctx.input.on_cursor(Some(size.as_vec2() / 2.0));
    }

    #[test]
    fn f1_toggles_editor_from_race_and_garage() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        tap(&mut g, &mut ctx, KeyCode::F1);
        assert_eq!(g.mode, Mode::Editor);
        tap(&mut g, &mut ctx, KeyCode::KeyG);
        assert_eq!(g.mode, Mode::Editor, "G — не из редактора");
        g.car.place(vec3(0.0, 50.0, 0.0), 0.0);
        tap(&mut g, &mut ctx, KeyCode::F1);
        assert_eq!(g.mode, Mode::Race);
        assert!(placed_at(&g, g.start));
        g.mode = Mode::Garage;
        tap(&mut g, &mut ctx, KeyCode::F1);
        assert_eq!(g.mode, Mode::Editor);
    }

    #[test]
    fn editor_draws_preview_and_cell_quad_of_one_mesh_under_cursor() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        let quads: Vec<_> = (ctx.assets.take_pending_meshes().into_iter())
            .filter(|m| m.handle == g.quad)
            .collect();
        let (vertices, indices) = editor::quad();
        assert_eq!(quads.len(), 1);
        assert_eq!(
            (&quads[0].vertices, &quads[0].indices),
            (&vertices, &indices)
        );
        tap(&mut g, &mut ctx, KeyCode::F1);
        tap(&mut g, &mut ctx, KeyCode::PageUp);
        g.draw(&mut ctx, 1.0);
        let scene = ctx.gfx.scene.take().expect("сцена");
        assert_eq!(
            (scene.models, scene.meshes),
            (g.editor.layout.draws.clone(), vec![])
        );
        show_editor(&g, &mut ctx);
        let ray = ctx.cursor_ray().expect("луч");
        for _ in 0..2 {
            g.draw(&mut ctx, 1.0);
        }
        let scene = ctx.gfx.scene.take().expect("сцена");
        assert_eq!(scene.camera, g.editor.camera(1.0));
        let mut want = g.editor.layout.draws.clone();
        want.extend(g.editor.preview(ray, &g.models));
        assert!(want.len() > g.editor.layout.draws.len());
        assert_eq!(scene.models, want);
        let quad = MeshDraw {
            mesh: g.quad,
            texture: Handle::WHITE,
            color: CELL,
            transform: g.editor.cell_quad(ray).expect("квад"),
        };
        assert_eq!(scene.meshes, [quad]);
        assert_eq!(ctx.assets.take_pending_meshes(), []);
    }

    #[test]
    fn editor_click_edits_draft_through_cursor_ray() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        tap(&mut g, &mut ctx, KeyCode::F1);
        tap(&mut g, &mut ctx, KeyCode::PageUp);
        show_editor(&g, &mut ctx);
        let roads = g.editor.draft.roads.len();
        tick(&mut g, &mut ctx);
        assert_eq!(g.editor.draft.roads.len(), roads, "без клика");
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        assert_eq!(g.editor.draft.roads.len(), roads + 1);
        let ray = ctx.cursor_ray().expect("луч");
        assert_eq!(
            g.editor.draft.roads.last().map(|r| (r.cell, r.level)),
            (g.editor.cell_quad(ray))
                .and_then(|m| cell_at(m.transform_point3(vec3(0.5, 0.0, 0.5))))
        );
        ctx.shown = None;
        ctx.input.on_button(MouseButton::Left, Released);
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        assert_eq!(
            g.editor.draft.roads.len(),
            roads + 1,
            "без показанного кадра"
        );
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
    fn garage_window_only_in_garage() {
        let mut ctx = Context::default();
        let h = harness(racing(&mut ctx));
        assert!(h.query_by_label("Гараж").is_none());
        let mut g = racing(&mut ctx);
        g.mode = Mode::Garage;
        let mut h = harness(g);
        h.get_by_label("Ехать").click();
        h.run();
        assert_eq!(h.state().garage.queue, [GarageCmd::Go]);
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

    /// Пустой каталог теста в `target/` копии дерева: файлы в нём можно переписывать.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/test-dirs"
        ))
        .join(format!("racing-{name}"));
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("старый каталог");
        }
        fs::create_dir_all(&dir).expect("каталог");
        dir
    }

    fn temp_oval(name: &str) -> PathBuf {
        let path = temp_dir(name).join("track.toml");
        fs::copy(OVAL, &path).expect("копия");
        path
    }

    /// Трассы из `assets/`, машины — копии в каталоге теста.
    fn temp_cars(name: &str) -> Dirs {
        let cars = temp_dir(name);
        for entry in fs::read_dir(CARS_DIR).expect("машины") {
            let from = entry.expect("машина").path();
            fs::copy(&from, cars.join(from.file_name().expect("имя"))).expect("копия");
        }
        Dirs {
            tracks: TRACKS_DIR.into(),
            cars,
        }
    }

    fn field(label: &str) -> usize {
        (TUNING.iter().position(|t| t.label == label)).expect("поле")
    }

    #[test]
    fn live_field_retunes_car_on_the_go() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ticks(&mut g, &mut ctx, 10);
        let body = g.car.body;
        g.tuning.queue.push(TuneCmd::Set(field("mass"), 2000.0));
        tick(&mut g, &mut ctx);
        assert_eq!(g.tuning.queue, []);
        assert_eq!(g.cars[g.garage.selected].mass, 2000.0);
        assert_eq!(g.car.config, g.cars[g.garage.selected]);
        assert_eq!(g.car.body.inv_mass, 2000.0_f32.recip());
        assert_eq!(g.car.prev, (body.pos, body.rot), "рестартовала");
        assert!(g.car.body.vel.length() > body.vel.length());
    }

    #[test]
    fn max_speed_slider_mid_race_caps_speed_without_restart() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        flat(&mut g, 0.0);
        g.road_cells = (-50..50)
            .flat_map(|x| (-50..50).map(move |z| (IVec2::new(x, z), 0)))
            .collect();
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        ticks(&mut g, &mut ctx, 60);
        let (pos, speed) = (g.car.body.pos, g.car.forward_speed());
        let cap = speed + 3.0;
        assert!(cap < g.car.config.max_speed - 10.0, "{speed}");
        g.tuning.queue.push(TuneCmd::Set(field("max_speed"), cap));
        tick(&mut g, &mut ctx);
        assert!(across(g.car.body.pos, pos) < 1.0, "рестартовала");
        let mut top = 0.0f32;
        for _ in 0..5 * 60 {
            tick(&mut g, &mut ctx);
            top = top.max(g.car.forward_speed());
        }
        assert!(top > cap - 0.5 && top < cap + 0.1, "{top} при {cap}");
    }

    #[test]
    fn restart_field_rebuilds_car_at_last_checkpoint() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        cross_start_line(&mut g, &mut ctx);
        ticks(&mut g, &mut ctx, 30);
        let first = Spawn::at(&g.race.checkpoints[0], &g.layout.world);
        for (label, value) in [("travel", 0.3), ("com_offset.y", -0.1)] {
            g.tuning.queue.push(TuneCmd::Set(field(label), value));
            tick(&mut g, &mut ctx);
            assert_eq!(g.car.config, g.cars[g.garage.selected]);
            assert_eq!(*(TUNING[field(label)].field)(&mut g.car.config), value);
            assert!(placed_at(&g, first), "{label}");
            assert_eq!(g.race.next, 1, "заезд рестартовал");
            ticks(&mut g, &mut ctx, 10);
        }
    }

    #[test]
    fn garage_tuning_changes_selected_car_for_next_race() {
        let mut ctx = Context::default();
        let path = PathBuf::from(OVAL);
        let text = fs::read_to_string(&path).expect("овал");
        let mut g = Racing::new(path, text, Dirs::assets(), &mut ctx.assets);
        let (speed, travel) = (field("max_speed"), field("travel"));
        let before = g.car.config.clone();
        g.tuning
            .queue
            .extend([TuneCmd::Set(speed, 20.0), TuneCmd::Set(travel, 0.3)]);
        tick(&mut g, &mut ctx);
        assert_eq!(g.car.config, before, "машина гаража не едет");
        tap(&mut g, &mut ctx, KeyCode::Enter);
        assert_eq!((g.car.config.max_speed, g.car.config.travel), (20.0, 0.3));
        assert!(placed_at(&g, g.start));
    }

    #[test]
    fn save_writes_car_file_and_reset_reads_it() {
        let mut ctx = Context::default();
        let dirs = temp_cars("tuning");
        let file = dirs.car("race");
        let text = fs::read_to_string(OVAL).expect("овал");
        let mut g = Racing::new(OVAL.into(), text, dirs, &mut ctx.assets);
        g.go(&ctx.assets);
        let mass = field("mass");
        g.tuning
            .queue
            .extend([TuneCmd::Set(mass, 1234.5), TuneCmd::Save]);
        tick(&mut g, &mut ctx);
        let saved = CarConfig::parse(&fs::read_to_string(&file).expect("файл")).expect("разбор");
        assert_eq!(saved, g.cars[g.garage.selected]);
        assert_eq!(saved.mass, 1234.5);
        assert_eq!(g.tuning.error, None);

        g.tuning
            .queue
            .extend([TuneCmd::Set(mass, 999.0), TuneCmd::Reset]);
        tick(&mut g, &mut ctx);
        assert_eq!(g.cars[g.garage.selected], saved);
        assert_eq!(g.car.config, saved);

        fs::remove_file(&file).expect("удаление");
        g.tuning
            .queue
            .extend([TuneCmd::Set(mass, 999.0), TuneCmd::Reset]);
        tick(&mut g, &mut ctx);
        let error = g.tuning.error.clone().expect("ошибка");
        assert!(
            error.starts_with(&format!("{}: ", file.display())),
            "{error}"
        );
        assert_eq!(g.car.config.mass, 999.0, "сброс без файла");

        fs::write(&file, "model = 1").expect("запись");
        g.tuning.queue.push(TuneCmd::Reset);
        tick(&mut g, &mut ctx);
        assert_ne!(g.tuning.error.as_deref(), Some(error.as_str()));
        assert!(g.tuning.error.is_some());
        assert_eq!(g.car.config.mass, 999.0);

        g.tuning.queue.push(TuneCmd::Save);
        tick(&mut g, &mut ctx);
        assert_eq!(g.tuning.error, None);
        let saved = CarConfig::parse(&fs::read_to_string(&file).expect("файл")).expect("разбор");
        assert_eq!(saved.mass, 999.0);
    }

    #[test]
    fn save_into_missing_dir_shows_error() {
        let mut ctx = Context::default();
        let dirs = temp_cars("tuning-missing");
        let text = fs::read_to_string(OVAL).expect("овал");
        let mut g = Racing::new(OVAL.into(), text, dirs.clone(), &mut ctx.assets);
        fs::remove_dir_all(&dirs.cars).expect("удаление");
        g.tuning.queue.push(TuneCmd::Save);
        tick(&mut g, &mut ctx);
        let error = g.tuning.error.expect("ошибка");
        assert!(
            error.starts_with(&dirs.car("race").display().to_string()),
            "{error}"
        );
    }

    #[test]
    fn tuning_panel_by_f2_in_race_and_always_in_garage() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        tap(&mut g, &mut ctx, KeyCode::F2);
        assert!(g.tuning.open);
        let mut h = harness(g);
        assert!(h.query_by_label("Характеристики").is_some());
        let speed = h.get_by_role_and_label(Role::Slider, "max_speed").rect();
        assert!(speed.max.x > 400.0, "не справа: {speed:?}");
        h.state_mut().tuning.open = false;
        h.run();
        assert!(h.query_by_label("Характеристики").is_none());
        h.state_mut().mode = Mode::Garage;
        h.run();
        assert!(h.query_by_label("Характеристики").is_some());
        assert!(
            h.get_by_role_and_label(Role::Slider, "max_speed")
                .rect()
                .min
                .x
                < 400.0,
            "не слева"
        );

        let mut g = racing(&mut ctx);
        tap(&mut g, &mut ctx, KeyCode::F2);
        tap(&mut g, &mut ctx, KeyCode::F2);
        assert!(!g.tuning.open);
        g.mode = Mode::Garage;
        tap(&mut g, &mut ctx, KeyCode::F2);
        assert!(!g.tuning.open, "F2 в гараже");
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
        g.ghost.best = Some(still_lap("race", Mat4::IDENTITY));
        drive_off(&mut g, &mut ctx);
        let text = include_str!("testdata/bridge8.toml");
        fs::write(&path, text).expect("запись");
        tick(&mut g, &mut ctx);

        let track = TrackDraft::parse(text).expect("восьмёрка");
        let start = Spawn::start(&track.chain().expect("цепочка"));
        assert_eq!(g.start, start);
        assert!(placed_at(&g, start));
        assert_eq!((g.respawn, g.race.next, g.race.best), (start, 0, None));
        assert_eq!(g.ghost.best, None);
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
        let dirs = Dirs::assets();
        assert_eq!(
            dirs.track(None),
            PathBuf::from(format!("{TRACKS_DIR}track01.toml"))
        );
        assert_eq!(
            dirs.track(Some("track02")),
            PathBuf::from(format!("{TRACKS_DIR}track02.toml"))
        );
        assert_eq!(
            dirs.car("race"),
            PathBuf::from(format!("{CARS_DIR}race.toml"))
        );
        let mut ctx = Context::default();
        let g = Racing::init(&mut ctx);
        assert_eq!(g.dirs, dirs);
        let path = dirs.track(std::env::var("TRACK").ok().as_deref());
        let text = fs::read_to_string(&path).expect("трасса");
        let (track, checkpoints) = load_track(&text, &g.cars).expect("трасса");
        assert_eq!(g.path, path);
        assert_eq!(g.start, Spawn::start(&checkpoints));
        assert_eq!(g.layout.draws.len(), layout::place(&track).len());
    }

    #[test]
    fn new_track_name_follows_largest_number() {
        assert_eq!(new_track_name(&[]), "track01.toml");
        let tracks = [
            "/t/track01.toml",
            "/t/track03.toml",
            "/t/oval.toml",
            "/t/track7.txt",
        ]
        .map(PathBuf::from);
        assert_eq!(new_track_name(&tracks), "track04.toml");
    }

    #[test]
    fn list_tracks_takes_toml_files_by_name() {
        let dir = temp_tracks("list");
        fs::write(dir.tracks.join("notes.txt"), "").expect("запись");
        let names: Vec<_> = (list_tracks(&dir.tracks).expect("список").iter())
            .map(|p| p.file_name().expect("имя").to_owned())
            .collect();
        assert_eq!(names, ["track01.toml", "track03.toml"]);
        let missing = dir.tracks.join("нет");
        let error = list_tracks(&missing).expect_err("нет каталога");
        assert!(error.starts_with(&missing.display().to_string()), "{error}");
    }

    /// Овал — `track01`, восьмёрка — `track03` каталога теста; машины из `assets/`.
    fn temp_tracks(name: &str) -> Dirs {
        let tracks = temp_dir(name);
        fs::copy(OVAL, tracks.join("track01.toml")).expect("копия");
        fs::write(tracks.join("track03.toml"), EIGHT).expect("копия");
        Dirs {
            tracks,
            cars: CARS_DIR.into(),
        }
    }

    const EIGHT: &str = include_str!("testdata/bridge8.toml");

    /// В редакторе овала `track01` каталога теста.
    fn editing(name: &str, ctx: &mut Context) -> Racing {
        let dirs = temp_tracks(name);
        let path = dirs.track(None);
        let text = fs::read_to_string(&path).expect("овал");
        let mut g = Racing::new(path, text, dirs, &mut ctx.assets);
        tap(&mut g, ctx, KeyCode::F1);
        assert_eq!(g.mode, Mode::Editor);
        g
    }

    /// Отвесно вниз над точкой `x, z`.
    fn down(x: f32, z: f32) -> Ray {
        Ray {
            origin: vec3(x, 30.0, z),
            dir: Vec3::NEG_Y,
        }
    }

    /// Конус внутри овала: цепочка цела, черновик `dirty`.
    fn add_cone(g: &mut Racing, ctx: &mut Context) {
        let piece = decor_piece("cone").expect("декор");
        let cone = Tool::Decor { piece, yaw: 0.0 };
        g.panel.queue.push(PanelCmd::Edit(EditorCmd::Tool(cone)));
        tick(g, ctx);
        assert!(g.editor.click(down(14.0, 10.0), &g.models, &ctx.assets));
        assert!(g.editor.dirty());
    }

    #[test]
    fn f1_races_whole_draft_unsaved_and_resets_best_lap_only_after_edit() {
        let mut ctx = Context::default();
        let mut g = editing("f1-draft", &mut ctx);
        let file = fs::read_to_string(&g.path).expect("файл");
        g.race.best = Some(500);
        g.ghost.best = Some(still_lap("race", Mat4::IDENTITY));
        tap(&mut g, &mut ctx, KeyCode::F1);
        assert_eq!((g.mode, g.race.best), (Mode::Race, Some(500)));
        tap(&mut g, &mut ctx, KeyCode::F1);
        add_cone(&mut g, &mut ctx);
        g.race.ticks = 77;
        tap(&mut g, &mut ctx, KeyCode::F1);
        assert_eq!(g.mode, Mode::Race);
        assert_eq!((g.race.best, g.ghost.best.is_none()), (None, true));
        assert_eq!(g.track, g.editor.draft);
        assert_eq!(g.layout.draws, g.editor.layout.draws);
        assert!(placed_at(&g, g.start));
        assert_eq!(g.race.ticks, 0);
        assert!(g.editor.dirty());
        assert_eq!(fs::read_to_string(&g.path).expect("файл"), file);
    }

    #[test]
    fn f1_with_broken_chain_stays_in_editor_naming_piece() {
        let mut ctx = Context::default();
        let mut g = editing("f1-broken", &mut ctx);
        g.panel
            .queue
            .push(PanelCmd::Edit(EditorCmd::Tool(Tool::Erase)));
        tick(&mut g, &mut ctx);
        // roadStraight [4, 0] — третья деталь овала.
        assert!(g.editor.click(down(18.0, 2.0), &g.models, &ctx.assets));
        let track = g.track.clone();
        tap(&mut g, &mut ctx, KeyCode::F1);
        assert_eq!(g.mode, Mode::Editor);
        assert_eq!(g.track, track);
        let entry = g.editor.chain.as_ref().err().and_then(DraftError::entry);
        let Some(Entry::Road(i)) = entry else {
            panic!("{:?}", g.editor.chain);
        };
        let status = editor_panel::chain_status(&g.editor.chain);
        assert!(
            status.starts_with(&format!("деталь #{}: ", i + 1)),
            "{status}"
        );
        g.draw(&mut ctx, 1.0);
        let scene = ctx.gfx.scene.take().expect("сцена");
        assert_eq!(scene.models, g.editor.draws());
        assert!(scene.models.iter().any(|m| m.tint == editor::BLOCKED));
    }

    /// Курсор на пиксель, где показанный 3D-кадр видит точку `p`.
    fn point_at(ctx: &mut Context, p: Vec3) {
        let scene = ctx.shown.and_then(|s| s.scene).expect("3D-кадр");
        let size = scene.size.as_vec2();
        let ndc = (scene.camera.view_proj(size.x / size.y)).project_point3(p);
        let px = vec2(ndc.x + 1.0, 1.0 - ndc.y) * size / 2.0;
        assert!(
            px.cmpge(Vec2::ZERO).all() && px.cmplt(size).all(),
            "{p} вне окна: {px}"
        );
        ctx.input.on_cursor(Some(px));
    }

    #[test]
    fn oval_built_by_editor_from_scratch_laps_on_autopilot_unsaved() {
        let mut ctx = Context::default();
        let mut g = editing("scratch", &mut ctx);
        g.panel.queue.push(PanelCmd::Switch(Switch::New));
        tick(&mut g, &mut ctx);
        assert_eq!(g.editor.draft.roads, []);
        let file = fs::read_to_string(&g.path).expect("новый файл");
        let pan = [KeyCode::KeyD, KeyCode::KeyS];
        for key in pan {
            ctx.input.on_key(key, Pressed, false);
        }
        let panned = (0..600).any(|_| {
            tick(&mut g, &mut ctx);
            g.editor.orbit.focus.x >= 12.0
        });
        assert!(panned, "камера не доехала до овала");
        for key in pan {
            ctx.input.on_key(key, Released, false);
        }
        let oval = TrackDraft::parse(&fs::read_to_string(OVAL).expect("овал")).expect("разбор");
        for road in &oval.roads {
            let tool = Tool::Road {
                piece: road.piece,
                rot: road.rot,
            };
            (g.panel.queue).extend([
                PanelCmd::Edit(EditorCmd::Tool(tool)),
                PanelCmd::Edit(EditorCmd::Level(road.level)),
            ]);
            tick(&mut g, &mut ctx);
            show_editor(&g, &mut ctx);
            let mid = (road.cell.as_vec2() + 0.5) * KIT_SCALE;
            point_at(
                &mut ctx,
                vec3(mid.x, road.level as f32 * LEVEL * KIT_SCALE, mid.y),
            );
            ctx.input.on_button(MouseButton::Left, Pressed);
            tick(&mut g, &mut ctx);
            ctx.input.on_button(MouseButton::Left, Released);
        }
        assert_eq!(g.editor.draft.roads, oval.roads);
        tap(&mut g, &mut ctx, KeyCode::F1);
        assert_eq!(g.mode, Mode::Race);
        assert_eq!(fs::read_to_string(&g.path).expect("файл"), file);
        autopilot::lap_on(&mut g, "овал редактора", autopilot::reference("track01"));
    }

    fn ctrl(g: &mut Racing, ctx: &mut Context, key: KeyCode) {
        ctx.input.on_key(KeyCode::ControlLeft, Pressed, false);
        tap(g, ctx, key);
        ctx.input.on_key(KeyCode::ControlLeft, Released, false);
    }

    #[test]
    fn ctrl_s_writes_draft_and_tells_watcher_ctrl_z_undoes() {
        let mut ctx = Context::default();
        let mut g = editing("ctrl-s", &mut ctx);
        let oval = g.editor.draft.clone();
        add_cone(&mut g, &mut ctx);
        tap(&mut g, &mut ctx, KeyCode::KeyS);
        assert!(g.editor.dirty(), "S без Ctrl");
        ctrl(&mut g, &mut ctx, KeyCode::KeyS);
        assert!(!g.editor.dirty());
        let text = fs::read_to_string(&g.path).expect("файл");
        assert_eq!(TrackDraft::parse(&text), Ok(g.editor.draft.clone()));
        assert_eq!(
            g.watch.poll(&g.path),
            Ok(None),
            "своё сохранение — не изменение"
        );
        assert_eq!(g.panel.error, None);
        ctrl(&mut g, &mut ctx, KeyCode::KeyZ);
        assert_eq!(g.editor.draft, oval);

        g.path = g.dirs.tracks.join("нет/track.toml");
        ctrl(&mut g, &mut ctx, KeyCode::KeyS);
        let error = g.panel.error.clone().expect("ошибка");
        assert!(error.starts_with(&g.path.display().to_string()), "{error}");
        assert!(g.editor.dirty());
        g.panel.queue.push(PanelCmd::LoadFromDisk);
        tick(&mut g, &mut ctx);
        assert!(g.panel.error.is_some() && g.editor.dirty());
    }

    #[test]
    fn new_creates_next_free_track_with_garage_car() {
        let mut ctx = Context::default();
        let mut g = editing("new", &mut ctx);
        let oval = g.track.clone();
        g.garage.selected = car_index(&g, "sedan-sports");
        g.panel.queue.push(PanelCmd::Switch(Switch::New));
        tick(&mut g, &mut ctx);
        let path = g.dirs.tracks.join("track04.toml");
        assert_eq!(g.path, path);
        let empty = TrackDraft {
            car: "sedan-sports".into(),
            roads: Vec::new(),
            decor: Vec::new(),
        };
        let text = fs::read_to_string(&path).expect("файл");
        assert_eq!(TrackDraft::parse(&text), Ok(empty.clone()));
        assert_eq!(g.editor.draft, empty);
        assert!(!g.editor.dirty());
        let names = ["track01.toml", "track03.toml", "track04.toml"];
        assert_eq!(g.panel.tracks, names.map(|n| g.dirs.tracks.join(n)));
        tap(&mut g, &mut ctx, KeyCode::F1);
        assert_eq!(g.mode, Mode::Editor, "без roadStart");
        assert_eq!(g.track, oval);

        fs::remove_dir_all(&g.dirs.tracks).expect("удаление");
        g.panel.queue.push(PanelCmd::Switch(Switch::New));
        tick(&mut g, &mut ctx);
        assert_eq!(g.path, path);
        let error = g.panel.error.clone().expect("ошибка");
        assert!(
            error.starts_with(&g.dirs.tracks.display().to_string()),
            "{error}"
        );
    }

    #[test]
    fn new_and_open_wait_for_confirmation_while_dirty() {
        let mut ctx = Context::default();
        let mut g = editing("confirm", &mut ctx);
        let first = g.path.clone();
        add_cone(&mut g, &mut ctx);
        let draft = g.editor.draft.clone();
        g.panel.queue.push(PanelCmd::Switch(Switch::New));
        tick(&mut g, &mut ctx);
        assert_eq!(g.panel.pending, Some(Switch::New));
        assert_eq!(g.editor.draft, draft);
        assert!(!g.dirs.tracks.join("track04.toml").exists());
        g.panel.queue.push(PanelCmd::Keep);
        tick(&mut g, &mut ctx);
        assert_eq!((&g.panel.pending, &g.editor.draft), (&None, &draft));

        let eight = g.dirs.tracks.join("track03.toml");
        g.panel
            .queue
            .push(PanelCmd::Switch(Switch::Open(eight.clone())));
        tick(&mut g, &mut ctx);
        assert_eq!(g.panel.pending, Some(Switch::Open(eight.clone())));
        assert_eq!(g.path, first);
        g.panel.queue.push(PanelCmd::Discard);
        tick(&mut g, &mut ctx);
        let want = TrackDraft::parse(EIGHT).expect("восьмёрка");
        assert_eq!((&g.path, &g.panel.pending), (&eight, &None));
        assert_eq!((&g.editor.draft, &g.track), (&want, &want));
        assert!(!g.editor.dirty());

        g.panel.queue.push(PanelCmd::Discard);
        g.panel
            .queue
            .push(PanelCmd::Switch(Switch::Open(first.clone())));
        tick(&mut g, &mut ctx);
        assert_eq!((&g.path, &g.panel.pending), (&first, &None));
        let missing = g.dirs.tracks.join("track09.toml");
        g.panel
            .queue
            .push(PanelCmd::Switch(Switch::Open(missing.clone())));
        tick(&mut g, &mut ctx);
        assert_eq!(g.path, first);
        let error = g.panel.error.clone().expect("ошибка");
        assert!(error.starts_with(&missing.display().to_string()), "{error}");
    }

    #[test]
    fn external_change_keeps_dirty_draft_and_replaces_clean_one() {
        let mut ctx = Context::default();
        let mut g = editing("external", &mut ctx);
        add_cone(&mut g, &mut ctx);
        let draft = g.editor.draft.clone();
        fs::write(&g.path, EIGHT).expect("запись");
        ticks(&mut g, &mut ctx, engine::WATCH_TICKS);
        assert_eq!(g.editor.draft, draft);
        assert!(g.panel.changed_on_disk);
        g.panel.queue.push(PanelCmd::LoadFromDisk);
        tick(&mut g, &mut ctx);
        let eight = TrackDraft::parse(EIGHT).expect("восьмёрка");
        assert_eq!((&g.editor.draft, &g.track), (&eight, &eight));
        assert!(!g.editor.dirty() && !g.panel.changed_on_disk);
        assert_eq!(g.panel.error, None);
        ticks(&mut g, &mut ctx, engine::WATCH_TICKS);
        assert_eq!(g.editor.draft, eight, "загруженный текст — не изменение");

        let oval = fs::read_to_string(OVAL).expect("овал");
        fs::write(&g.path, &oval).expect("запись");
        ticks(&mut g, &mut ctx, engine::WATCH_TICKS);
        let want = TrackDraft::parse(&oval).expect("овал");
        assert_eq!((&g.editor.draft, &g.track), (&want, &want));
        assert!(!g.panel.changed_on_disk);

        fs::write(&g.path, "car = \"race\"\n").expect("запись");
        ticks(&mut g, &mut ctx, engine::WATCH_TICKS);
        assert_eq!(g.editor.draft.roads, []);
        assert_eq!(g.track, want, "заезд — по прежней цельной");
        assert!(g.error.is_some());
    }

    #[test]
    fn editor_panel_shown_only_in_editor() {
        let mut ctx = Context::default();
        let mut g = racing(&mut ctx);
        g.panel.tracks = vec![g.path.clone()];
        let mut h = harness(g);
        assert!(h.query_by_label("Редактор").is_none());
        h.state_mut().mode = Mode::Editor;
        h.run();
        assert!(h.query_by_label("Редактор").is_some());
        h.get_by_label("стереть").click();
        h.run();
        let erase = PanelCmd::Edit(EditorCmd::Tool(Tool::Erase));
        assert_eq!(h.state().panel.queue, [erase]);
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
