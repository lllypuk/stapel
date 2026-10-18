use std::f32::consts::FRAC_PI_2;
use std::fmt;

use engine::{IVec2, Mat4, Vec2, vec2, vec3};
use serde::Deserialize;

use crate::piece::{self, Piece};

/// Сторона клетки пути, м: концы всех деталей — на узлах сетки.
pub(crate) const CELL: f32 = 2.0;
/// Запас ведущего конца состава до тупика и стрелки: кузов выступает за тележку.
pub(crate) const MARGIN: f32 = 0.5;

/// Ребро и направление проезда: `true` — по возрастанию `s`.
pub(crate) type Pass = (usize, bool);

/// Поворот вокруг Y на четверти оборота, как у `Mat4::from_rotation_y`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rot(u8);

impl Rot {
    fn from_degrees(deg: i64) -> Option<Self> {
        if deg % 90 != 0 {
            return None;
        }
        u8::try_from((deg / 90).rem_euclid(4)).ok().map(Self)
    }

    pub(crate) fn radians(self) -> f32 {
        f32::from(self.0) * FRAC_PI_2
    }

    /// Без синусов: концы деталей остаются точно на узлах сетки, и стыки сравниваются без допуска.
    fn apply(self, v: Vec2) -> Vec2 {
        match self.0 {
            0 => v,
            1 => vec2(v.y, -v.x),
            2 => -v,
            _ => vec2(-v.y, v.x),
        }
    }
}

/// Деталь пути на карте; плоскость карты — (x, z) мира.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rail {
    pub(crate) piece: Piece,
    /// Клетка начала детали.
    pub(crate) cell: IVec2,
    pub(crate) rot: Rot,
    pub(crate) mirror: bool,
}

impl Rail {
    fn origin(&self) -> Vec2 {
        self.cell.as_vec2() * CELL
    }

    /// Точка оси и касательная по ходу `s`.
    pub(crate) fn point_at(&self, s: f32) -> (Vec2, Vec2) {
        let (p, d) = self.piece.point_at(self.mirror, s);
        (self.origin() + self.rot.apply(p), self.rot.apply(d))
    }

    /// Концы `s = 0` и `s = len` и направления от них внутрь детали.
    fn ends(&self) -> [(Vec2, Vec2); 2] {
        let (end, exit) = self.piece.end(self.mirror);
        [
            (self.origin(), self.rot.apply(Vec2::Y)),
            (self.origin() + self.rot.apply(end), -self.rot.apply(exit)),
        ]
    }

    /// Модель → мир.
    pub(crate) fn transform(&self) -> Mat4 {
        let o = self.origin();
        Mat4::from_translation(vec3(o.x, 0.0, o.y))
            * Mat4::from_rotation_y(self.rot.radians())
            * piece::transform(self.mirror)
    }
}

/// Ребро графа — деталь `[[rail]]` с тем же индексом.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Edge {
    pub(crate) rail: Rail,
    /// Порт и его сторона у концов `s = 0` и `s = len`.
    pub(crate) ends: [(usize, usize); 2],
}

impl Edge {
    pub(crate) fn len(&self) -> f32 {
        self.rail.piece.len()
    }
}

/// Стык: концы деталей в одной точке.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Port {
    pub(crate) pos: Vec2,
    /// Направление внутрь деталей стороны 0; у стороны 1 — обратное.
    pub(crate) dir: Vec2,
    /// Рёбра по сторонам.
    pub(crate) sides: [Vec<usize>; 2],
}

/// Стык с двумя деталями на стороне `side`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Switch {
    pub(crate) port: usize,
    pub(crate) side: usize,
    pub(crate) branches: [usize; 2],
    /// Индекс в `branches`.
    pub(crate) active: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Road {
    /// Имя из `MODELS`.
    pub(crate) tile: &'static str,
    pub(crate) cell: IVec2,
    pub(crate) rot: Rot,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Building {
    /// Имя из `MODELS`.
    pub(crate) model: &'static str,
    /// (x, z), м.
    pub(crate) pos: Vec2,
    /// Градусы.
    pub(crate) rot: f32,
    pub(crate) scale: f32,
}

/// Запись карты: индекс в своём списке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Rail(usize),
    Road(usize),
    Building(usize),
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rail(i) => write!(f, "rail[{i}]"),
            Self::Road(i) => write!(f, "road[{i}]"),
            Self::Building(i) => write!(f, "building[{i}]"),
        }
    }
}

