use std::ops::RangeInclusive;

use engine::{IVec2, Vec3, ivec2, vec3};

/// Метров в единице Racing Kit.
pub(crate) const KIT_SCALE: f32 = 4.0;
/// Метров в единице Car Kit.
pub(crate) const CAR_KIT_SCALE: f32 = 1.0;
/// Высота уровня в единицах набора: асфальт моста и выхода `roadRampLong`.
pub(crate) const LEVEL: f32 = 0.5;

/// Модели Car Kit, на которых ездят.
pub(crate) const CARS: [&str; 4] = ["race", "race-future", "sedan-sports", "hatchback-sports"];

/// Сторона клетки; +X и +Z — рост индексов клетки.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    PosX,
    NegX,
    PosZ,
    NegZ,
}

impl Side {
    pub(crate) fn step(self) -> IVec2 {
        match self {
            Self::PosX => ivec2(1, 0),
            Self::NegX => ivec2(-1, 0),
            Self::PosZ => ivec2(0, 1),
            Self::NegZ => ivec2(0, -1),
        }
    }

    pub(crate) fn opposite(self) -> Self {
        match self {
            Self::PosX => Self::NegX,
            Self::NegX => Self::PosX,
            Self::PosZ => Self::NegZ,
            Self::NegZ => Self::PosZ,
        }
    }

    pub(crate) fn rotated(self, rot: Rot) -> Self {
        (0..rot.0).fold(self, |side, _| match side {
            Self::PosZ => Self::PosX,
            Self::PosX => Self::NegZ,
            Self::NegZ => Self::NegX,
            Self::NegX => Self::PosZ,
        })
    }
}

/// Поворот детали вокруг +Y на четверти оборота против часовой при взгляде сверху: +Z → +X.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Rot(u8);

impl Rot {
    /// `None`, если угол не кратен 90°.
    pub(crate) fn from_degrees(deg: i64) -> Option<Self> {
        match deg.rem_euclid(360) {
            0 => Some(Self(0)),
            90 => Some(Self(1)),
            180 => Some(Self(2)),
            270 => Some(Self(3)),
            _ => None,
        }
    }

    pub(crate) fn radians(self) -> f32 {
        f32::from(self.0) * std::f32::consts::FRAC_PI_2
    }

    /// Поворот смещения клетки вокруг якорной клетки детали.
    pub(crate) fn cell(self, cell: IVec2) -> IVec2 {
        (0..self.0).fold(cell, |c, _| ivec2(c.y, -c.x))
    }
}

/// Стык: сторона клетки детали, через которую идёт дорога; `level` — над уровнем детали.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct End {
    pub(crate) cell: IVec2,
    pub(crate) side: Side,
    pub(crate) level: u32,
}

/// Дорожная деталь Racing Kit; все дороги твёрдые.
#[derive(Debug, PartialEq)]
pub(crate) struct Piece {
    pub(crate) name: &'static str,
    /// Клетки `[0, size)` при `rot = 0`.
    pub(crate) size: IVec2,
    /// Уровни, занятые в каждой клетке, над уровнем детали.
    pub(crate) levels: RangeInclusive<u32>,
    /// Сдвиг модели после корня GLB, в единицах набора: клетки — в `[0, size]`, асфальт — на высоте уровня.
    pub(crate) offset: Vec3,
    /// У рамп и старта `ends[1]` — выход.
    pub(crate) ends: [End; 2],
    /// Оси `barrierWall` вдоль боков приподнятой дороги, в клетках детали.
    pub(crate) walls: &'static [[Vec3; 2]],
}

impl Piece {
    pub(crate) fn cells(&self, rot: Rot) -> impl Iterator<Item = IVec2> {
        let size = self.size;
        (0..size.x).flat_map(move |x| (0..size.y).map(move |z| rot.cell(ivec2(x, z))))
    }

    pub(crate) fn ends(&self, rot: Rot) -> [End; 2] {
        self.ends.map(|end| End {
            cell: rot.cell(end.cell),
            side: end.side.rotated(rot),
            level: end.level,
        })
    }
}

/// Декор ставится свободно; твёрдый попадает в мир столкновений.
#[derive(Debug, PartialEq)]
pub(crate) struct DecorPiece {
    pub(crate) name: &'static str,
    /// Сдвиг модели после корня GLB: центр габарита по XZ — в начале координат, низ — на нуле.
    pub(crate) offset: Vec3,
    pub(crate) scale: f32,
    pub(crate) solid: bool,
}

const fn end(x: i32, z: i32, side: Side, level: u32) -> End {
    End {
        cell: ivec2(x, z),
        side,
        level,
    }
}

