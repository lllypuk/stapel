use std::collections::VecDeque;

use std::f32::consts::FRAC_PI_2;

use engine::{
    Aabb3, Assets, Camera3D, Color, IVec2, Input, KeyCode, Mat4, ModelDraw, Quat, Ray, Vec2, Vec3,
    Vertex, vec2, vec3,
};

use crate::layout::{Layout, Models, place};
use crate::pieces::{DecorPiece, KIT_SCALE, LEVEL, PIECES, Piece, Rot};
use crate::track::{Checkpoint, DecorEntry, DraftError, Entry, MAX_LEVEL, RoadEntry, TrackDraft};

/// Снимков отмены не больше.
pub(crate) const UNDO_LIMIT: usize = 100;
/// Шаг поворота декора, градусы.
pub(crate) const YAW_STEP: f32 = 15.0;
/// Дальше луч по миру не ищет, м.
const PICK_RANGE: f32 = 1000.0;
/// Наклон взгляда камеры вниз.
const PITCH: f32 = 0.95;
/// Расстояние камеры до точки орбиты — `NEAREST · ZOOM_STEP^zoom`, м.
const NEAREST: f32 = 8.0;
const ZOOM_STEP: f32 = 1.25;
pub(crate) const MAX_ZOOM: i32 = 12;
/// Сдвиг за тик в долях расстояния до точки орбиты.
const PAN: f32 = 0.02;
const FAR: f32 = 600.0;
/// Квад клетки над плоскостью уровня, м: иначе он мерцает с дорогой.
const QUAD_LIFT: f32 = 0.05;
pub(crate) const PLACEABLE: Color = Color {
    r: 0.3,
    g: 1.0,
    b: 0.3,
    a: 0.6,
};
pub(crate) const BLOCKED: Color = Color {
    r: 1.0,
    g: 0.25,
    b: 0.2,
    a: 0.6,
};

/// Что делает клик; `yaw` — в градусах, как в файле.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Tool {
    Road {
        piece: &'static Piece,
        rot: Rot,
    },
    Decor {
        piece: &'static DecorPiece,
        yaw: f32,
    },
    Erase,
}

impl Tool {
    /// На `step` шагов: дорога — по 90°, декор — по `YAW_STEP`.
    pub(crate) fn turned(self, step: i8) -> Self {
        match self {
            Self::Road { piece, rot } => Self::Road {
                piece,
                rot: Rot::from_degrees(rot.degrees() + 90 * i64::from(step)).expect("кратен 90"),
            },
            Self::Decor { piece, yaw } => Self::Decor {
                piece,
                yaw: (yaw + YAW_STEP * f32::from(step)).rem_euclid(360.0),
            },
            Self::Erase => self,
        }
    }
}

/// Команда панели редактора; разбирает следующий `update`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EditorCmd {
    Tool(Tool),
    Level(u32),
    Undo,
}

/// Камера редактора — сверху наискось на точку плоскости уровня.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Orbit {
    /// Точка орбиты по X и Z, м.
    pub(crate) focus: Vec2,
    prev: Vec2,
    /// Четверти оборота против часовой сверху; 0 — взгляд по −Z.
    pub(crate) turn: u8,
    pub(crate) zoom: i32,
    /// Строки колеса, ещё не ставшие шагом зума.
    wheel: f32,
}

impl Orbit {
    fn new(focus: Vec2) -> Self {
        Self {
            focus,
            prev: focus,
            turn: 0,
            zoom: MAX_ZOOM / 2,
            wheel: 0.0,
        }
    }

    /// Взгляд по горизонтали.
    fn heading(&self) -> Vec2 {
        let v = Quat::from_rotation_y(f32::from(self.turn) * FRAC_PI_2) * Vec3::NEG_Z;
        vec2(v.x, v.z)
    }

    pub(crate) fn distance(&self) -> f32 {
        NEAREST * ZOOM_STEP.powi(self.zoom)
    }

    /// Колесо от себя — ближе.
    pub(crate) fn scroll(&mut self, lines: f32) {
        self.wheel += lines;
        let steps = self.wheel.trunc();
        self.wheel -= steps;
        #[expect(clippy::cast_possible_truncation, reason = "строк за тик единицы")]
        let steps = steps as i32;
        self.zoom = (self.zoom - steps).clamp(0, MAX_ZOOM);
    }

