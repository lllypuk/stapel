use std::collections::{HashMap, HashSet};
use std::fmt;

use engine::{IVec2, Mat4, Vec3, ivec2, vec3};
use serde::{Deserialize, Serialize};
use toml::Spanned;

use crate::pieces::{DecorPiece, End, KIT_SCALE, LEVEL, Piece, Rot, decor_piece, piece};

/// Запись черновика: индекс в `roads` или `decor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Road(usize),
    Decor(usize),
}

/// Ошибка черновика; `road` — индекс в `roads`.
#[derive(Debug, PartialEq)]
pub(crate) enum DraftError {
    Overlap {
        road: usize,
        cell: IVec2,
        level: u32,
    },
    Start {
        count: usize,
    },
    Gap {
        road: usize,
    },
    Branch {
        road: usize,
    },
    Level {
        road: usize,
        level: u32,
        found: u32,
    },
    Bounds {
        entry: Entry,
    },
}

impl DraftError {
    /// У `Start` записи нет.
    pub(crate) fn entry(&self) -> Option<Entry> {
        match *self {
            Self::Overlap { road, .. }
            | Self::Gap { road }
            | Self::Branch { road }
            | Self::Level { road, .. } => Some(Entry::Road(road)),
            Self::Bounds { entry } => Some(entry),
            Self::Start { .. } => None,
        }
    }
}

impl fmt::Display for DraftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overlap { cell, level, .. } => {
                write!(f, "клетка {cell} уровня {level} уже занята")
            }
            Self::Start { count } => write!(f, "нужен один roadStart, а их {count}"),
            Self::Gap { .. } => f.write_str("за выходом детали нет дороги"),
            Self::Branch { .. } => f.write_str("деталь вне цепочки от старта"),
            Self::Level { level, found, .. } => write!(
                f,
                "выход на уровне {level}, а дорога за ним — на уровне {found}"
            ),
            Self::Bounds { .. } => write!(
                f,
                "клетка вне ±{MAX_CELL}, уровень больше {MAX_LEVEL} или позиция не число либо вне трассы"
            ),
        }
    }
}

/// Ошибка разбора трассы; `line` — с единицы, строка поля `piece`.
#[derive(Debug, PartialEq)]
pub(crate) enum TrackError {
    Toml(String),
    UnknownPiece {
        line: usize,
        name: String,
    },
    Rot {
        line: usize,
        rot: i64,
    },
    Draft {
        line: Option<usize>,
        error: DraftError,
    },
}

/// Клетка по модулю и уровень — не больше: в debug сумма клеток детали иначе паникует переполнением.
const MAX_CELL: i32 = 1000;
pub(crate) const MAX_LEVEL: u32 = 16;

impl fmt::Display for TrackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(msg) => f.write_str(msg),
            Self::UnknownPiece { line, name } => {
                write!(f, "строка {line}: неизвестная деталь {name:?}")
            }
            Self::Rot { line, rot } => write!(f, "строка {line}: rot {rot} не кратен 90"),
            Self::Draft {
                line: Some(line),
                error,
            } => write!(f, "строка {line}: {error}"),
            Self::Draft { line: None, error } => error.fmt(f),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoadEntry {
    pub(crate) piece: &'static Piece,
    pub(crate) cell: IVec2,
    pub(crate) level: u32,
    pub(crate) rot: Rot,
}

impl RoadEntry {
    /// Стыки в клетках и уровнях трассы.
    fn ends(&self) -> [End; 2] {
        self.piece.ends(self.rot).map(|e| End {
            cell: self.cell + e.cell,
            side: e.side,
            level: self.level + e.level,
        })
    }

    /// Занятые клетки трассы с уровнями.
    pub(crate) fn cells(&self) -> impl Iterator<Item = (IVec2, u32)> {
        self.piece.cells(self.rot).flat_map(move |c| {
            (self.piece.levels.clone()).map(move |l| (self.cell + c, self.level + l))
        })
    }

    /// Клетки детали → мир: клетки трассы по 4 м, поворот вокруг центра якорной клетки.
    pub(crate) fn frame(&self) -> Mat4 {
        let anchor = vec3(
            self.cell.x as f32 + 0.5,
            self.level as f32 * LEVEL,
            self.cell.y as f32 + 0.5,
        );
        Mat4::from_translation(anchor * KIT_SCALE)
            * Mat4::from_rotation_y(self.rot.radians())
            * Mat4::from_scale(Vec3::splat(KIT_SCALE))
            * Mat4::from_translation(vec3(-0.5, 0.0, -0.5))
    }

    /// Модель → мир.
    pub(crate) fn transform(&self) -> Mat4 {
        self.frame() * Mat4::from_translation(self.piece.offset)
    }

