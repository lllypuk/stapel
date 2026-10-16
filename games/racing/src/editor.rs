use std::collections::VecDeque;

use engine::{Aabb3, Assets, IVec2, Ray, Vec3, vec2};

use crate::layout::{Layout, Models};
use crate::pieces::{DecorPiece, KIT_SCALE, LEVEL, PIECES, Piece, Rot};
use crate::track::{DecorEntry, Entry, MAX_LEVEL, RoadEntry, TrackDraft};

/// Снимков отмены не больше.
pub(crate) const UNDO_LIMIT: usize = 100;
/// Дальше луч по миру не ищет, м.
const PICK_RANGE: f32 = 1000.0;

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

/// Команда панели редактора; разбирает следующий `update`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EditorCmd {
    Tool(Tool),
    Level(u32),
    Undo,
}

/// Правка черновика трассы инструментом на текущем уровне.
pub(crate) struct Editor {
    pub(crate) draft: TrackDraft,
    /// Перестраивается после каждой правки и отмены: по нему идёт пикинг.
    pub(crate) layout: Layout,
    pub(crate) tool: Tool,
    pub(crate) level: u32,
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

/// Клетка над или под точкой.
fn cell(p: Vec3) -> IVec2 {
    (vec2(p.x, p.z) / KIT_SCALE).floor().as_ivec2()
}

impl Editor {
    /// Чистый редактор черновика `draft` с диска.
    pub(crate) fn new(draft: TrackDraft, models: &Models, assets: &Assets) -> Self {
        Self {
            layout: Layout::new(&draft, models, assets),
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

    pub(crate) fn apply(&mut self, cmd: EditorCmd, models: &Models, assets: &Assets) {
        match cmd {
            EditorCmd::Tool(tool) => self.tool = tool,
            EditorCmd::Level(level) => self.level = level.min(MAX_LEVEL),
            EditorCmd::Undo => {
                if let Some(before) = self.undo.pop_back() {
                    self.draft = before;
                    self.layout = Layout::new(&self.draft, models, assets);
                }
            }
        }
    }

    /// Высота плоскости текущего уровня, м.
    fn floor(&self) -> f32 {
        self.level as f32 * LEVEL * KIT_SCALE
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
        self.layout = Layout::new(&self.draft, models, assets);
        true
    }
}

#[cfg(test)]
mod tests {
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
}