    fn tick(&mut self, input: &Input) {
        self.prev = self.focus;
        if input.just_pressed(KeyCode::KeyQ) {
            self.turn = (self.turn + 1) % 4;
        }
        if input.just_pressed(KeyCode::KeyE) {
            self.turn = (self.turn + 3) % 4;
        }
        let forward = self.heading();
        let right = vec2(-forward.y, forward.x);
        let axis = |plus, minus| f32::from(input.is_down(plus)) - f32::from(input.is_down(minus));
        let dir = forward * axis(KeyCode::KeyW, KeyCode::KeyS)
            + right * axis(KeyCode::KeyD, KeyCode::KeyA);
        self.focus += dir * PAN * self.distance();
        self.scroll(input.wheel().y);
    }

    /// Глаз между тиками; точка орбиты на высоте `floor`.
    fn camera(&self, alpha: f32, floor: f32) -> Camera3D {
        let focus = self.prev.lerp(self.focus, alpha);
        let target = vec3(focus.x, floor, focus.y);
        let back = -self.heading() * PITCH.cos();
        let eye = target + vec3(back.x, PITCH.sin(), back.y) * self.distance();
        let mut camera = Camera3D {
            far: FAR,
            ..Camera3D::default()
        };
        camera.look_at(eye, target);
        camera
    }
}

/// Квад клетки 1×1 в плоскости XZ от начала, нормаль вверх.
pub(crate) fn quad() -> (Vec<Vertex>, Vec<u32>) {
    let vertices = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]
        .map(|[x, z]| Vertex {
            pos: [x, 0.0, z],
            normal: [0.0, 1.0, 0.0],
            uv: [x, z],
            ..Vertex::default()
        })
        .to_vec();
    (vertices, vec![0, 1, 2, 0, 2, 3])
}

/// Правка черновика трассы инструментом на текущем уровне.
pub(crate) struct Editor {
    pub(crate) draft: TrackDraft,
    /// Перестраивается после каждой правки и отмены: по нему идёт пикинг.
    pub(crate) layout: Layout,
    /// Цепочка черновика; пересчитывается вместе с `layout`.
    pub(crate) chain: Result<Vec<Checkpoint>, DraftError>,
    pub(crate) tool: Tool,
    pub(crate) level: u32,
    pub(crate) orbit: Orbit,
    undo: VecDeque<TrackDraft>,
    saved: TrackDraft,
}

/// Расстояние вдоль луча до плоскости высоты `y`; луч вдоль неё или от неё — `None`.
fn plane_t(ray: Ray, y: f32) -> Option<f32> {
    let t = (y - ray.origin.y) / ray.dir.y;
    (t.is_finite() && t >= 0.0).then_some(t)
}

/// Расстояние вдоль луча до входа в `b`; из начала внутри — 0.
fn enter_t(ray: Ray, b: Aabb3) -> Option<f32> {
    let inv = ray.dir.recip();
    let (a, c) = ((b.min - ray.origin) * inv, (b.max - ray.origin) * inv);
    let near = a.min(c).max_element().max(0.0);
    (near <= a.max(c).min_element()).then_some(near)
}

fn at(ray: Ray, t: f32) -> Vec3 {
    ray.origin + ray.dir * t
}

/// Черновик одной детали превью.
fn piece_draft(roads: Vec<RoadEntry>, decor: Vec<DecorEntry>) -> TrackDraft {
    TrackDraft {
        car: String::new(),
        roads,
        decor,
    }
}

/// Клетка над или под точкой.
fn cell(p: Vec3) -> IVec2 {
    (vec2(p.x, p.z) / KIT_SCALE).floor().as_ivec2()
}

impl Editor {
    /// Чистый редактор черновика `draft` с диска; камера — над серединой трассы.
    pub(crate) fn new(draft: TrackDraft, models: &Models, assets: &Assets) -> Self {
        let layout = Layout::new(&draft, models, assets);
        let all = (layout.bounds.iter().map(|(_, b)| *b)).reduce(|a, b| Aabb3 {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
        });
        let mid = all.map_or(Vec3::ZERO, |b| (b.min + b.max) / 2.0);
        Self {
            layout,
            chain: draft.chain(),
            orbit: Orbit::new(vec2(mid.x, mid.z)),
            tool: Tool::Road {
                piece: &PIECES[0],
                rot: Rot::default(),
            },
            level: 0,
            undo: VecDeque::new(),
            saved: draft.clone(),
            draft,
        }
    }

    pub(crate) fn dirty(&self) -> bool {
        self.draft != self.saved
    }

    /// Черновик записан в файл.
    pub(crate) fn saved(&mut self) {
        self.saved = self.draft.clone();
    }

    /// Черновик с диска вместо текущего, чистый и без отмены; камера, инструмент и уровень прежние.
    pub(crate) fn replace(&mut self, draft: TrackDraft, models: &Models, assets: &Assets) {
        self.saved = draft.clone();
        self.draft = draft;
        self.undo.clear();
        self.rebuild(models, assets);
    }