    /// Запись `road` в пределах и не на клетках `taken`.
    fn check(&self, road: usize, taken: &HashMap<(IVec2, u32), usize>) -> Result<(), DraftError> {
        if self
            .cell
            .to_array()
            .iter()
            .any(|c| c.unsigned_abs() > MAX_CELL.unsigned_abs())
            || self.level > MAX_LEVEL
        {
            return Err(DraftError::Bounds {
                entry: Entry::Road(road),
            });
        }
        match self.cells().find(|c| taken.contains_key(c)) {
            Some((cell, level)) => Err(DraftError::Overlap { road, cell, level }),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DecorEntry {
    pub(crate) piece: &'static DecorPiece,
    /// В метрах.
    pub(crate) pos: Vec3,
    pub(crate) yaw_deg: f32,
}

impl DecorEntry {
    /// Модель → мир.
    pub(crate) fn transform(&self) -> Mat4 {
        Mat4::from_translation(self.pos)
            * Mat4::from_rotation_y(self.yaw_deg.to_radians())
            * Mat4::from_scale(Vec3::splat(self.piece.scale))
            * Mat4::from_translation(self.piece.offset)
    }

    pub(crate) fn in_bounds(&self) -> bool {
        let limit = (MAX_CELL + 1) as f32 * KIT_SCALE;
        self.pos.is_finite() && self.pos.abs().max_element() <= limit && self.yaw_deg.is_finite()
    }
}

/// Прямоугольник поперёк дороги на выходе детали: ширина клетки, высота — до дороги выше.
#[derive(Debug, PartialEq)]
pub(crate) struct Checkpoint {
    pub(crate) road: usize,
    /// Середина нижнего края, м.
    pub(crate) origin: Vec3,
    /// Горизонтальное направление цепочки.
    pub(crate) normal: Vec3,
    /// Над выходом без дороги выше — бесконечность, м.
    pub(crate) height: f32,
}

impl Checkpoint {
    /// `ceiling` — нижний занятый уровень над выходом в клетках по обе стороны стыка.
    fn new(road: usize, exit: End, ceiling: Option<u32>) -> Self {
        let step = exit.side.step().as_vec2();
        let center = exit.cell.as_vec2() + 0.5 + step / 2.0;
        Self {
            road,
            origin: vec3(center.x, exit.level as f32 * LEVEL, center.y) * KIT_SCALE,
            normal: vec3(step.x, 0.0, step.y),
            height: ceiling.map_or(f32::INFINITY, |l| {
                (l - exit.level) as f32 * LEVEL * KIT_SCALE
            }),
        }
    }

    /// Отрезок движения `from → to` пересёк прямоугольник по направлению цепочки.
    pub(crate) fn crossed(&self, from: Vec3, to: Vec3) -> bool {
        let (a, b) = (
            (from - self.origin).dot(self.normal),
            (to - self.origin).dot(self.normal),
        );
        if a >= 0.0 || b < 0.0 {
            return false;
        }
        let p = from.lerp(to, a / (a - b)) - self.origin;
        let across = self.normal.cross(Vec3::Y);
        p.dot(across).abs() <= KIT_SCALE / 2.0 && (0.0..=self.height).contains(&p.y)
    }
}

/// Клетка и уровень точки в метрах; ниже уровня 0 — `None`.
pub(crate) fn cell_at(p: Vec3) -> Option<(IVec2, u32)> {
    let c = (p / vec3(KIT_SCALE, LEVEL * KIT_SCALE, KIT_SCALE))
        .floor()
        .as_ivec3();
    Some((ivec2(c.x, c.z), u32::try_from(c.y).ok()?))
}

/// Трасса как в файле: цепочка не обязана замыкаться, порядок записей — порядок файла.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrackDraft {
    pub(crate) car: String,
    pub(crate) roads: Vec<RoadEntry>,
    pub(crate) decor: Vec<DecorEntry>,
}

/// `N` — имя детали: `Spanned` при чтении ради строки ошибки, `&str` при записи.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, bound(deserialize = "N: Deserialize<'de>"))]
struct TrackFile<N> {
    car: String,
    #[serde(default)]
    road: Vec<RoadFile<N>>,
    #[serde(default)]
    decor: Vec<DecorFile<N>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RoadFile<N> {
    piece: N,
    cell: [i32; 2],
    #[serde(default)]
    level: u32,
    #[serde(default)]
    rot: i64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DecorFile<N> {
    piece: N,
    pos: [f32; 3],
    #[serde(default)]
    yaw: f32,
}

/// Строки поля `piece` записей файла.
#[derive(Debug)]
pub(crate) struct Lines {
    roads: Vec<usize>,
    decor: Vec<usize>,
}

impl Lines {
    pub(crate) fn locate(&self, error: DraftError) -> TrackError {
        let line = error.entry().map(|e| match e {
            Entry::Road(i) => self.roads[i],
            Entry::Decor(i) => self.decor[i],
        });
        TrackError::Draft { line, error }
    }
}

impl TrackDraft {
    #[cfg(test)]
    pub(crate) fn parse(text: &str) -> Result<Self, TrackError> {
        Self::parse_lines(text).map(|(draft, _)| draft)
    }

    /// Строки нужны, чтобы назвать место ошибки `chain`.
    pub(crate) fn parse_lines(text: &str) -> Result<(Self, Lines), TrackError> {
        let file: TrackFile<Spanned<String>> =
            toml::from_str(text).map_err(|e| TrackError::Toml(e.to_string()))?;
        let line = |name: &Spanned<String>| text[..name.span().start].matches('\n').count() + 1;
        let unknown = |name: &Spanned<String>| TrackError::UnknownPiece {
            line: line(name),
            name: name.get_ref().clone(),
        };
        let roads = (file.road.iter())
            .map(|r| {
                Ok(RoadEntry {
                    piece: piece(r.piece.get_ref()).ok_or_else(|| unknown(&r.piece))?,
                    cell: IVec2::from(r.cell),
                    level: r.level,
                    rot: Rot::from_degrees(r.rot).ok_or(TrackError::Rot {
                        line: line(&r.piece),
                        rot: r.rot,
                    })?,
                })
            })
            .collect::<Result<_, _>>()?;
        let decor = (file.decor.iter())
            .map(|d| {
                Ok(DecorEntry {
                    piece: decor_piece(d.piece.get_ref()).ok_or_else(|| unknown(&d.piece))?,
                    pos: Vec3::from(d.pos),
                    yaw_deg: d.yaw,
                })
            })
            .collect::<Result<_, _>>()?;
        let lines = Lines {
            roads: file.road.iter().map(|r| line(&r.piece)).collect(),
            decor: file.decor.iter().map(|d| line(&d.piece)).collect(),
        };
        let draft = Self {
            car: file.car,
            roads,
            decor,
        };
        draft.validate().map_err(|e| lines.locate(e))?;
        Ok((draft, lines))
    }

    /// Пределы клеток и позиций, пересечения дорог; цепочку не требует.
    pub(crate) fn validate(&self) -> Result<(), DraftError> {
        let mut taken = HashMap::new();
        for (i, r) in self.roads.iter().enumerate() {
            r.check(i, &taken)?;
            taken.extend(r.cells().map(|c| (c, i)));
        }
        match self.decor.iter().position(|d| !d.in_bounds()) {
            Some(i) => Err(DraftError::Bounds {
                entry: Entry::Decor(i),
            }),
            None => Ok(()),
        }
    }

    /// Запись дороги на каждой занятой клетке и уровне.
    pub(crate) fn occupancy(&self) -> HashMap<(IVec2, u32), usize> {
        (self.roads.iter().enumerate())
            .flat_map(|(i, r)| r.cells().map(move |c| (c, i)))
            .collect()
    }

    /// Встанет ли `road` следующей записью.
    pub(crate) fn can_place(&self, road: &RoadEntry) -> Result<(), DraftError> {
        road.check(self.roads.len(), &self.occupancy())
    }

    /// Файл целиком: комментарии прежнего файла пропадают.
    #[cfg_attr(not(test), expect(dead_code, reason = "сохранение трассы — задача 16"))]
    pub(crate) fn to_toml(&self) -> String {
        let file = TrackFile {
            car: self.car.clone(),
            road: (self.roads.iter())
                .map(|r| RoadFile {
                    piece: r.piece.name,
                    cell: r.cell.to_array(),
                    level: r.level,
                    rot: r.rot.degrees(),
                })
                .collect(),
            decor: (self.decor.iter())
                .map(|d| DecorFile {
                    piece: d.piece.name,
                    pos: d.pos.to_array(),
                    yaw: d.yaw_deg,
                })
                .collect(),
        };
        toml::to_string(&file).expect("строки и конечные числа пишутся в TOML")
    }

    /// Клетки и уровни под дорогой.
    pub(crate) fn road_cells(&self) -> HashSet<(IVec2, u32)> {
        self.roads.iter().flat_map(RoadEntry::cells).collect()
    }

    /// Чекпоинты на выходах деталей по цепочке от `roadStart`, первый — у старта.
    /// Повториться может только старт: стык в занятой клетке уже отсечён `Overlap`.
    pub(crate) fn chain(&self) -> Result<Vec<Checkpoint>, DraftError> {
        let starts: Vec<usize> = (0..self.roads.len())
            .filter(|&i| self.roads[i].piece.name == "roadStart")
            .collect();
        let &[start] = &starts[..] else {
            return Err(DraftError::Start {
                count: starts.len(),
            });
        };
        let taken = self.road_cells();
        let mut checkpoints = Vec::with_capacity(self.roads.len());
        let (mut road, mut exit) = (start, self.roads[start].ends()[1]);
        loop {
            let (cell, side) = (exit.cell + exit.side.step(), exit.side.opposite());
            let ceiling = (taken.iter())
                .filter(|&&(c, l)| (c == exit.cell || c == cell) && l > exit.level)
                .map(|&(_, l)| l)
                .min();
            checkpoints.push(Checkpoint::new(road, exit, ceiling));
            let next = (self.roads.iter().enumerate())
                .flat_map(|(i, r)| r.ends().into_iter().enumerate().map(move |e| (i, e)))
                .filter(|(_, (_, e))| e.cell == cell && e.side == side)
                .min_by_key(|(_, (_, e))| e.level != exit.level);
            let Some((i, (entry, e))) = next else {
                return Err(DraftError::Gap { road });
            };
            if e.level != exit.level {
                return Err(DraftError::Level {
                    road,
                    level: exit.level,
                    found: e.level,
                });
            }
            if checkpoints.iter().any(|c| c.road == i) {
                break;
            }
            (road, exit) = (i, self.roads[i].ends()[1 - entry]);
        }
        if let Some(road) = (0..self.roads.len()).find(|i| checkpoints.iter().all(|c| c.road != *i))
        {
            return Err(DraftError::Branch { road });
        }
        Ok(checkpoints)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use engine::vec3;

    use super::*;

    fn err(text: &str) -> TrackError {
        TrackDraft::parse(text).expect_err("трасса разобралась")
    }

    fn at(line: usize, error: DraftError) -> TrackError {
        TrackError::Draft {
            line: Some(line),
            error,
        }
    }

    #[test]
    fn parses_fields_and_defaults() {
        let track = TrackDraft::parse(
            "car = \"race\"\n\
             [[road]]\npiece = \"roadCornerLarge\"\ncell = [3, -5]\nlevel = 1\nrot = 270\n\
             [[road]]\npiece = \"roadStraight\"\ncell = [0, 0]\n\
             [[decor]]\npiece = \"barrierRed\"\npos = [17.0, 0.5, 30.0]\nyaw = 30.0\n\
             [[decor]]\npiece = \"cone\"\npos = [1.0, 0.0, 2.0]\n",
        )
        .expect("трасса");
        assert_eq!(track.car, "race");
        let [a, b] = &track.roads[..] else {
            panic!("{:?}", track.roads)
        };
        assert_eq!(a.piece.name, "roadCornerLarge");
        assert_eq!(
            (a.cell, a.level, a.rot),
            (ivec2(3, -5), 1, Rot::from_degrees(270).expect("rot"))
        );
        assert_eq!(b.piece.name, "roadStraight");
        assert_eq!((b.cell, b.level, b.rot), (ivec2(0, 0), 0, Rot::default()));
        let [c, d] = &track.decor[..] else {
            panic!("{:?}", track.decor)
        };
        assert_eq!(
            (c.piece.name, c.pos, c.yaw_deg),
            ("barrierRed", vec3(17.0, 0.5, 30.0), 30.0)
        );
        assert_eq!(
            (d.piece.name, d.pos, d.yaw_deg),
            ("cone", vec3(1.0, 0.0, 2.0), 0.0)
        );
    }

    #[test]
    fn unknown_piece_names_line() {
        assert_eq!(
            err("car = \"race\"\n\n[[road]]\ncell = [0, 0]\npiece = \"roadSplit\"\n"),
            TrackError::UnknownPiece {
                line: 5,
                name: "roadSplit".into()
            }
        );
        assert_eq!(
            err("car = \"race\"\n[[decor]]\npiece = \"roadStraight\"\npos = [0.0, 0.0, 0.0]\n"),
            TrackError::UnknownPiece {
                line: 3,
                name: "roadStraight".into()
            }
        );
    }

    #[test]
    fn rot_must_be_multiple_of_90() {
        let e =
            err("car = \"race\"\n[[road]]\npiece = \"roadStraight\"\ncell = [0, 0]\nrot = 45\n");
        assert_eq!(e, TrackError::Rot { line: 3, rot: 45 });
        assert_eq!(e.to_string(), "строка 3: rot 45 не кратен 90");
    }

    #[test]
    fn huge_cell_or_level_and_non_finite_decor_fail() {
        let road = |cell: &str, level: u32| {
            err(&format!(
                "car = \"race\"\n[[road]]\npiece = \"roadStraightLong\"\ncell = {cell}\nlevel = {level}\n"
            ))
        };
        let bounds = at(
            3,
            DraftError::Bounds {
                entry: Entry::Road(0),
            },
        );
        let e = road("[2147483647, 0]", 0);
        assert_eq!(e, bounds);
        assert_eq!(
            e.to_string(),
            "строка 3: клетка вне ±1000, уровень больше 16 или позиция не число либо вне трассы"
        );
        assert_eq!(road("[0, -1001]", 0), bounds);
        assert_eq!(road("[-2147483648, 0]", 0), bounds);
        assert_eq!(road("[0, 0]", 17), bounds);
        for (pos, yaw) in [
            ("[nan, 0, 0]", "0"),
            ("[0, 0, 0]", "inf"),
            ("[3000000, 0, 0]", "0"),
            ("[0, -4004.01, 0]", "0"),
        ] {
            let e = err(&format!(
                "car = \"race\"\n[[decor]]\npiece = \"cone\"\npos = {pos}\nyaw = {yaw}\n"
            ));
            assert_eq!(
                e,
                at(
                    3,
                    DraftError::Bounds {
                        entry: Entry::Decor(0)
                    }
                )
            );
        }
    }

    #[test]
    fn largest_cell_and_level_parse() {
        let text = "car = \"race\"\n[[road]]\npiece = \"roadStraight\"\ncell = [1000, -1000]\nlevel = 16\n";
        let track = TrackDraft::parse(text).expect("трасса");
        assert_eq!(
            (track.roads[0].cell, track.roads[0].level),
            (ivec2(1000, -1000), 16)
        );
        let edge = "car = \"race\"\n[[decor]]\npiece = \"cone\"\npos = [4004.0, 0.0, -4004.0]\n";
        assert_eq!(
            TrackDraft::parse(edge).expect("трасса").decor[0].pos,
            vec3(4004.0, 0.0, -4004.0)
        );
    }

    fn roads(entries: &[(&str, [i32; 2], u32, i64)]) -> String {
        entries.iter().fold(
            String::from("car = \"race\"\n"),
            |mut text, (piece, [x, z], level, rot)| {
                let _ = write!(
                    text,
                    "[[road]]\npiece = \"{piece}\"\ncell = [{x}, {z}]\nlevel = {level}\nrot = {rot}\n"
                );
                text
            },
        )
    }

    #[test]
    fn two_pieces_in_one_cell_and_level_fail() {
        assert_eq!(
            err(&roads(&[
                ("roadStraight", [2, 3], 0, 0),
                ("roadStraight", [2, 3], 0, 90)
            ])),
            at(
                8,
                DraftError::Overlap {
                    road: 1,
                    cell: ivec2(2, 3),
                    level: 0
                }
            )
        );
        // Вторая клетка длинной прямой при rot = 90 — +X.
        assert_eq!(
            err(&roads(&[
                ("roadStraightLong", [0, 0], 0, 90),
                ("roadCornerSmall", [1, 0], 0, 0)
            ])),
            at(
                8,
                DraftError::Overlap {
                    road: 1,
                    cell: ivec2(1, 0),
                    level: 0
                }
            )
        );
        assert_eq!(
            err(&roads(&[
                ("roadStraight", [0, 0], 1, 0),
                ("roadStraightBridgeMid", [0, 0], 0, 0)
            ])),
            at(
                8,
                DraftError::Overlap {
                    road: 1,
                    cell: ivec2(0, 0),
                    level: 1
                }
            )
        );
        assert_eq!(
            err(&roads(&[
                ("roadStraight", [0, 0], 0, 0),
                ("roadStraightBridge", [0, 0], 0, 0)
            ]))
            .to_string(),
            "строка 8: клетка [0, 0] уровня 0 уже занята"
        );
    }

    #[test]
    fn piece_under_bridge_on_other_level_is_fine() {
        let track = TrackDraft::parse(&roads(&[
            ("roadStraight", [0, 0], 0, 90),
            ("roadStraightBridgeMid", [0, 0], 0, 0),
            ("roadStraight", [0, 0], 2, 0),
        ]))
        .expect("трасса");
        assert_eq!(track.roads.len(), 3);
    }

    #[test]
    fn toml_error_reports_line() {
        let e = err("car = \"race\"\n[[road]]\npiece = \n");
        assert!(matches!(e, TrackError::Toml(_)), "{e:?}");
        assert!(e.to_string().contains("line 3"), "{e}");
        assert!(matches!(err("[[road]]\n"), TrackError::Toml(_)));
        assert!(
            err("car = \"race\"\n[[road]]\npiece = \"roadStraight\"\ncell = [0, 0]\nrott = 90\n")
                .to_string()
                .contains("rott")
        );
    }

    #[test]
    fn oval_parses() {
        let track = TrackDraft::parse(include_str!("testdata/oval.toml")).expect("овал");
        assert_eq!(track.car, "race");
        assert_eq!(track.roads.len(), 10);
        assert_eq!(
            track
                .roads
                .iter()
                .filter(|r| r.piece.name == "roadStart")
                .count(),
            1
        );
        assert_ne!(track.decor, []);
    }

    fn chain(text: &str) -> Result<Vec<Checkpoint>, TrackError> {
        let (track, lines) = TrackDraft::parse_lines(text).expect("трасса");
        track.chain().map_err(|e| lines.locate(e))
    }

    /// Строка `cell = …` за строкой ошибки.
    fn cell_after(text: &str, line: usize) -> &str {
        text.lines().nth(line).expect("строка")
    }

    #[test]
    fn oval_chains_every_road_from_start_in_its_direction() {
        let oval = TrackDraft::parse(include_str!("testdata/oval.toml")).expect("овал");
        let checkpoints = oval.chain().expect("цепочка");
        assert_eq!(
            checkpoints.iter().map(|c| c.road).collect::<Vec<_>>(),
            [1, 0, 9, 8, 7, 6, 5, 4, 3, 2]
        );
        // Старт — клетки [2, 0] и [3, 0], выход к −X.
        assert_eq!(
            checkpoints[0],
            Checkpoint {
                road: 1,
                origin: vec3(8.0, 0.0, 2.0),
                normal: vec3(-1.0, 0.0, 0.0),
                height: f32::INFINITY,
            }
        );
        // Угол [0, 0] выводит к +Z из клетки [0, 1].
        assert_eq!(checkpoints[1].origin, vec3(2.0, 0.0, 8.0));
        assert_eq!(checkpoints[1].normal, vec3(0.0, 0.0, 1.0));
        for (c, next) in checkpoints.iter().zip(checkpoints.iter().cycle().skip(1)) {
            let road = &oval.roads[next.road];
            let ahead = ((c.origin + c.normal * 2.0) / KIT_SCALE).floor().as_ivec3();
            assert!(
                road.piece
                    .cells(road.rot)
                    .any(|cell| road.cell + cell == ivec2(ahead.x, ahead.z)),
                "{c:?} → {}",
                road.piece.name
            );
        }
    }

    #[test]
    fn gap_names_last_piece_before_it() {
        let text = include_str!("testdata/gap.toml");
        let e = chain(text).expect_err("цепочка с дырой");
        let TrackError::Draft {
            line: Some(line),
            error: DraftError::Gap { .. },
        } = e
        else {
            panic!("{e:?}")
        };
        assert_eq!(cell_after(text, line), "cell = [6, 4]");
        assert_eq!(
            e.to_string(),
            format!("строка {line}: за выходом детали нет дороги")
        );
    }

    #[test]
    fn branch_off_the_loop_fails() {
        let text = include_str!("testdata/branch.toml");
        let e = chain(text).expect_err("цепочка с веткой");
        let TrackError::Draft {
            line: Some(line),
            error: DraftError::Branch { .. },
        } = e
        else {
            panic!("{e:?}")
        };
        assert_eq!(cell_after(text, line), "cell = [3, -1]");
        assert_eq!(
            e.to_string(),
            format!("строка {line}: деталь вне цепочки от старта")
        );
    }

    #[test]
    fn level_mismatch_fails_but_road_on_matching_level_wins() {
        // Выход старта [0, 0] к −Z на уровне 0, мост за ним — на уровне 1.
        let e = chain(&roads(&[
            ("roadStart", [0, 0], 0, 0),
            ("roadStraightBridge", [0, -1], 0, 0),
        ]))
        .expect_err("уровни не совпали");
        assert_eq!(
            e,
            at(
                3,
                DraftError::Level {
                    road: 0,
                    level: 0,
                    found: 1
                }
            )
        );
        assert_eq!(
            e.to_string(),
            "строка 3: выход на уровне 0, а дорога за ним — на уровне 1"
        );
        assert_eq!(
            chain(&roads(&[
                ("roadStart", [0, 0], 0, 0),
                ("roadStraightBridgeMid", [0, -1], 0, 0),
                ("roadStraight", [0, -1], 0, 0),
            ])),
            Err(at(13, DraftError::Gap { road: 2 }))
        );
    }

    #[test]
    fn exactly_one_start() {
        let e = chain(&roads(&[("roadStraight", [0, 0], 0, 0)])).expect_err("без старта");
        assert_eq!(
            e,
            TrackError::Draft {
                line: None,
                error: DraftError::Start { count: 0 }
            }
        );
        assert_eq!(e.to_string(), "нужен один roadStart, а их 0");
        assert_eq!(
            chain(&roads(&[
                ("roadStart", [0, 0], 0, 0),
                ("roadStart", [1, 0], 0, 0)
            ])),
            Err(TrackError::Draft {
                line: None,
                error: DraftError::Start { count: 2 }
            })
        );
    }

    #[test]
    fn figure_eight_passes_both_pieces_under_bridge_in_order() {
        let track = TrackDraft::parse(include_str!("testdata/bridge8.toml")).expect("восьмёрка");
        let checkpoints = track.chain().expect("цепочка");
        assert_eq!(
            checkpoints.iter().map(|c| c.road).collect::<Vec<_>>(),
            (0..track.roads.len()).collect::<Vec<_>>()
        );
        let [under, over] = [3, 12].map(|i| &track.roads[i]);
        assert_eq!((under.cell, under.level), (IVec2::ZERO, 0));
        assert_eq!(over.cell, IVec2::ZERO);
        assert_eq!(over.piece.name, "roadStraightBridgeMid");
        assert_eq!(checkpoints[3].origin, vec3(0.0, 0.0, 2.0));
        assert_eq!(checkpoints[12].origin, vec3(2.0, 2.0, 4.0));
    }

    #[test]
    fn road_cells_hold_every_level_of_piece() {
        let track = TrackDraft::parse(include_str!("testdata/bridge8.toml")).expect("восьмёрка");
        let cells = track.road_cells();
        let taken: usize = (track.roads.iter())
            .map(|r| r.piece.cells(r.rot).count() * r.piece.levels.clone().count())
            .sum();
        assert_eq!(cells.len(), taken);
        assert!(cells.contains(&(IVec2::ZERO, 0)) && cells.contains(&(IVec2::ZERO, 1)));
        assert!(cells.contains(&(ivec2(-2, -1), 0)), "угол 2×2 целиком");
        assert!(!cells.contains(&(ivec2(1, 0), 1)));
    }

    #[test]
    fn cell_at_floors_meters_to_cell_and_level_band() {
        assert_eq!(cell_at(vec3(0.5, 0.3, 3.9)), Some((IVec2::ZERO, 0)));
        assert_eq!(cell_at(vec3(-0.1, 2.0, 4.0)), Some((ivec2(-1, 1), 1)));
        assert_eq!(cell_at(vec3(8.0, 1.99, -4.1)), Some((ivec2(2, -2), 0)));
        assert_eq!(cell_at(vec3(0.0, -0.01, 0.0)), None);
    }

    /// Выход клетки [0, 0] к +X под дорогой уровнем выше: плоскость x = 4, z в 0..4, y в 0..2.
    fn exit_pos_x(level: u32) -> Checkpoint {
        Checkpoint::new(
            0,
            End {
                cell: IVec2::ZERO,
                side: crate::pieces::Side::PosX,
                level,
            },
            Some(level + 1),
        )
    }

    #[test]
    fn checkpoint_counts_crossing_in_chain_direction() {
        let c = exit_pos_x(0);
        assert_eq!(c.origin, vec3(4.0, 0.0, 2.0));
        assert!(c.crossed(vec3(3.5, 0.5, 2.0), vec3(4.5, 0.5, 2.5)));
        assert!(c.crossed(vec3(3.5, 0.5, 3.9), vec3(4.5, 0.5, 3.9)));
        assert!(c.crossed(vec3(3.5, 1.9, 0.1), vec3(4.5, 1.9, 0.1)));
        // Пересечение на середине косого отрезка, концы — за краем прямоугольника.
        assert!(c.crossed(vec3(3.0, 0.5, -1.0), vec3(5.0, 0.5, 5.0)));
        // Ровно на плоскости — засчитан конец, а не начало следующего отрезка.
        assert!(c.crossed(vec3(3.5, 0.5, 2.0), vec3(4.0, 0.5, 2.0)));
        assert!(!c.crossed(vec3(4.0, 0.5, 2.0), vec3(4.5, 0.5, 2.0)));
        let up = exit_pos_x(1);
        assert!(up.crossed(vec3(3.5, 2.5, 2.0), vec3(4.5, 2.5, 2.0)));
        assert!(!up.crossed(vec3(3.5, 0.5, 2.0), vec3(4.5, 0.5, 2.0)));
    }

    #[test]
    fn checkpoint_ignores_reverse_crossing() {
        let c = exit_pos_x(0);
        assert!(!c.crossed(vec3(4.5, 0.5, 2.0), vec3(3.5, 0.5, 2.0)));
        assert!(!c.crossed(vec3(3.0, 0.5, 2.0), vec3(3.5, 0.5, 2.0)));
        assert!(!c.crossed(vec3(4.5, 0.5, 2.0), vec3(5.0, 0.5, 2.0)));
    }

    #[test]
    fn parallel_road_outside_rectangle_does_not_count() {
        let c = exit_pos_x(0);
        for z in [4.1, 6.0, -0.1, -2.0] {
            assert!(!c.crossed(vec3(3.5, 0.5, z), vec3(4.5, 0.5, z)), "{z}");
        }
        for y in [2.1, -0.1] {
            assert!(!c.crossed(vec3(3.5, y, 2.0), vec3(4.5, y, 2.0)), "{y}");
        }
    }

    #[test]
    fn checkpoint_without_road_above_counts_any_jump_height() {
        let c = Checkpoint {
            height: f32::INFINITY,
            ..exit_pos_x(0)
        };
        assert!(c.crossed(vec3(3.5, 9.0, 2.0), vec3(4.5, 9.0, 2.0)));
        assert!(!c.crossed(vec3(3.5, -0.1, 2.0), vec3(4.5, -0.1, 2.0)));
    }

    #[test]
    fn bridge8_checkpoints_stop_under_bridge_deck() {
        let track = TrackDraft::parse(include_str!("testdata/bridge8.toml")).expect("восьмёрка");
        let checkpoints = track.chain().expect("цепочка");
        let under: Vec<_> = (checkpoints.iter())
            .filter(|c| c.height.is_finite())
            .map(|c| (c.origin.y, c.height))
            .collect();
        assert_eq!(under, [(0.0, LEVEL * KIT_SCALE); 4]);
    }

    fn road(name: &str, cell: IVec2, level: u32, deg: i64) -> RoadEntry {
        RoadEntry {
            piece: piece(name).expect("деталь"),
            cell,
            level,
            rot: Rot::from_degrees(deg).expect("rot"),
        }
    }

    fn draft(roads: Vec<RoadEntry>) -> TrackDraft {
        TrackDraft {
            car: "race".into(),
            roads,
            decor: Vec::new(),
        }
    }

    #[test]
    fn toml_round_trip_keeps_draft() {
        for text in [
            include_str!("../assets/tracks/track01.toml"),
            include_str!("../assets/tracks/track02.toml"),
            include_str!("../assets/tracks/track03.toml"),
            include_str!("testdata/oval.toml"),
            include_str!("testdata/gap.toml"),
            include_str!("testdata/branch.toml"),
            include_str!("testdata/bridge8.toml"),
        ] {
            let track = TrackDraft::parse(text).expect("трасса");
            assert_eq!(TrackDraft::parse(&track.to_toml()), Ok(track));
        }
    }

    #[test]
    fn to_toml_writes_file_back_without_angle_tails() {
        let text = "car = \"race\"\n\n\
                    [[road]]\npiece = \"roadStart\"\ncell = [-3, 4]\nlevel = 2\nrot = 270\n\n\
                    [[decor]]\npiece = \"cone\"\npos = [0.37, 0.5, -5.96]\nyaw = 30.0\n";
        assert_eq!(TrackDraft::parse(text).expect("трасса").to_toml(), text);
    }

    #[test]
    fn draft_errors_name_entry_and_parse_turns_it_into_line() {
        let mut track = draft(vec![
            road("roadStraight", IVec2::ZERO, 0, 0),
            road("roadStraight", ivec2(5, 5), 0, 0),
            road("roadStraightLong", ivec2(5, 4), 0, 0),
        ]);
        let e = track.validate().expect_err("пересечение");
        assert_eq!(
            e,
            DraftError::Overlap {
                road: 2,
                cell: ivec2(5, 5),
                level: 0
            }
        );
        assert_eq!(e.entry(), Some(Entry::Road(2)));
        assert_eq!(e.to_string(), "клетка [5, 5] уровня 0 уже занята");
        track.roads.pop();
        let cone = |pos| DecorEntry {
            piece: decor_piece("cone").expect("конус"),
            pos,
            yaw_deg: 0.0,
        };
        track.decor = vec![cone(Vec3::ZERO), cone(Vec3::NAN)];
        let e = track.validate().expect_err("позиция");
        assert_eq!(e.entry(), Some(Entry::Decor(1)));
        track.decor.clear();
        let e = track.chain().expect_err("без старта");
        assert_eq!(
            (e.entry(), e.to_string()),
            (None, "нужен один roadStart, а их 0".into())
        );
        assert_eq!(
            err("car = \"race\"\n\
                 [[decor]]\npiece = \"cone\"\npos = [0.0, 0.0, 0.0]\n\
                 [[decor]]\npiece = \"cone\"\npos = [0.0, 0.0, 0.0]\nyaw = nan\n"),
            at(
                6,
                DraftError::Bounds {
                    entry: Entry::Decor(1)
                }
            )
        );
    }

    #[test]
    fn can_place_refuses_taken_cells_on_any_level_and_out_of_bounds() {
        // Рампа занимает [0, 0] и [0, 1] на уровнях 0 и 1.
        let track = draft(vec![
            road("roadRampLong", IVec2::ZERO, 0, 0),
            road("roadStraight", ivec2(3, 0), 1, 0),
        ]);
        let taken = |cell, level| {
            Err(DraftError::Overlap {
                road: 2,
                cell,
                level,
            })
        };
        let place = |name, cell, level| track.can_place(&road(name, cell, level, 0));
        assert_eq!(place("roadStraight", ivec2(0, 1), 1), taken(ivec2(0, 1), 1));
        assert_eq!(place("roadStraight", IVec2::ZERO, 0), taken(IVec2::ZERO, 0));
        assert_eq!(
            place("roadRampLong", ivec2(3, -1), 0),
            taken(ivec2(3, 0), 1)
        );
        assert_eq!(place("roadStraight", IVec2::ZERO, 2), Ok(()));
        assert_eq!(place("roadStraight", ivec2(3, 0), 0), Ok(()));
        assert_eq!(place("roadStraight", ivec2(1000, -1000), 16), Ok(()));
        let out = Err(DraftError::Bounds {
            entry: Entry::Road(2),
        });
        assert_eq!(place("roadStraight", ivec2(1001, 0), 0), out);
        assert_eq!(place("roadStraight", ivec2(0, -1001), 0), out);
        assert_eq!(place("roadStraight", ivec2(5, 5), 17), out);
    }

    #[test]
    fn occupancy_names_owner_of_every_cell() {
        let track = TrackDraft::parse(include_str!("testdata/bridge8.toml")).expect("восьмёрка");
        let owners = track.occupancy();
        assert_eq!(owners.len(), track.road_cells().len());
        for (i, r) in track.roads.iter().enumerate() {
            for c in r.cells() {
                assert_eq!(owners[&c], i, "{c:?} {}", r.piece.name);
            }
        }
        assert!(
            (track.roads.iter()).any(|r| r.cells().count() > 1),
            "есть многоклеточные"
        );
    }

    #[test]
    fn open_draft_validates_but_does_not_chain() {
        let track = draft(vec![
            road("roadStart", IVec2::ZERO, 0, 0),
            road("roadStraight", ivec2(0, -1), 0, 0),
        ]);
        assert_eq!(track.validate(), Ok(()));
        assert_eq!(track.chain(), Err(DraftError::Gap { road: 1 }));
    }
}
