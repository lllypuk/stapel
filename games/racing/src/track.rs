use std::collections::HashSet;
use std::fmt;

use engine::{IVec2, Mat4, Vec3, ivec2, vec3};
use serde::Deserialize;
use toml::Spanned;

use crate::pieces::{DecorPiece, End, KIT_SCALE, LEVEL, Piece, Rot, decor_piece, piece};

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
    Overlap {
        line: usize,
        cell: IVec2,
        level: u32,
    },
    Start {
        count: usize,
    },
    Gap {
        line: usize,
    },
    Branch {
        line: usize,
    },
    Level {
        line: usize,
        level: u32,
        found: u32,
    },
}

impl fmt::Display for TrackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(msg) => f.write_str(msg),
            Self::UnknownPiece { line, name } => {
                write!(f, "строка {line}: неизвестная деталь {name:?}")
            }
            Self::Rot { line, rot } => write!(f, "строка {line}: rot {rot} не кратен 90"),
            Self::Overlap { line, cell, level } => {
                write!(f, "строка {line}: клетка {cell} уровня {level} уже занята")
            }
            Self::Start { count } => write!(f, "нужен один roadStart, а их {count}"),
            Self::Gap { line } => write!(f, "строка {line}: за выходом детали нет дороги"),
            Self::Branch { line } => write!(f, "строка {line}: деталь вне цепочки от старта"),
            Self::Level { line, level, found } => write!(
                f,
                "строка {line}: выход на уровне {level}, а дорога за ним — на уровне {found}"
            ),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Road {
    pub(crate) piece: &'static Piece,
    pub(crate) cell: IVec2,
    pub(crate) level: u32,
    pub(crate) rot: Rot,
    pub(crate) line: usize,
}

impl Road {
    /// Стыки в клетках и уровнях трассы.
    fn ends(&self) -> [End; 2] {
        self.piece.ends(self.rot).map(|e| End {
            cell: self.cell + e.cell,
            side: e.side,
            level: self.level + e.level,
        })
    }