    fn rebuild(&mut self, models: &Models, assets: &Assets) {
        self.layout = Layout::new(&self.draft, models, assets);
        self.chain = self.draft.chain();
    }

    /// Модели черновика; деталь, на которой рвётся цепочка, — `BLOCKED`.
    pub(crate) fn draws(&self) -> Vec<ModelDraw> {
        let broken = self.chain.as_ref().err().and_then(DraftError::entry);
        (self.layout.draws.iter().zip(&self.layout.bounds))
            .map(|(d, &(entry, _))| ModelDraw {
                tint: if Some(entry) == broken {
                    BLOCKED
                } else {
                    d.tint
                },
                ..d.clone()
            })
            .collect()
    }

    pub(crate) fn apply(&mut self, cmd: EditorCmd, models: &Models, assets: &Assets) {
        match cmd {
            EditorCmd::Tool(tool) => self.tool = tool,
            EditorCmd::Level(level) => self.level = level.min(MAX_LEVEL),
            EditorCmd::Undo => {
                if let Some(before) = self.undo.pop_back() {
                    self.draft = before;
                    self.rebuild(models, assets);
                }
            }
        }
    }

    /// Высота плоскости текущего уровня, м.
    fn floor(&self) -> f32 {
        self.level as f32 * LEVEL * KIT_SCALE
    }

    /// Камера и уровень с клавиатуры и колеса.
    pub(crate) fn control(&mut self, input: &Input) {
        self.orbit.tick(input);
        if input.just_pressed(KeyCode::PageUp) {
            self.level = (self.level + 1).min(MAX_LEVEL);
        }
        if input.just_pressed(KeyCode::PageDown) {
            self.level = self.level.saturating_sub(1);
        }
    }

    pub(crate) fn camera(&self, alpha: f32) -> Camera3D {
        self.orbit.camera(alpha, self.floor())
    }

    /// Деталь инструмента под лучом: `PLACEABLE`, если её можно поставить, иначе `BLOCKED`.
    pub(crate) fn preview(&self, ray: Ray, models: &Models) -> Vec<ModelDraw> {
        let (track, ok) = match self.tool {
            Tool::Road { piece, rot } => {
                let Some(road) = self.road_at(ray, piece, rot) else {
                    return Vec::new();
                };
                let ok = self.draft.can_place(&road).is_ok();
                (piece_draft(vec![road], Vec::new()), ok)
            }
            Tool::Decor { piece, yaw } => {
                let Some(decor) = self.decor_at(ray, piece, yaw) else {
                    return Vec::new();
                };
                (piece_draft(Vec::new(), vec![decor]), decor.in_bounds())
            }
            Tool::Erase => return Vec::new(),
        };
        let tint = if ok { PLACEABLE } else { BLOCKED };
        (place(&track).into_iter())
            .map(|p| ModelDraw {
                model: models.get(p.name),
                transform: p.transform,
                pose: None,
                tint,
            })
            .collect()
    }

    /// Квад `quad` → мир: клетка под лучом на плоскости уровня.
    pub(crate) fn cell_quad(&self, ray: Ray) -> Option<Mat4> {
        let floor = self.floor();
        let c = cell(at(ray, plane_t(ray, floor)?)).as_vec2() * KIT_SCALE;
        Some(
            Mat4::from_translation(vec3(c.x, floor + QUAD_LIFT, c.y))
                * Mat4::from_scale(vec3(KIT_SCALE, 1.0, KIT_SCALE)),
        )
    }

    /// Деталь с якорем в клетке под лучом на плоскости уровня; занятость не проверяет.
    pub(crate) fn road_at(&self, ray: Ray, piece: &'static Piece, rot: Rot) -> Option<RoadEntry> {
        let p = at(ray, plane_t(ray, self.floor())?);
        Some(RoadEntry {
            piece,
            cell: cell(p),
            level: self.level,
            rot,
        })
    }

    /// Декор в точке попадания по миру, мимо — на плоскости уровня; пределы не проверяет.
    pub(crate) fn decor_at(
        &self,
        ray: Ray,
        piece: &'static DecorPiece,
        yaw: f32,
    ) -> Option<DecorEntry> {
        let hit = (self.layout.world).raycast(ray.origin, ray.dir, PICK_RANGE, |_| true);
        let pos = match hit {
            Some(h) => h.point,
            None => at(ray, plane_t(ray, self.floor())?),
        };
        Some(DecorEntry {
            piece,
            pos,
            yaw_deg: yaw,
        })
    }

