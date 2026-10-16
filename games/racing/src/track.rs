use std::collections::HashSet;
use std::fmt;

use engine::{IVec2, Vec3};
use serde::Deserialize;
use toml::Spanned;

use crate::pieces::{DecorPiece, Piece, Rot, decor_piece, piece};

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
        }
    }
}

#[derive(Debug)]
pub(crate) struct Road {
    pub(crate) piece: &'static Piece,
    pub(crate) cell: IVec2,
    pub(crate) level: u32,
    pub(crate) rot: Rot,
}

#[derive(Debug)]
pub(crate) struct Decor {
    pub(crate) piece: &'static DecorPiece,
    /// В метрах.
    pub(crate) pos: Vec3,
    /// В радианах; в файле — градусы.
    pub(crate) yaw: f32,
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
            let cell = IVec2::from(r.cell);
            for c in piece.cells(rot) {
                for l in piece.levels.clone() {
                    let (cell, level) = (cell + c, r.level + l);
                    if !taken.insert((cell, level)) {
                        return Err(TrackError::Overlap {
                            line: line(&r.piece),
                            cell,
                            level,
                        });
                    }
                }
            }
            road.push(Road {
                piece,
                cell,
                level: r.level,
                rot,
            });
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
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use engine::{ivec2, vec3};

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
}