/// Внутренняя грань барьера толщиной 0,123 — на краю асфальта 0,155.
const WALL_X: f32 = 0.155 - 0.123 / 2.0;
/// Низ барьера — на внешней полосе, она на 0,01 ниже асфальта.
const WALL_Y: f32 = LEVEL - 0.01;

const BRIDGE_WALLS: [[Vec3; 2]; 2] = [
    [vec3(WALL_X, WALL_Y, 0.0), vec3(WALL_X, WALL_Y, 1.0)],
    [
        vec3(1.0 - WALL_X, WALL_Y, 0.0),
        vec3(1.0 - WALL_X, WALL_Y, 1.0),
    ],
];

/// Барьер подъёма `roadRampLong` от `z` до `z + 1`; подъём прямой, следующий уровень — у `z = 0`.
const fn ramp_wall(x: f32, z: f32) -> [Vec3; 2] {
    let drop = LEVEL / 2.0;
    [
        vec3(x, WALL_Y - drop * z, z),
        vec3(x, WALL_Y - drop * (z + 1.0), z + 1.0),
    ]
}

/// По отрезку на клетку: растянутый барьер растянул бы и полосы, и скосы на концах.
const RAMP_WALLS: [[Vec3; 2]; 4] = [
    ramp_wall(WALL_X, 0.0),
    ramp_wall(WALL_X, 1.0),
    ramp_wall(1.0 - WALL_X, 0.0),
    ramp_wall(1.0 - WALL_X, 1.0),
];

const fn straight(
    name: &'static str,
    levels: RangeInclusive<u32>,
    level: u32,
    walls: &'static [[Vec3; 2]],
) -> Piece {
    Piece {
        name,
        size: ivec2(1, 1),
        levels,
        offset: vec3(0.35, 0.0, 1.65),
        ends: [end(0, 0, Side::PosZ, level), end(0, 0, Side::NegZ, level)],
        walls,
    }
}

const fn corner(name: &'static str, n: i32) -> Piece {
    Piece {
        name,
        size: ivec2(n, n),
        levels: 0..=0,
        offset: vec3(0.35, 0.0, 0.65 + n as f32),
        ends: [end(0, n - 1, Side::PosZ, 0), end(n - 1, 0, Side::PosX, 0)],
        walls: &[],
    }
}

/// Асфальт стыков у всех деталей — x 0,155..0,845 клетки; мост и `roadRampLong` поднимают его на 0,5.
pub(crate) const PIECES: [Piece; 11] = [
    straight("roadStraight", 0..=0, 0, &[]),
    Piece {
        name: "roadStraightLong",
        size: ivec2(1, 2),
        levels: 0..=0,
        offset: vec3(0.35, 0.0, 2.65),
        ends: [end(0, 1, Side::PosZ, 0), end(0, 0, Side::NegZ, 0)],
        walls: &[],
    },
    corner("roadCornerSmall", 1),
    corner("roadCornerLarge", 2),
    corner("roadCornerLarger", 3),
    // Асфальт сдвинут на +0,13 по X; стойки арки выходят в соседние клетки на 0,13.
    Piece {
        name: "roadStart",
        size: ivec2(1, 2),
        levels: 0..=0,
        offset: vec3(0.22, 0.0, 2.65),
        ends: [end(0, 1, Side::PosZ, 0), end(0, 0, Side::NegZ, 0)],
        walls: &[],
    },
    Piece {
        name: "roadRampLong",
        size: ivec2(1, 2),
        levels: 0..=1,
        offset: vec3(0.35, 0.0, 2.65),
        ends: [end(0, 1, Side::PosZ, 0), end(0, 0, Side::NegZ, 1)],
        walls: &RAMP_WALLS,
    },
    // Опора у стыка +Z до земли.
    straight("roadStraightBridgeStart", 0..=1, 1, &BRIDGE_WALLS),
    // Только настил, низ на 0,38: под ним проходит дорога уровня 0.
    straight("roadStraightBridgeMid", 1..=1, 1, &BRIDGE_WALLS),
    // Опоры и земля под настилом.
    straight("roadStraightBridge", 0..=1, 1, &BRIDGE_WALLS),
    // Выход на 0,25 — трамплин: машина приземляется на уровень 0.
    straight("roadRamp", 0..=0, 0, &[]),
];

const fn decor(name: &'static str, offset: Vec3, solid: bool) -> DecorPiece {
    DecorPiece {
        name,
        offset,
        scale: KIT_SCALE,
        solid,
    }
}