    /// Ближайшее по лучу: габарит декора или дорога в клетке под лучом на плоскости уровня.
    pub(crate) fn erase_at(&self, ray: Ray) -> Option<Entry> {
        let decor = (self.layout.bounds.iter())
            .filter(|(e, _)| matches!(e, Entry::Decor(_)))
            .filter_map(|&(e, b)| Some((enter_t(ray, b)?, e)));
        let road = plane_t(ray, self.floor()).and_then(|t| {
            let &i = (self.draft.occupancy()).get(&(cell(at(ray, t)), self.level))?;
            Some((t, Entry::Road(i)))
        });
        (decor.chain(road))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, e)| e)
    }

    /// Клик инструментом по лучу из курсора; `false` — черновик не изменился.
    pub(crate) fn click(&mut self, ray: Ray, models: &Models, assets: &Assets) -> bool {
        let before = self.draft.clone();
        match self.tool {
            Tool::Road { piece, rot } => {
                let Some(road) = self.road_at(ray, piece, rot) else {
                    return false;
                };
                if self.draft.can_place(&road).is_err() {
                    return false;
                }
                self.draft.roads.push(road);
            }
            Tool::Decor { piece, yaw } => {
                let Some(decor) = self.decor_at(ray, piece, yaw).filter(DecorEntry::in_bounds)
                else {
                    return false;
                };
                self.draft.decor.push(decor);
            }
            Tool::Erase => match self.erase_at(ray) {
                Some(Entry::Road(i)) => {
                    self.draft.roads.remove(i);
                }
                Some(Entry::Decor(i)) => {
                    self.draft.decor.remove(i);
                }
                None => return false,
            },
        }
        if self.undo.len() == UNDO_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(before);
        self.rebuild(models, assets);
        true
    }
}

#[cfg(test)]
mod tests {
    use engine::ElementState::{Pressed, Released};
    use engine::{ivec2, vec3};

    use super::*;
    use crate::pieces::{decor_piece, piece};

    fn loaded() -> (Assets, Models) {
        let mut assets = Assets::default();
        let models = Models::load(&mut assets);
        (assets, models)
    }

    fn draft(roads: Vec<RoadEntry>, decor: Vec<DecorEntry>) -> TrackDraft {
        TrackDraft {
            car: "race".into(),
            roads,
            decor,
        }
    }

    fn road(name: &str, cell: IVec2, level: u32, rot: Rot) -> RoadEntry {
        RoadEntry {
            piece: piece(name).expect("деталь"),
            cell,
            level,
            rot,
        }
    }

    fn decor(name: &str, pos: Vec3) -> DecorEntry {
        DecorEntry {
            piece: decor_piece(name).expect("декор"),
            pos,
            yaw_deg: 0.0,
        }
    }

    /// Отвесно вниз над точкой `x, z` плоскости.
    fn down(x: f32, z: f32) -> Ray {
        Ray {
            origin: vec3(x, 30.0, z),
            dir: Vec3::NEG_Y,
        }
    }

    /// Отвесно вниз над серединой клетки.
    fn down_cell(cell: IVec2) -> Ray {
        let c = (cell.as_vec2() + 0.5) * KIT_SCALE;
        down(c.x, c.y)
    }

    fn rot(deg: i64) -> Rot {
        Rot::from_degrees(deg).expect("rot")
    }

    fn road_tool(name: &str, deg: i64) -> Tool {
        Tool::Road {
            piece: piece(name).expect("деталь"),
            rot: rot(deg),
        }
    }

    fn decor_tool(name: &str, yaw: f32) -> Tool {
        Tool::Decor {
            piece: decor_piece(name).expect("декор"),
            yaw,
        }
    }

    #[test]
    fn road_goes_to_cell_under_ray_on_level_plane_and_not_onto_taken() {
        let (assets, models) = loaded();
        let mut e = Editor::new(draft(Vec::new(), Vec::new()), &models, &assets);
        e.apply(EditorCmd::Level(1), &models, &assets);
        e.apply(
            EditorCmd::Tool(road_tool("roadStraightLong", 90)),
            &models,
            &assets,
        );
        // Наискось: на высоте 2 м уровня 1 луч в клетке (2, 1), ниже — уже в (3, 1).
        let slant = Ray {
            origin: vec3(0.5, 7.0, 6.0),
            dir: vec3(2.0, -1.0, 0.0).normalize(),
        };
        assert!(e.click(slant, &models, &assets));
        let placed = road("roadStraightLong", ivec2(2, 1), 1, rot(90));
        assert_eq!(e.draft.roads, [placed]);
        assert!(!e.click(slant, &models, &assets));
        let up = Ray {
            dir: Vec3::Y,
            ..down_cell(ivec2(5, 5))
        };
        assert!(!e.click(up, &models, &assets));
        assert_eq!(e.draft.roads, [placed]);
    }