    /// Занятые клетки трассы с уровнями.
    fn cells(&self) -> impl Iterator<Item = (IVec2, u32)> {
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
}

#[derive(Debug)]
pub(crate) struct Decor {
    pub(crate) piece: &'static DecorPiece,
    /// В метрах.
    pub(crate) pos: Vec3,
    /// В радианах; в файле — градусы.
    pub(crate) yaw: f32,
}

impl Decor {
    /// Модель → мир.
    pub(crate) fn transform(&self) -> Mat4 {
        Mat4::from_translation(self.pos)
            * Mat4::from_rotation_y(self.yaw)
            * Mat4::from_scale(Vec3::splat(self.piece.scale))
            * Mat4::from_translation(self.piece.offset)
    }
}

/// Прямоугольник поперёк дороги на выходе детали: ширина клетки, высота — полоса уровня.
#[derive(Debug, PartialEq)]
pub(crate) struct Checkpoint {
    pub(crate) road: usize,
    /// Середина нижнего края, м.
    pub(crate) origin: Vec3,
    /// Горизонтальное направление цепочки.
    pub(crate) normal: Vec3,
}

impl Checkpoint {
    fn new(road: usize, exit: End) -> Self {
        let step = exit.side.step().as_vec2();
        let center = exit.cell.as_vec2() + 0.5 + step / 2.0;
        Self {
            road,
            origin: vec3(center.x, exit.level as f32 * LEVEL, center.y) * KIT_SCALE,
            normal: vec3(step.x, 0.0, step.y),
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
        p.dot(across).abs() <= KIT_SCALE / 2.0 && (0.0..=LEVEL * KIT_SCALE).contains(&p.y)
    }
}

/// Клетка и уровень точки в метрах; ниже уровня 0 — `None`.
pub(crate) fn cell_at(p: Vec3) -> Option<(IVec2, u32)> {
    let c = (p / vec3(KIT_SCALE, LEVEL * KIT_SCALE, KIT_SCALE))
        .floor()
        .as_ivec3();
    Some((ivec2(c.x, c.z), u32::try_from(c.y).ok()?))
}

#[derive(Debug)]
pub(crate) struct Track {
    pub(crate) car: String,
    pub(crate) road: Vec<Road>,
    pub(crate) decor: Vec<Decor>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackFile {
    car: String,
    #[serde(default)]
    road: Vec<RoadFile>,
    #[serde(default)]
    decor: Vec<DecorFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoadFile {
    piece: Spanned<String>,
    cell: [i32; 2],
    #[serde(default)]
    level: u32,
    #[serde(default)]
    rot: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecorFile {
    piece: Spanned<String>,
    pos: [f32; 3],
    #[serde(default)]
    yaw: f32,
}

impl Track {
    pub(crate) fn parse(text: &str) -> Result<Self, TrackError> {
        let file: TrackFile = toml::from_str(text).map_err(|e| TrackError::Toml(e.to_string()))?;
        let line = |name: &Spanned<String>| text[..name.span().start].matches('\n').count() + 1;
        let unknown = |name: &Spanned<String>| TrackError::UnknownPiece {
            line: line(name),
            name: name.get_ref().clone(),
        };
        let mut taken = HashSet::new();
        let mut road = Vec::with_capacity(file.road.len());
        for r in &file.road {
            let piece = piece(r.piece.get_ref()).ok_or_else(|| unknown(&r.piece))?;
            let rot = Rot::from_degrees(r.rot).ok_or(TrackError::Rot {
                line: line(&r.piece),
                rot: r.rot,
            })?;
            let r = Road {
                piece,
                cell: IVec2::from(r.cell),
                level: r.level,
                rot,
                line: line(&r.piece),
            };
            for (cell, level) in r.cells() {
                if !taken.insert((cell, level)) {
                    return Err(TrackError::Overlap {
                        line: r.line,
                        cell,
                        level,
                    });
                }
            }
            road.push(r);
        }
        let decor = file
            .decor
            .iter()
            .map(|d| {
                Ok(Decor {
                    piece: decor_piece(d.piece.get_ref()).ok_or_else(|| unknown(&d.piece))?,
                    pos: Vec3::from(d.pos),
                    yaw: d.yaw.to_radians(),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            car: file.car,
            road,
            decor,
        })
    }

    /// Клетки и уровни под дорогой.
    pub(crate) fn road_cells(&self) -> HashSet<(IVec2, u32)> {
        self.road.iter().flat_map(Road::cells).collect()
    }

    /// Чекпоинты на выходах деталей по цепочке от `roadStart`, первый — у старта.
    /// Повториться может только старт: стык в занятой клетке уже отсечён `Overlap`.
    pub(crate) fn chain(&self) -> Result<Vec<Checkpoint>, TrackError> {
        let starts: Vec<usize> = (0..self.road.len())
            .filter(|&i| self.road[i].piece.name == "roadStart")
            .collect();
        let &[start] = &starts[..] else {
            return Err(TrackError::Start {
                count: starts.len(),
            });
        };
        let mut checkpoints = Vec::with_capacity(self.road.len());
        let (mut road, mut exit) = (start, self.road[start].ends()[1]);
        loop {
            checkpoints.push(Checkpoint::new(road, exit));
            let (cell, side) = (exit.cell + exit.side.step(), exit.side.opposite());
            let next = (self.road.iter().enumerate())
                .flat_map(|(i, r)| r.ends().into_iter().enumerate().map(move |e| (i, e)))
                .filter(|(_, (_, e))| e.cell == cell && e.side == side)
                .min_by_key(|(_, (_, e))| e.level != exit.level);
            let line = self.road[road].line;
            let Some((i, (entry, e))) = next else {
                return Err(TrackError::Gap { line });
            };
            if e.level != exit.level {
                return Err(TrackError::Level {
                    line,
                    level: exit.level,
                    found: e.level,
                });
            }
            if checkpoints.iter().any(|c| c.road == i) {
                break;
            }
            (road, exit) = (i, self.road[i].ends()[1 - entry]);
        }
        if let Some(r) = (0..self.road.len()).find(|i| checkpoints.iter().all(|c| c.road != *i)) {
            return Err(TrackError::Branch {
                line: self.road[r].line,
            });
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
        Track::parse(text).expect_err("трасса разобралась")
    }

    #[test]
    fn parses_fields_and_defaults() {
        let track = Track::parse(
            "car = \"race\"\n\
             [[road]]\npiece = \"roadCornerLarge\"\ncell = [3, -5]\nlevel = 1\nrot = 270\n\
             [[road]]\npiece = \"roadStraight\"\ncell = [0, 0]\n\
             [[decor]]\npiece = \"barrierRed\"\npos = [17.0, 0.5, 30.0]\nyaw = 30.0\n\
             [[decor]]\npiece = \"cone\"\npos = [1.0, 0.0, 2.0]\n",
        )
        .expect("трасса");
        assert_eq!(track.car, "race");
        let [a, b] = &track.road[..] else {
            panic!("{:?}", track.road)
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
        assert_eq!((c.piece.name, c.pos), ("barrierRed", vec3(17.0, 0.5, 30.0)));
        assert!((c.yaw - 30f32.to_radians()).abs() < 1e-6);
        assert_eq!(
            (d.piece.name, d.pos, d.yaw),
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
            TrackError::Overlap {
                line: 8,
                cell: ivec2(2, 3),
                level: 0
            }
        );
        // Вторая клетка длинной прямой при rot = 90 — +X.
        assert_eq!(
            err(&roads(&[
                ("roadStraightLong", [0, 0], 0, 90),
                ("roadCornerSmall", [1, 0], 0, 0)
            ])),
            TrackError::Overlap {
                line: 8,
                cell: ivec2(1, 0),
                level: 0
            }
        );
        assert_eq!(
            err(&roads(&[
                ("roadStraight", [0, 0], 1, 0),
                ("roadStraightBridgeMid", [0, 0], 0, 0)
            ])),
            TrackError::Overlap {
                line: 8,
                cell: ivec2(0, 0),
                level: 1
            }
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
        let track = Track::parse(&roads(&[
            ("roadStraight", [0, 0], 0, 90),
            ("roadStraightBridgeMid", [0, 0], 0, 0),
            ("roadStraight", [0, 0], 2, 0),
        ]))
        .expect("трасса");
        assert_eq!(track.road.len(), 3);
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
        let track = Track::parse(include_str!("testdata/oval.toml")).expect("овал");
        assert_eq!(track.car, "race");
        assert_eq!(track.road.len(), 10);
        assert_eq!(
            track
                .road
                .iter()
                .filter(|r| r.piece.name == "roadStart")
                .count(),
            1
        );
        assert!(!track.decor.is_empty());
    }

    fn chain(text: &str) -> Result<Vec<Checkpoint>, TrackError> {
        Track::parse(text).expect("трасса").chain()
    }

    /// Строка `cell = …` за строкой ошибки.
    fn cell_after(text: &str, line: usize) -> &str {
        text.lines().nth(line).expect("строка")
    }

    #[test]
    fn oval_chains_every_road_from_start_in_its_direction() {
        let oval = Track::parse(include_str!("testdata/oval.toml")).expect("овал");
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
            }
        );
        // Угол [0, 0] выводит к +Z из клетки [0, 1].
        assert_eq!(checkpoints[1].origin, vec3(2.0, 0.0, 8.0));
        assert_eq!(checkpoints[1].normal, vec3(0.0, 0.0, 1.0));
        for (c, next) in checkpoints.iter().zip(checkpoints.iter().cycle().skip(1)) {
            let road = &oval.road[next.road];
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
        let TrackError::Gap { line } = e else {
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
        let TrackError::Branch { line } = e else {
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
            TrackError::Level {
                line: 3,
                level: 0,
                found: 1
            }
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
            Err(TrackError::Gap { line: 13 })
        );
    }

    #[test]
    fn exactly_one_start() {
        let e = chain(&roads(&[("roadStraight", [0, 0], 0, 0)])).expect_err("без старта");
        assert_eq!(e, TrackError::Start { count: 0 });
        assert_eq!(e.to_string(), "нужен один roadStart, а их 0");
        assert_eq!(
            chain(&roads(&[
                ("roadStart", [0, 0], 0, 0),
                ("roadStart", [1, 0], 0, 0)
            ])),
            Err(TrackError::Start { count: 2 })
        );
    }

    #[test]
    fn figure_eight_passes_both_pieces_under_bridge_in_order() {
        let track = Track::parse(include_str!("testdata/bridge8.toml")).expect("восьмёрка");
        let checkpoints = track.chain().expect("цепочка");
        assert_eq!(
            checkpoints.iter().map(|c| c.road).collect::<Vec<_>>(),
            (0..track.road.len()).collect::<Vec<_>>()
        );
        let [under, over] = [3, 12].map(|i| &track.road[i]);
        assert_eq!((under.cell, under.level), (IVec2::ZERO, 0));
        assert_eq!(over.cell, IVec2::ZERO);
        assert_eq!(over.piece.name, "roadStraightBridgeMid");
        assert_eq!(checkpoints[3].origin, vec3(0.0, 0.0, 2.0));
        assert_eq!(checkpoints[12].origin, vec3(2.0, 2.0, 4.0));
    }

    #[test]
    fn road_cells_hold_every_level_of_piece() {
        let track = Track::parse(include_str!("testdata/bridge8.toml")).expect("восьмёрка");
        let cells = track.road_cells();
        let taken: usize = (track.road.iter())
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

    /// Выход клетки [0, 0] к +X: плоскость x = 4, z в 0..4, y в 0..2.
    fn exit_pos_x(level: u32) -> Checkpoint {
        Checkpoint::new(
            0,
            End {
                cell: IVec2::ZERO,
                side: crate::pieces::Side::PosX,
                level,
            },
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
}