pub(crate) const DECOR: [DecorPiece; 8] = [
    decor("barrierWall", vec3(-0.15, 0.01, 0.7115), true),
    decor("barrierRed", vec3(0.225, 0.01, 0.7115), true),
    decor("barrierWhite", vec3(0.225, 0.01, 0.7115), true),
    decor("rail", vec3(-0.15, 0.01, 0.625), false),
    decor("treeLarge", vec3(0.35, 0.01, 0.65), false),
    decor("treeSmall", vec3(0.35, 0.01, 0.65), false),
    decor("grandStand", vec3(-0.15, 0.01, 1.15), false),
    DecorPiece {
        name: "cone",
        offset: Vec3::ZERO,
        scale: CAR_KIT_SCALE,
        solid: false,
    },
];

/// Барьер вдоль боков приподнятой дороги.
pub(crate) const BARRIER_WALL: &DecorPiece = &DECOR[0];

pub(crate) fn piece(name: &str) -> Option<&'static Piece> {
    PIECES.iter().find(|p| p.name == name)
}

pub(crate) fn decor_piece(name: &str) -> Option<&'static DecorPiece> {
    DECOR.iter().find(|d| d.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROTS: [Rot; 4] = [Rot(0), Rot(1), Rot(2), Rot(3)];

    fn get(name: &str) -> &'static Piece {
        piece(name).unwrap_or_else(|| panic!("нет детали {name}"))
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn rot_from_degrees_accepts_multiples_of_90() {
        assert_eq!(Rot::from_degrees(0), Some(Rot(0)));
        assert_eq!(Rot::from_degrees(90), Some(Rot(1)));
        assert_eq!(Rot::from_degrees(180), Some(Rot(2)));
        assert_eq!(Rot::from_degrees(-90), Some(Rot(3)));
        assert_eq!(Rot::from_degrees(450), Some(Rot(1)));
        assert_eq!(Rot::from_degrees(45), None);
        assert_eq!(Rot::from_degrees(-30), None);
    }

    #[test]
    fn quarter_turn_takes_pos_z_to_pos_x() {
        assert_eq!(Rot(1).cell(ivec2(0, 1)), ivec2(1, 0));
        assert_eq!(Rot(1).cell(ivec2(1, 0)), ivec2(0, -1));
        assert_eq!(Rot(2).cell(ivec2(2, 1)), ivec2(-2, -1));
        assert_eq!(Rot(3).cell(ivec2(0, 1)), ivec2(-1, 0));
        for side in [Side::PosX, Side::NegX, Side::PosZ, Side::NegZ] {
            for rot in ROTS {
                assert_eq!(
                    side.rotated(rot).step(),
                    rot.cell(side.step()),
                    "{side:?} {rot:?}"
                );
                assert_eq!(side.opposite().step(), -side.step(), "{side:?}");
            }
        }
    }

    #[test]
    fn ends_turn_with_piece() {
        let large = get("roadCornerLarge");
        assert_eq!(
            large.ends(Rot(1)),
            [end(1, 0, Side::PosX, 0), end(0, -1, Side::NegZ, 0)]
        );
        assert_eq!(
            large.cells(Rot(1)).collect::<Vec<_>>(),
            [ivec2(0, 0), ivec2(1, 0), ivec2(0, -1), ivec2(1, -1)]
        );
        for piece in &PIECES {
            for rot in ROTS {
                let cells: Vec<_> = piece.cells(rot).collect();
                assert_eq!(i32::try_from(cells.len()), Ok(piece.size.x * piece.size.y));
                for e in piece.ends(rot) {
                    assert!(cells.contains(&e.cell), "{} {rot:?}", piece.name);
                    assert!(
                        !cells.contains(&(e.cell + e.side.step())),
                        "{} {rot:?}: стык внутрь детали",
                        piece.name
                    );
                }
            }
        }
    }

    #[test]
    fn ramp_ends_rise_but_jump_lands_on_level_0() {
        let ramp = get("roadRamp");
        assert_eq!(ramp.ends.map(|e| e.level), [0, 0]);
        assert_eq!(ramp.ends[1].side, Side::NegZ);
        let long = get("roadRampLong");
        assert_eq!(long.ends.map(|e| e.level), [0, 1]);
        assert_eq!(long.ends[1].side, Side::NegZ);
        assert_eq!(long.levels, 0..=1);
        for name in [
            "roadStraightBridgeStart",
            "roadStraightBridgeMid",
            "roadStraightBridge",
        ] {
            assert_eq!(get(name).ends.map(|e| e.level), [1, 1], "{name}");
        }
        assert_eq!(get("roadStraightBridgeMid").levels, 1..=1);
    }

    /// Габарит травы или опор по XZ после корня GLB: `[min_x, min_z, max_x, max_z]`.
    const FOOTPRINTS: [(&str, [f32; 4]); 11] = [
        ("roadStraight", [-0.35, -1.65, 0.65, -0.65]),
        ("roadStraightLong", [-0.35, -2.65, 0.65, -0.65]),
        ("roadCornerSmall", [-0.35, -1.65, 0.65, -0.65]),
        ("roadCornerLarge", [-0.35, -2.65, 1.65, -0.65]),
        ("roadCornerLarger", [-0.35, -3.65, 2.65, -0.65]),
        ("roadStart", [-0.22, -2.65, 0.78, -0.65]),
        ("roadRampLong", [-0.35, -2.65, 0.65, -0.65]),
        ("roadStraightBridgeStart", [-0.35, -1.65, 0.65, -0.65]),
        ("roadStraightBridgeMid", [-0.35, -1.65, 0.65, -0.65]),
        ("roadStraightBridge", [-0.35, -1.65, 0.65, -0.65]),
        ("roadRamp", [-0.35, -1.65, 0.65, -0.65]),
    ];

    #[test]
    fn offset_maps_measured_footprint_onto_cells() {
        assert_eq!(FOOTPRINTS.len(), PIECES.len());
        for (name, [x0, z0, x1, z1]) in FOOTPRINTS {
            let p = get(name);
            let size = p.size.as_vec2();
            assert!(
                close(x0 + p.offset.x, 0.0) && close(z0 + p.offset.z, 0.0),
                "{name}"
            );
            assert!(
                close(x1 + p.offset.x, size.x) && close(z1 + p.offset.z, size.y),
                "{name}"
            );
            assert!(close(p.offset.y, 0.0), "{name}");
        }
    }

    #[test]
    fn start_offset_aligns_its_asphalt_with_straight_end() {
        // Асфальт на стыках по X после корня GLB.
        let straight = [-0.195, 0.495].map(|x| x + get("roadStraight").offset.x);
        let start = [-0.065, 0.625].map(|x| x + get("roadStart").offset.x);
        assert!(close(straight[0], 0.155) && close(straight[1], 0.845));
        assert!(close(start[0], straight[0]) && close(start[1], straight[1]));
    }

    /// Габарит после корня GLB: `[min, max]`.
    const DECOR_BOUNDS: [(&str, [Vec3; 2]); 8] = [
        (
            "barrierWall",
            [vec3(-0.35, -0.01, -0.773), vec3(0.65, 0.121, -0.65)],
        ),
        (
            "barrierRed",
            [vec3(-0.35, -0.01, -0.773), vec3(-0.1, 0.121, -0.65)],
        ),
        (
            "barrierWhite",
            [vec3(-0.35, -0.01, -0.773), vec3(-0.1, 0.121, -0.65)],
        ),
        ("rail", [vec3(-0.35, -0.01, -0.65), vec3(0.65, 0.15, -0.6)]),
        (
            "treeLarge",
            [vec3(-0.528, -0.01, -0.856), vec3(-0.172, 1.497, -0.444)],
        ),
        (
            "treeSmall",
            [vec3(-0.477, -0.01, -0.796), vec3(-0.223, 1.06, -0.504)],
        ),
        (
            "grandStand",
            [vec3(-0.35, -0.01, -1.65), vec3(0.65, 0.886, -0.65)],
        ),
        (
            "cone",
            [vec3(-0.238, 0.0, -0.238), vec3(0.238, 0.595, 0.238)],
        ),
    ];

    #[test]
    fn decor_offset_centers_and_grounds_measured_bounds() {
        assert_eq!(DECOR_BOUNDS.len(), DECOR.len());
        for (name, [min, max]) in DECOR_BOUNDS {
            let d = decor_piece(name).unwrap_or_else(|| panic!("нет декора {name}"));
            let center = (min + max) / 2.0 + d.offset;
            assert!(close(center.x, 0.0) && close(center.z, 0.0), "{name}");
            assert!(close(min.y + d.offset.y, 0.0), "{name}");
            assert_eq!(d.solid, name.starts_with("barrier"), "{name}");
            let scale = if name == "cone" {
                CAR_KIT_SCALE
            } else {
                KIT_SCALE
            };
            assert!(close(d.scale, scale), "{name}");
        }
    }

    #[test]
    fn unknown_names_are_none() {
        assert!(piece("roadSplit").is_none());
        assert!(piece("cone").is_none());
        assert!(decor_piece("roadStraight").is_none());
        assert_eq!(BARRIER_WALL.name, "barrierWall");
    }

    #[test]
    fn only_raised_pieces_have_walls() {
        for p in &PIECES {
            assert_eq!(p.walls.is_empty(), *p.levels.end() == 0, "{}", p.name);
        }
    }
}