    #[test]
    fn erase_on_any_cell_removes_whole_multi_cell_piece() {
        let (assets, models) = loaded();
        let long = road("roadStraightLong", ivec2(2, 1), 1, rot(90));
        let other = road("roadStraight", ivec2(-3, 0), 1, rot(0));
        let mut e = Editor::new(draft(vec![other, long], Vec::new()), &models, &assets);
        e.apply(EditorCmd::Tool(Tool::Erase), &models, &assets);
        let (far, _) = (long.cells())
            .find(|&(c, _)| c != long.cell)
            .expect("вторая клетка");
        assert!(!e.click(down_cell(far), &models, &assets), "уровень 0 пуст");
        e.apply(EditorCmd::Level(1), &models, &assets);
        assert!(e.click(down_cell(far), &models, &assets));
        assert_eq!(e.draft.roads, [other]);
        assert!(!e.click(down_cell(ivec2(9, 9)), &models, &assets));
    }

    #[test]
    fn decor_lands_on_world_hit_else_on_plane_and_not_out_of_bounds() {
        let (assets, models) = loaded();
        let deck = road("roadStraight", IVec2::ZERO, 1, rot(0));
        let mut e = Editor::new(draft(vec![deck], Vec::new()), &models, &assets);
        e.apply(
            EditorCmd::Tool(decor_tool("treeLarge", 30.0)),
            &models,
            &assets,
        );
        assert!(e.click(down(2.0, 2.0), &models, &assets));
        let on_deck = e.draft.decor[0].pos;
        assert!((on_deck.y - 2.0).abs() < 0.5, "{on_deck}");
        assert_eq!(
            (on_deck.x, on_deck.z, e.draft.decor[0].yaw_deg),
            (2.0, 2.0, 30.0)
        );
        assert!(e.click(down(10.0, 2.0), &models, &assets));
        assert_eq!(e.draft.decor[1].pos, vec3(10.0, 0.0, 2.0));
        assert!(!e.click(down(5000.0, 2.0), &models, &assets));
        assert_eq!(e.draft.decor.len(), 2);
    }

    #[test]
    fn erase_removes_nearest_decor_box_before_road_behind_it() {
        let (assets, models) = loaded();
        let ground = road("roadStraight", IVec2::ZERO, 0, rot(0));
        let (far, near) = (
            decor("treeLarge", vec3(20.0, 0.0, 10.0)),
            decor("treeLarge", vec3(10.0, 0.0, 10.0)),
        );
        let cone = decor("cone", vec3(2.0, 0.0, 2.0));
        let mut e = Editor::new(draft(vec![ground], vec![far, near, cone]), &models, &assets);
        e.apply(EditorCmd::Tool(Tool::Erase), &models, &assets);
        let along = Ray {
            origin: vec3(-10.0, 1.0, 10.0),
            dir: Vec3::X,
        };
        assert!(e.click(along, &models, &assets));
        assert_eq!(e.draft.decor, [far, cone]);
        assert!(e.click(down(2.0, 2.0), &models, &assets));
        assert_eq!(
            (e.draft.decor.as_slice(), e.draft.roads.len()),
            ([far].as_slice(), 1)
        );
        assert!(e.click(down(3.5, 3.5), &models, &assets));
        assert_eq!(e.draft.roads, []);
    }

    #[test]
    fn undo_steps_back_up_to_limit_and_dirty_tracks_saved() {
        let (assets, models) = loaded();
        let mut e = Editor::new(draft(Vec::new(), Vec::new()), &models, &assets);
        assert!(!e.dirty());
        e.apply(EditorCmd::Undo, &models, &assets);
        assert_eq!(e.draft, draft(Vec::new(), Vec::new()));
        e.apply(EditorCmd::Tool(decor_tool("cone", 0.0)), &models, &assets);
        for i in 0..=UNDO_LIMIT {
            assert!(e.click(down(i as f32, 0.0), &models, &assets));
        }
        assert!(e.dirty());
        e.saved();
        assert!(!e.dirty());
        e.apply(EditorCmd::Undo, &models, &assets);
        assert_eq!(e.draft.decor.len(), UNDO_LIMIT);
        assert!(e.dirty());
        for _ in 0..UNDO_LIMIT {
            e.apply(EditorCmd::Undo, &models, &assets);
        }
        assert_eq!(e.draft.decor, [decor("cone", Vec3::ZERO)]);
        assert_eq!(e.layout.draws.len(), 1);
    }