/// Ошибка карты; `rail` — индекс в `[[rail]]`.
#[derive(Debug, PartialEq)]
pub(crate) enum TrackError {
    Toml(String),
    Unknown { entry: Entry, name: String },
    Rot { entry: Entry, rot: i64 },
    Angle { rail: usize },
    Crossing { rail: usize },
    Fan { rail: usize },
    NoDepot { rail: usize },
    DepotNotDeadEnd { rail: usize },
    DepotShort { rail: usize, len: f32, need: f32 },
}

impl fmt::Display for TrackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(msg) => f.write_str(msg),
            Self::Unknown { entry, name } => write!(f, "{entry}: неизвестная модель {name:?}"),
            Self::Rot { entry, rot } => write!(f, "{entry}: rot {rot} не кратен 90"),
            Self::Angle { rail } => write!(f, "rail[{rail}]: стык с другой деталью под углом"),
            Self::Crossing { rail } => {
                write!(f, "rail[{rail}]: по обе стороны стыка больше одной детали")
            }
            Self::Fan { rail } => {
                write!(f, "rail[{rail}]: с одной стороны стыка больше двух деталей")
            }
            Self::NoDepot { rail } => write!(f, "depot: нет записи rail[{rail}]"),
            Self::DepotNotDeadEnd { rail } => write!(f, "depot: rail[{rail}] не тупик"),
            Self::DepotShort { rail, len, need } => write!(
                f,
                "depot: путь от rail[{rail}] до стрелки {len:.2} м, нужно {need:.2}"
            ),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapFile {
    depot: DepotFile,
    #[serde(default)]
    rail: Vec<RailFile>,
    #[serde(default)]
    road: Vec<RoadFile>,
    #[serde(default)]
    building: Vec<BuildingFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DepotFile {
    rail: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RailFile {
    piece: String,
    cell: [i32; 2],
    #[serde(default)]
    rot: i64,
    #[serde(default)]
    mirror: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoadFile {
    tile: String,
    cell: [i32; 2],
    #[serde(default)]
    rot: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildingFile {
    model: String,
    pos: [f32; 2],
    #[serde(default)]
    rot: f32,
    #[serde(default = "one")]
    scale: f32,
}

fn one() -> f32 {
    1.0
}

/// Путь и окружение карты.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Track {
    pub(crate) edges: Vec<Edge>,
    pub(crate) ports: Vec<Port>,
    pub(crate) switches: Vec<Switch>,
    /// От тупика депо до первой стрелки.
    pub(crate) depot: Vec<Pass>,
    pub(crate) roads: Vec<Road>,
    pub(crate) buildings: Vec<Building>,
}

impl Track {
    /// `max_len` — длина самого длинного состава: путь депо его вмещает.
    pub(crate) fn parse(text: &str, max_len: f32) -> Result<Self, TrackError> {
        let file: MapFile = toml::from_str(text).map_err(|e| TrackError::Toml(e.to_string()))?;
        let rails = (file.rail.iter().enumerate())
            .map(|(i, r)| {
                let entry = Entry::Rail(i);
                Ok(Rail {
                    piece: Piece::ALL
                        .into_iter()
                        .find(|p| p.model().strip_prefix("train/railroad-") == Some(&r.piece))
                        .ok_or_else(|| unknown(entry, &r.piece))?,
                    cell: IVec2::from(r.cell),
                    rot: rot(entry, r.rot)?,
                    mirror: r.mirror,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let roads = (file.road.iter().enumerate())
            .map(|(i, r)| {
                let entry = Entry::Road(i);
                Ok(Road {
                    tile: model("road", &r.tile).ok_or_else(|| unknown(entry, &r.tile))?,
                    cell: IVec2::from(r.cell),
                    rot: rot(entry, r.rot)?,
                })
            })
            .collect::<Result<_, _>>()?;
        let buildings = (file.building.iter().enumerate())
            .map(|(i, b)| {
                Ok(Building {
                    model: model("city", &b.model)
                        .ok_or_else(|| unknown(Entry::Building(i), &b.model))?,
                    pos: Vec2::from(b.pos),
                    rot: b.rot,
                    scale: b.scale,
                })
            })
            .collect::<Result<_, _>>()?;
        let (edges, ports) = graph(rails)?;
        let switches = (ports.iter().enumerate())
            .flat_map(|(port, p)| {
                (0..2).filter_map(move |side| match p.sides[side][..] {
                    [a, b] => Some(Switch {
                        port,
                        side,
                        branches: [a, b],
                        active: 0,
                    }),
                    _ => None,
                })
            })
            .collect();
        let mut track = Self {
            edges,
            ports,
            switches,
            depot: Vec::new(),
            roads,
            buildings,
        };
        track.depot = track.depot_path(file.depot.rail, max_len)?;
        Ok(track)
    }

    /// Проезд за выходом `pass`; `None` — тупик или пошёрстная стрелка, переведённая на другую ветку.
    pub(crate) fn next(&self, (edge, forward): Pass) -> Option<Pass> {
        let (port, side) = self.edges[edge].ends[usize::from(forward)];
        let other = &self.ports[port].sides[1 - side];
        let to = match self.switches.iter().find(|s| s.port == port) {
            Some(s) if s.side == side => {
                if s.branches[s.active] != edge {
                    return None;
                }
                *other.first()?
            }
            Some(s) => s.branches[s.active],
            None => *other.first()?,
        };
        Some((to, self.edges[to].ends[0].0 == port))
    }

    fn depot_path(&self, rail: usize, max_len: f32) -> Result<Vec<Pass>, TrackError> {
        let edge = self.edges.get(rail).ok_or(TrackError::NoDepot { rail })?;
        let start = (0..2)
            .find(|&end| {
                let (port, side) = edge.ends[end];
                self.ports[port].sides[1 - side].is_empty()
            })
            .ok_or(TrackError::DepotNotDeadEnd { rail })?;
        let mut path = vec![(rail, start == 0)];
        for _ in 1..self.edges.len() {
            let last = path[path.len() - 1];
            let port = self.edges[last.0].ends[usize::from(last.1)].0;
            if self.switches.iter().any(|s| s.port == port) {
                break;
            }
            let Some(pass) = self.next(last) else { break };
            path.push(pass);
        }
        let len = path.iter().map(|&(e, _)| self.edges[e].len()).sum();
        let need = max_len + MARGIN;
        if len < need {
            return Err(TrackError::DepotShort { rail, len, need });
        }
        Ok(path)
    }
}

fn unknown(entry: Entry, name: &str) -> TrackError {
    TrackError::Unknown {
        entry,
        name: name.to_owned(),
    }
}

fn rot(entry: Entry, deg: i64) -> Result<Rot, TrackError> {
    Rot::from_degrees(deg).ok_or(TrackError::Rot { entry, rot: deg })
}

/// Имя из `MODELS` в подкаталоге `set`.
fn model(set: &str, name: &str) -> Option<&'static str> {
    crate::MODELS
        .iter()
        .copied()
        .find(|m| m.strip_prefix(set).and_then(|m| m.strip_prefix('/')) == Some(name))
}

/// Стыки по точному совпадению точки и касательной концов.
fn graph(rails: Vec<Rail>) -> Result<(Vec<Edge>, Vec<Port>), TrackError> {
    let mut ports: Vec<Port> = Vec::new();
    let mut edges = Vec::with_capacity(rails.len());
    for (i, rail) in rails.into_iter().enumerate() {
        let mut ends = [(0, 0); 2];
        for (end, (pos, into)) in ends.iter_mut().zip(rail.ends()) {
            let port = ports.iter().position(|p| p.pos == pos).unwrap_or_else(|| {
                ports.push(Port {
                    pos,
                    dir: into,
                    sides: [Vec::new(), Vec::new()],
                });
                ports.len() - 1
            });
            let p = &mut ports[port];
            let side = [p.dir, -p.dir]
                .iter()
                .position(|&d| d == into)
                .ok_or(TrackError::Angle { rail: i })?;
            p.sides[side].push(i);
            let [a, b] = p.sides.each_ref().map(Vec::len);
            if a > 2 || b > 2 {
                return Err(TrackError::Fan { rail: i });
            }
            if a > 1 && b > 1 {
                return Err(TrackError::Crossing { rail: i });
            }
            *end = (port, side);
        }
        edges.push(Edge { rail, ends });
    }
    Ok((edges, ports))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPOT0: &str = "depot = { rail = 0 }\n";

    /// Деталь, клетка, поворот, зеркало.
    type RailRow = (&'static str, [i32; 2], i64, bool);

    fn rail(piece: &str, cell: [i32; 2], rot: i64, mirror: bool) -> String {
        format!(
            "[[rail]]\npiece = \"{piece}\"\ncell = [{}, {}]\nrot = {rot}\nmirror = {mirror}\n",
            cell[0], cell[1]
        )
    }

    fn parse(rails: &[RailRow]) -> Result<Track, TrackError> {
        parse_with(DEPOT0, rails, 0.0)
    }

    fn parse_with(head: &str, rails: &[RailRow], max_len: f32) -> Result<Track, TrackError> {
        let text: String = rails.iter().map(|&(p, c, r, m)| rail(p, c, r, m)).collect();
        Track::parse(&format!("{head}{text}"), max_len)
    }

    fn ok(rails: &[RailRow]) -> Track {
        parse(rails).expect("карта")
    }

    /// Тупик (0, −8) → (0, −4) → стрелка (0, 0): прямая и S-деталь на (0, 4) и (−2, 4).
    const FORK: [RailRow; 4] = [
        ("straight", [0, -4], 0, false),
        ("straight", [0, 0], 180, false),
        ("straight", [0, 0], 0, false),
        ("curve", [0, 0], 0, false),
    ];

    #[test]
    fn rot_takes_quarter_turns_of_any_sign() {
        assert_eq!(Rot::from_degrees(0), Some(Rot(0)));
        assert_eq!(Rot::from_degrees(-90), Some(Rot(3)));
        assert_eq!(Rot::from_degrees(450), Some(Rot(1)));
        assert_eq!(Rot::from_degrees(180), Some(Rot(2)));
        assert_eq!(Rot::from_degrees(45), None);
        assert_eq!(Rot::from_degrees(-30), None);
    }

    #[test]
    fn rail_transform_follows_graph_axis() {
        for piece in Piece::ALL {
            for q in 0..4 {
                for mirror in [false, true] {
                    let rail = Rail {
                        piece,
                        cell: IVec2::new(3, -2),
                        rot: Rot(q),
                        mirror,
                    };
                    let m = rail.transform();
                    for s in [0.0, 0.7, piece.len() / 2.0, piece.len()] {
                        let (local, dir) = piece.point_at(false, s);
                        let (p, d) = rail.point_at(s);
                        let world = m.transform_point3(vec3(local.x, 0.0, local.y));
                        let world_d = m.transform_vector3(vec3(dir.x, 0.0, dir.y));
                        assert!(
                            vec2(world.x, world.z).distance(p) < 1e-4,
                            "{piece:?} {q} {mirror}: {world} {p}"
                        );
                        assert!(
                            vec2(world_d.x, world_d.z).distance(d) < 1e-4,
                            "{piece:?} {q} {mirror}: {world_d} {d}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn joints_join_by_point_and_tangent() {
        let t = ok(&[
            ("straight", [0, 0], 0, false),
            ("straight", [0, 4], 180, false),
            ("corner-small", [0, 4], 0, true),
            ("straight", [1, 5], 90, false),
            ("straight", [1, 0], 0, false),
        ]);
        assert_eq!(t.next((0, true)), Some((1, false)));
        assert_eq!(t.next((1, true)), Some((0, false)));
        assert_eq!(t.next((1, false)), Some((2, true)));
        assert_eq!(t.next((2, true)), Some((3, true)));
        assert_eq!(t.next((3, false)), Some((2, false)));
        assert_eq!(t.next((2, false)), Some((1, true)));
        assert_eq!(t.next((0, false)), None);
        assert_eq!(t.next((3, true)), None);
        assert_eq!(t.next((4, true)), None);
        assert_eq!(t.next((4, false)), None);
        assert_eq!(t.ports.len(), 7);
        assert_eq!(t.switches, []);
        assert_eq!(t.ports[t.edges[3].ends[1].0].pos, vec2(6.0, 10.0));
        for (i, e) in t.edges.iter().enumerate() {
            for ((port, side), (pos, into)) in e.ends.into_iter().zip(e.rail.ends()) {
                let p = &t.ports[port];
                assert_eq!(p.pos, pos);
                assert_eq!(if side == 0 { p.dir } else { -p.dir }, into);
                assert!(p.sides[side].contains(&i));
            }
        }
    }

    #[test]
    fn switch_is_recognized_and_follows_active_branch() {
        let mut t = ok(&FORK);
        let [s] = &t.switches[..] else {
            panic!("{:?}", t.switches)
        };
        assert_eq!(t.ports[s.port].pos, Vec2::ZERO);
        assert_eq!((s.branches, s.active), ([2, 3], 0));
        assert_eq!(t.ports[s.port].sides[s.side], [2, 3]);
        assert_eq!(t.next((1, false)), Some((2, true)));
        t.switches[0].active = 1;
        assert_eq!(t.next((1, false)), Some((3, true)));
        assert_eq!(t.next((3, true)), None);
        assert_eq!(t.ports[t.edges[3].ends[1].0].pos, vec2(-2.0, 4.0));
    }

    #[test]
    fn branch_to_branch_is_impossible() {
        let mut t = ok(&FORK);
        for active in 0..2 {
            t.switches[0].active = active;
            for branch in [2, 3] {
                let to = t.next((branch, false));
                let expected = (t.switches[0].branches[active] == branch).then_some((1, true));
                assert_eq!(to, expected, "активна {active}, с ветки {branch}");
            }
        }
    }

    #[test]
    fn depot_path_is_chain_to_first_switch() {
        let t = ok(&FORK);
        assert_eq!(t.depot, [(0, true), (1, false)]);
        let t = parse_with("depot = { rail = 2 }\n", &FORK, 0.0).expect("карта");
        assert_eq!(t.depot, [(2, false)]);
        let mut chain = FORK.to_vec();
        chain.push(("straight", [0, -4], 180, false));
        let t = parse_with("depot = { rail = 4 }\n", &chain, 0.0).expect("карта");
        assert_eq!(t.depot, [(4, false), (0, true), (1, false)]);
    }

    #[test]
    fn depot_path_without_switch_ends_at_dead_end() {
        let t = ok(&[
            ("straight", [0, 0], 0, false),
            ("straight", [0, 2], 0, false),
        ]);
        assert_eq!(t.depot, [(0, true), (1, true)]);
    }

    #[test]
    fn depot_holds_longest_consist_with_margin() {
        let at = |max_len| parse_with(DEPOT0, &FORK, max_len);
        assert!(at(8.0 - MARGIN).is_ok());
        assert_eq!(
            at(7.75),
            Err(TrackError::DepotShort {
                rail: 0,
                len: 8.0,
                need: 7.75 + MARGIN
            })
        );
    }

    #[test]
    fn depot_must_be_existing_dead_end() {
        assert_eq!(
            parse_with("depot = { rail = 1 }\n", &FORK, 0.0),
            Err(TrackError::DepotNotDeadEnd { rail: 1 })
        );
        assert_eq!(
            parse_with("depot = { rail = 4 }\n", &FORK, 0.0),
            Err(TrackError::NoDepot { rail: 4 })
        );
    }

    #[test]
    fn graph_errors_name_rail() {
        let cases: [(&[RailRow], TrackError); 3] = [
            (
                &[
                    ("straight", [0, 0], 0, false),
                    ("straight", [0, 2], 90, false),
                ],
                TrackError::Angle { rail: 1 },
            ),
            (
                &[
                    ("straight", [0, 0], 0, false),
                    ("curve", [1, 0], 0, false),
                    ("straight", [0, 2], 0, false),
                    ("curve", [0, 2], 0, true),
                    ("straight", [0, 4], 0, false),
                ],
                TrackError::Crossing { rail: 3 },
            ),
            (
                &[
                    ("straight", [0, 0], 0, false),
                    ("straight", [0, 2], 0, false),
                    ("curve", [0, 2], 0, false),
                    ("curve", [0, 2], 0, true),
                ],
                TrackError::Fan { rail: 3 },
            ),
        ];
        for (rails, error) in cases {
            assert_eq!(parse(rails), Err(error));
        }
    }

    #[test]
    fn names_and_rot_errors_name_entry() {
        let err = |text: &str| Track::parse(&format!("{DEPOT0}{text}"), 0.0).expect_err("ошибка");
        let straight = rail("straight", [0, 0], 0, false);
        assert_eq!(
            err(&format!("{straight}{}", rail("turnout", [0, 2], 0, false))),
            TrackError::Unknown {
                entry: Entry::Rail(1),
                name: "turnout".into()
            }
        );
        assert_eq!(
            err(&rail("straight", [0, 0], 45, false)),
            TrackError::Rot {
                entry: Entry::Rail(0),
                rot: 45
            }
        );
        let road = |tile: &str, rot: i64| {
            format!("[[road]]\ntile = \"{tile}\"\ncell = [0, 0]\nrot = {rot}\n")
        };
        assert_eq!(
            err(&format!(
                "{straight}{}{}",
                road("grass", 0),
                road("building-a", 0)
            )),
            TrackError::Unknown {
                entry: Entry::Road(1),
                name: "building-a".into()
            }
        );
        assert_eq!(
            err(&format!("{straight}{}", road("grass", 100))),
            TrackError::Rot {
                entry: Entry::Road(0),
                rot: 100
            }
        );
        assert_eq!(
            err(&format!(
                "{straight}[[building]]\nmodel = \"grass\"\npos = [0.0, 0.0]\n"
            )),
            TrackError::Unknown {
                entry: Entry::Building(0),
                name: "grass".into()
            }
        );
        assert!(matches!(err("[[rail]]\npiece = 1\n"), TrackError::Toml(_)));
        assert!(matches!(
            Track::parse(&straight, 0.0),
            Err(TrackError::Toml(_))
        ));
    }

    #[test]
    fn parses_fields_and_defaults() {
        let t = Track::parse(
            &format!(
                "{DEPOT0}{}[[rail]]\npiece = \"corner-large\"\ncell = [-1, 0]\n\
                 [[road]]\ntile = \"road-corner\"\ncell = [3, -5]\nrot = 270\n\
                 [[road]]\ntile = \"plaza\"\ncell = [1, 2]\n\
                 [[building]]\nmodel = \"building-skyscraper-a\"\npos = [1.5, -2.0]\nrot = 30.0\nscale = 1.5\n\
                 [[building]]\nmodel = \"building-c\"\npos = [4.0, 5.0]\n",
                rail("corner-small", [2, 3], -90, true)
            ),
            0.0,
        )
        .expect("карта");
        let rails: Vec<_> = t.edges.iter().map(|e| e.rail).collect();
        assert_eq!(
            rails,
            [
                Rail {
                    piece: Piece::CornerSmall,
                    cell: IVec2::new(2, 3),
                    rot: Rot(3),
                    mirror: true
                },
                Rail {
                    piece: Piece::CornerLarge,
                    cell: IVec2::new(-1, 0),
                    rot: Rot(0),
                    mirror: false
                },
            ]
        );
        assert_eq!(
            t.roads,
            [
                Road {
                    tile: "road/road-corner",
                    cell: IVec2::new(3, -5),
                    rot: Rot(3)
                },
                Road {
                    tile: "road/plaza",
                    cell: IVec2::new(1, 2),
                    rot: Rot(0)
                },
            ]
        );
        assert_eq!(
            t.buildings,
            [
                Building {
                    model: "city/building-skyscraper-a",
                    pos: vec2(1.5, -2.0),
                    rot: 30.0,
                    scale: 1.5
                },
                Building {
                    model: "city/building-c",
                    pos: vec2(4.0, 5.0),
                    rot: 0.0,
                    scale: 1.0
                },
            ]
        );
        assert_eq!(t.edges[1].len(), Piece::CornerLarge.len());
    }

    #[test]
    fn errors_print_entry() {
        let cases = [
            (TrackError::Toml("плохой TOML".into()), "плохой TOML"),
            (
                TrackError::Unknown {
                    entry: Entry::Building(2),
                    name: "x".into(),
                },
                "building[2]: неизвестная модель \"x\"",
            ),
            (
                TrackError::Rot {
                    entry: Entry::Road(1),
                    rot: 45,
                },
                "road[1]: rot 45 не кратен 90",
            ),
            (
                TrackError::Unknown {
                    entry: Entry::Rail(0),
                    name: "y".into(),
                },
                "rail[0]: неизвестная модель \"y\"",
            ),
            (
                TrackError::Angle { rail: 3 },
                "rail[3]: стык с другой деталью под углом",
            ),
            (
                TrackError::Crossing { rail: 4 },
                "rail[4]: по обе стороны стыка больше одной детали",
            ),
            (
                TrackError::Fan { rail: 5 },
                "rail[5]: с одной стороны стыка больше двух деталей",
            ),
            (TrackError::NoDepot { rail: 6 }, "depot: нет записи rail[6]"),
            (
                TrackError::DepotNotDeadEnd { rail: 7 },
                "depot: rail[7] не тупик",
            ),
            (
                TrackError::DepotShort {
                    rail: 8,
                    len: 4.0,
                    need: 21.5,
                },
                "depot: путь от rail[8] до стрелки 4.00 м, нужно 21.50",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }
}