    #[test]
    fn picking_follows_layout_after_edit_and_undo() {
        let (assets, models) = loaded();
        let mut e = Editor::new(draft(Vec::new(), Vec::new()), &models, &assets);
        e.apply(EditorCmd::Level(1), &models, &assets);
        e.apply(
            EditorCmd::Tool(road_tool("roadStraight", 0)),
            &models,
            &assets,
        );
        assert!(e.click(down_cell(IVec2::ZERO), &models, &assets));
        e.apply(EditorCmd::Level(0), &models, &assets);
        let tree = decor_piece("treeLarge").expect("декор");
        let on = |e: &Editor| e.decor_at(down(2.0, 2.0), tree, 0.0).expect("декор").pos.y;
        assert!(on(&e) > 1.5, "{}", on(&e));
        e.apply(EditorCmd::Undo, &models, &assets);
        assert_eq!(on(&e), 0.0);
    }

    #[test]
    fn level_is_capped() {
        let (assets, models) = loaded();
        let mut e = Editor::new(draft(Vec::new(), Vec::new()), &models, &assets);
        e.apply(EditorCmd::Level(MAX_LEVEL + 5), &models, &assets);
        assert_eq!(e.level, MAX_LEVEL);
    }

    fn empty() -> (Assets, Models, Editor) {
        let (assets, models) = loaded();
        let e = Editor::new(draft(Vec::new(), Vec::new()), &models, &assets);
        (assets, models, e)
    }

    /// Тик с клавишей, нажатой в нём и отпущенной после.
    fn tap(e: &mut Editor, key: KeyCode) {
        let mut input = Input::default();
        input.on_key(key, Pressed, false);
        e.control(&input);
        input.on_key(key, Released, false);
    }

    /// Точка орбиты в NDC камеры кадра.
    fn on_screen(e: &Editor, p: Vec2) -> Vec3 {
        let p = vec3(p.x, e.floor(), p.y);
        e.camera(1.0).view_proj(1.0).project_point3(p)
    }

    #[test]
    fn camera_looks_at_track_middle_on_level_plane() {
        let (assets, models) = loaded();
        let a = road("roadStraight", ivec2(2, 2), 0, rot(0));
        let b = road("roadStraight", ivec2(5, 7), 0, rot(0));
        let mut e = Editor::new(draft(vec![a, b], Vec::new()), &models, &assets);
        assert!(
            e.orbit.focus.abs_diff_eq(vec2(16.0, 20.0), 0.5),
            "{}",
            e.orbit.focus
        );
        tap(&mut e, KeyCode::PageUp);
        let ray = e.camera(1.0).ray(Vec2::ZERO, 1.0);
        let hit = at(ray, plane_t(ray, LEVEL * KIT_SCALE).expect("вниз"));
        assert!(hit.abs_diff_eq(vec3(16.0, 2.0, 20.0), 0.5), "{hit}");
        assert!(ray.origin.y > hit.y);
        let (_, _, empty) = empty();
        assert_eq!(empty.orbit.focus, Vec2::ZERO);
    }

    #[test]
    fn wasd_pans_relative_to_turn() {
        let (_, _, mut e) = empty();
        let step = PAN * e.orbit.distance();
        let mut input = Input::default();
        input.on_key(KeyCode::KeyW, Pressed, false);
        e.control(&input);
        assert!(e.orbit.focus.abs_diff_eq(vec2(0.0, -step), 1e-5));
        assert!(on_screen(&e, Vec2::ZERO).y < 0.0, "старая точка ушла вниз");
        input.on_key(KeyCode::KeyW, Released, false);
        input.on_key(KeyCode::KeyD, Pressed, false);
        let before = e.orbit.focus;
        e.control(&input);
        assert!(e.orbit.focus.abs_diff_eq(before + vec2(step, 0.0), 1e-5));
        assert!(on_screen(&e, before).x < 0.0, "старая точка ушла влево");
        input.on_key(KeyCode::KeyD, Released, false);
        // Взгляд по −X: назад — по +X, влево — по +Z.
        e.orbit.turn = 1;
        for (key, moved) in [
            (KeyCode::KeyS, vec2(step, 0.0)),
            (KeyCode::KeyA, vec2(0.0, step)),
        ] {
            input.on_key(key, Pressed, false);
            let before = e.orbit.focus;
            e.control(&input);
            input.on_key(key, Released, false);
            assert!(e.orbit.focus.abs_diff_eq(before + moved, 1e-5), "{key:?}");
        }
        let before = e.orbit.focus;
        e.control(&input);
        assert_eq!(e.orbit.focus, before);
    }

    #[test]
    fn q_and_e_turn_by_quarter_and_wrap() {
        let (_, _, mut e) = empty();
        let p = vec2(-10.0, 0.0);
        tap(&mut e, KeyCode::KeyQ);
        assert_eq!(e.orbit.turn, 1);
        let ndc = on_screen(&e, p);
        assert!(
            ndc.x.abs() < 1e-4 && ndc.y > 0.0,
            "точка слева теперь впереди: {ndc}"
        );
        for _ in 0..3 {
            tap(&mut e, KeyCode::KeyQ);
        }
        assert_eq!(e.orbit.turn, 0);
        tap(&mut e, KeyCode::KeyE);
        assert_eq!(e.orbit.turn, 3);
        let ndc = on_screen(&e, -p);
        assert!(
            ndc.x.abs() < 1e-4 && ndc.y > 0.0,
            "точка справа теперь впереди: {ndc}"
        );
        tap(&mut e, KeyCode::KeyE);
        assert_eq!(e.orbit.turn, 2);
    }

    #[test]
    fn wheel_zooms_within_limits_and_keeps_fraction() {
        let (_, _, mut e) = empty();
        let o = &mut e.orbit;
        let start = o.zoom;
        o.scroll(0.6);
        assert_eq!(o.zoom, start);
        o.scroll(-0.2);
        assert_eq!(o.zoom, start);
        o.scroll(0.6);
        assert_eq!(o.zoom, start - 1);
        let near = o.distance();
        o.scroll(-1.0);
        assert!(o.distance() > near);
        o.scroll(100.0);
        assert_eq!((o.zoom, o.distance()), (0, NEAREST));
        o.scroll(-100.0);
        assert_eq!(o.zoom, MAX_ZOOM);
        let depth = on_screen(&e, Vec2::ZERO).z;
        assert!(
            depth < 1.0,
            "на дальнем зуме точка орбиты не за дальней плоскостью: {depth}"
        );
        let eye = e.camera(1.0).pos;
        assert!((eye.length() - e.orbit.distance()).abs() < 1e-3, "{eye}");
    }

    #[test]
    fn page_keys_change_level_within_range() {
        let (_, _, mut e) = empty();
        tap(&mut e, KeyCode::PageDown);
        assert_eq!(e.level, 0);
        tap(&mut e, KeyCode::PageUp);
        assert_eq!(e.level, 1);
        for _ in 0..MAX_LEVEL + 3 {
            tap(&mut e, KeyCode::PageUp);
        }
        assert_eq!(e.level, MAX_LEVEL);
        tap(&mut e, KeyCode::PageDown);
        assert_eq!(e.level, MAX_LEVEL - 1);
    }

    #[test]
    fn camera_eases_focus_between_ticks() {
        let (_, _, mut e) = empty();
        let mut input = Input::default();
        input.on_key(KeyCode::KeyD, Pressed, false);
        e.control(&input);
        let (a, b) = (e.camera(0.0).pos, e.camera(1.0).pos);
        assert!((b - a).abs_diff_eq(vec3(PAN * e.orbit.distance(), 0.0, 0.0), 1e-4));
        assert!(e.camera(0.5).pos.abs_diff_eq(a.lerp(b, 0.5), 1e-4));
    }

    #[test]
    fn preview_is_green_where_piece_fits_and_red_where_not() {
        let (assets, models) = loaded();
        let taken = road("roadStraight", IVec2::ZERO, 0, rot(0));
        let mut e = Editor::new(draft(vec![taken], Vec::new()), &models, &assets);
        e.apply(
            EditorCmd::Tool(road_tool("roadRampLong", 90)),
            &models,
            &assets,
        );
        let free = road("roadRampLong", ivec2(4, 4), 0, rot(90));
        let tints = |e: &Editor, ray| -> Vec<Color> {
            e.preview(ray, &models).iter().map(|d| d.tint).collect()
        };
        let draws = e.preview(down_cell(ivec2(4, 4)), &models);
        let want = place(&piece_draft(vec![free], Vec::new()));
        assert!(want.len() > 1, "с барьерами");
        assert_eq!(draws.len(), want.len());
        assert_eq!(draws[0].model, models.get("roadRampLong"));
        assert_eq!(draws[0].transform, free.transform());
        assert!(
            draws
                .iter()
                .all(|d| d.tint == PLACEABLE && d.pose.is_none())
        );
        assert!(
            tints(&e, down_cell(IVec2::ZERO))
                .iter()
                .all(|&c| c == BLOCKED)
        );
        assert_ne!(tints(&e, down_cell(IVec2::ZERO)), []);
        let up = Ray {
            dir: Vec3::Y,
            ..down(0.0, 0.0)
        };
        assert_eq!(tints(&e, up), []);
        e.apply(EditorCmd::Tool(decor_tool("cone", 0.0)), &models, &assets);
        assert_eq!(tints(&e, down(30.0, 2.0)), [PLACEABLE]);
        assert_eq!(tints(&e, down(5000.0, 2.0)), [BLOCKED]);
        assert_eq!(tints(&e, up), []);
        e.apply(EditorCmd::Tool(Tool::Erase), &models, &assets);
        assert_eq!(tints(&e, down(2.0, 2.0)), []);
    }

    #[test]
    fn turned_steps_road_by_quarter_and_decor_by_yaw_step_with_wrap() {
        assert_eq!(
            road_tool("roadStraight", 270).turned(1),
            road_tool("roadStraight", 0)
        );
        assert_eq!(
            road_tool("roadStraight", 0).turned(-1),
            road_tool("roadStraight", 270)
        );
        assert_eq!(decor_tool("cone", 350.0).turned(1), decor_tool("cone", 5.0));
        assert_eq!(
            decor_tool("cone", 0.0).turned(-1),
            decor_tool("cone", 345.0)
        );
        assert_eq!(Tool::Erase.turned(1), Tool::Erase);
        assert_eq!(
            road_tool("roadStraight", 90).turned(2),
            road_tool("roadStraight", 270)
        );
        assert_eq!(decor_tool("cone", 0.0).turned(2), decor_tool("cone", 30.0));
    }

    #[test]
    fn replace_takes_clean_draft_without_undo_and_keeps_camera() {
        let (assets, models, mut e) = empty();
        e.apply(EditorCmd::Tool(decor_tool("cone", 0.0)), &models, &assets);
        assert!(e.click(down(1.0, 1.0), &models, &assets));
        e.orbit.turn = 2;
        let start = road("roadStart", IVec2::ZERO, 0, rot(0));
        let disk = draft(vec![start], Vec::new());
        e.replace(disk.clone(), &models, &assets);
        assert_eq!(e.draft, disk);
        assert!(!e.dirty());
        assert_eq!(e.chain, disk.chain());
        assert_eq!(e.layout.draws.len(), place(&disk).len());
        assert_eq!((e.orbit.turn, e.tool), (2, decor_tool("cone", 0.0)));
        e.apply(EditorCmd::Undo, &models, &assets);
        assert_eq!(e.draft, disk);
    }

    #[test]
    fn draws_tint_piece_breaking_chain() {
        let (assets, models) = loaded();
        let start = road("roadStart", IVec2::ZERO, 0, rot(0));
        let cone = decor("cone", vec3(20.0, 0.0, 20.0));
        let mut e = Editor::new(draft(vec![start], vec![cone]), &models, &assets);
        let Err(error) = &e.chain else {
            panic!("одна деталь — не круг");
        };
        assert_eq!(error.entry(), Some(Entry::Road(0)));
        let draws = e.draws();
        assert_eq!(draws.len(), e.layout.draws.len());
        for ((d, plain), (entry, _)) in draws.iter().zip(&e.layout.draws).zip(&e.layout.bounds) {
            let tint = if *entry == Entry::Road(0) {
                BLOCKED
            } else {
                Color::WHITE
            };
            assert_eq!(
                *d,
                ModelDraw {
                    tint,
                    ..plain.clone()
                }
            );
        }
        assert!(
            draws.iter().any(|d| d.tint == Color::WHITE),
            "декор не подсвечен"
        );
        e.replace(draft(Vec::new(), vec![cone]), &models, &assets);
        assert_eq!(e.chain.as_ref().err().and_then(DraftError::entry), None);
        assert_eq!(e.draws(), e.layout.draws);
    }

    #[test]
    fn cell_quad_covers_cell_under_ray_on_level_plane() {
        let (_, _, mut e) = empty();
        e.level = 1;
        let slant = Ray {
            origin: vec3(0.5, 7.0, 6.0),
            dir: vec3(2.0, -1.0, 0.0).normalize(),
        };
        let to_world = e.cell_quad(slant).expect("квад");
        let (vertices, indices) = quad();
        let corners: Vec<Vec3> = (vertices.iter())
            .map(|v| to_world.transform_point3(Vec3::from(v.pos)))
            .collect();
        let top = 2.0 + QUAD_LIFT;
        assert_eq!(
            corners,
            [
                vec3(8.0, top, 4.0),
                vec3(8.0, top, 8.0),
                vec3(12.0, top, 8.0),
                vec3(12.0, top, 4.0)
            ]
        );
        for v in &vertices {
            assert_eq!((v.normal, v.uv), ([0.0, 1.0, 0.0], [v.pos[0], v.pos[2]]));
        }
        for tri in indices.as_chunks::<3>().0 {
            let [p0, p1, p2] = tri.map(|i| corners[i as usize]);
            assert!((p1 - p0).cross(p2 - p0).y > 0.0, "лицом вверх");
        }
        let up = Ray {
            dir: Vec3::Y,
            ..slant
        };
        assert_eq!(e.cell_quad(up), None);
    }
}
