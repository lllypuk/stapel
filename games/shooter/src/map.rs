use std::f32::consts::{FRAC_PI_2, PI};
use std::fmt;

use stapel::{IVec2, ivec2};

/// Ошибка разбора карты; `line` — с единицы.
#[derive(Debug, PartialEq)]
pub(crate) enum MapError {
    Ragged { line: usize },
    Unknown { line: usize, ch: char },
    NoPlayer,
    TwoPlayers { line: usize },
    NoExit,
    TwoExits { line: usize },
    DoorWithoutKey { line: usize },
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ragged { line } => write!(f, "строка {line}: длина не как у первой"),
            Self::Unknown { line, ch } => write!(f, "строка {line}: неизвестный символ {ch:?}"),
            Self::NoPlayer => write!(f, "нет P"),
            Self::TwoPlayers { line } => write!(f, "строка {line}: второй P"),
            Self::NoExit => write!(f, "нет X"),
            Self::TwoExits { line } => write!(f, "строка {line}: второй X"),
            Self::DoorWithoutKey { line } => write!(f, "строка {line}: дверь без ключа на карте"),
        }
    }
}

/// Сторона света; север — первая строка карты, −Z мира.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Dir {
    N,
    E,
    S,
    W,
}

impl Dir {
    /// Порядок выбора взгляда игрока.
    pub(crate) const ALL: [Self; 4] = [Self::N, Self::E, Self::S, Self::W];

    pub(crate) fn offset(self) -> IVec2 {
        match self {
            Self::N => ivec2(0, -1),
            Self::E => ivec2(1, 0),
            Self::S => ivec2(0, 1),
            Self::W => ivec2(-1, 0),
        }
    }

    /// Поворот вокруг +Y, переводящий −Z в эту сторону; он же `Camera3D::yaw` взгляда.
    pub(crate) fn yaw(self) -> f32 {
        match self {
            Self::N => 0.0,
            Self::E => -FRAC_PI_2,
            Self::S => PI,
            Self::W => FRAC_PI_2,
        }
    }
}

/// Что стоит в клетке; дверь, враги и предметы лежат на траве и хранятся списками.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Tile {
    Grass,
    Fence,
    Tree,
    Rock,
}

impl Tile {
    pub(crate) fn solid(self) -> bool {
        self != Self::Grass
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EnemyKind {
    Brawler,
    Gunner,
    Runner,
}

/// Уровень из ASCII-карты; клетка `(x, y)` — столбец и строка сверху.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Map {
    pub(crate) size: IVec2,
    /// По строкам сверху вниз.
    tiles: Vec<Tile>,
    pub(crate) player: IVec2,
    pub(crate) facing: Dir,
    pub(crate) exit: IVec2,
    pub(crate) doors: Vec<IVec2>,
    pub(crate) keys: Vec<IVec2>,
    pub(crate) medkits: Vec<IVec2>,
    pub(crate) ammo: Vec<IVec2>,
    pub(crate) enemies: Vec<(EnemyKind, IVec2)>,
}

impl Map {
    pub(crate) fn parse(text: &str) -> Result<Self, MapError> {
        let rows: Vec<Vec<char>> = text.lines().map(|l| l.chars().collect()).collect();
        let width = rows.first().map_or(0, Vec::len);
        let to_i32 = |n: usize| i32::try_from(n).expect("карта меньше 2³¹ клеток");
        let mut map = Self {
            size: ivec2(to_i32(width), to_i32(rows.len())),
            tiles: vec![Tile::Grass; width * rows.len()],
            player: IVec2::ZERO,
            facing: Dir::N,
            exit: IVec2::ZERO,
            doors: Vec::new(),
            keys: Vec::new(),
            medkits: Vec::new(),
            ammo: Vec::new(),
            enemies: Vec::new(),
        };
        let mut player = None;
        let mut exit = None;
        let mut first_door = None;
        for (i, row) in rows.iter().enumerate() {
            let line = i + 1;
            if row.len() != width {
                return Err(MapError::Ragged { line });
            }
            for (j, &ch) in row.iter().enumerate() {
                let cell = ivec2(to_i32(j), to_i32(i));
                let tile = &mut map.tiles[i * width + j];
                match ch {
                    '#' => *tile = Tile::Fence,
                    'T' => *tile = Tile::Tree,
                    'R' => *tile = Tile::Rock,
                    'P' if player.is_some() => return Err(MapError::TwoPlayers { line }),
                    'P' => player = Some(cell),
                    'X' if exit.is_some() => return Err(MapError::TwoExits { line }),
                    'X' => exit = Some(cell),
                    'D' => {
                        first_door.get_or_insert(line);
                        map.doors.push(cell);
                    }
                    'k' => map.keys.push(cell),
                    '+' => map.medkits.push(cell),
                    'a' => map.ammo.push(cell),
                    'm' => map.enemies.push((EnemyKind::Brawler, cell)),
                    's' => map.enemies.push((EnemyKind::Gunner, cell)),
                    'f' => map.enemies.push((EnemyKind::Runner, cell)),
                    '.' => {}
                    ch => return Err(MapError::Unknown { line, ch }),
                }
            }
        }
        map.player = player.ok_or(MapError::NoPlayer)?;
        map.exit = exit.ok_or(MapError::NoExit)?;
        if let Some(line) = first_door.filter(|_| map.keys.is_empty()) {
            return Err(MapError::DoorWithoutKey { line });
        }
        map.facing = Dir::ALL
            .into_iter()
            .find(|d| map.open(map.player + d.offset()))
            .unwrap_or(Dir::N);
        Ok(map)
    }

    /// `None` вне карты.
    pub(crate) fn tile(&self, cell: IVec2) -> Option<Tile> {
        let inside = cell.cmpge(IVec2::ZERO).all() && cell.cmplt(self.size).all();
        inside.then(|| self.tiles[to_index(cell, self.size.x)])
    }

    /// Трава без двери: закрытая на старте дверь взгляду игрока не годится.
    fn open(&self, cell: IVec2) -> bool {
        self.tile(cell) == Some(Tile::Grass) && !self.doors.contains(&cell)
    }

    pub(crate) fn cells(&self) -> impl Iterator<Item = IVec2> + '_ {
        (0..self.size.y).flat_map(move |y| (0..self.size.x).map(move |x| ivec2(x, y)))
    }
}

#[expect(
    clippy::cast_sign_loss,
    reason = "клетка внутри карты, координаты неотрицательны"
)]
fn to_index(cell: IVec2, width: i32) -> usize {
    (cell.y * width + cell.x) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LEVEL01;
    use EnemyKind::{Brawler, Gunner, Runner};
    use Tile::{Fence, Grass, Rock, Tree};

    fn parse(text: &str) -> Map {
        Map::parse(text).unwrap()
    }

    #[test]
    fn first_line_is_north_and_obstacles_are_solid_tiles() {
        let m = parse("#TR\n.PX\n");
        assert_eq!(
            (m.size, m.player, m.exit),
            (ivec2(3, 2), ivec2(1, 1), ivec2(2, 1))
        );
        assert_eq!(m.tiles, [Fence, Tree, Rock, Grass, Grass, Grass]);
        assert_eq!(m.tile(ivec2(1, 0)), Some(Tree));
        assert_eq!(m.tile(ivec2(0, 1)), Some(Grass));
        assert!(Fence.solid() && Tree.solid() && Rock.solid() && !Grass.solid());
    }

    #[test]
    fn outside_map_has_no_tile() {
        let m = parse("###\n#PX\n");
        for cell in [ivec2(-1, 0), ivec2(3, 0), ivec2(0, -1), ivec2(0, 2)] {
            assert_eq!(m.tile(cell), None, "{cell}");
        }
    }

    #[test]
    fn items_and_enemies_lie_on_grass_in_order_from_top() {
        let m = parse("kDm+\nsaf.\nPX.f\n");
        assert_eq!(m.keys, [ivec2(0, 0)]);
        assert_eq!(m.doors, [ivec2(1, 0)]);
        assert_eq!(m.medkits, [ivec2(3, 0)]);
        assert_eq!(m.ammo, [ivec2(1, 1)]);
        assert_eq!(
            m.enemies,
            [
                (Brawler, ivec2(2, 0)),
                (Gunner, ivec2(0, 1)),
                (Runner, ivec2(2, 1)),
                (Runner, ivec2(3, 2)),
            ]
        );
        assert!(m.cells().all(|c| m.tile(c) == Some(Grass)));
    }

    #[test]
    fn cells_go_row_by_row() {
        let m = parse("PX\n..\n");
        let cells: Vec<_> = m.cells().collect();
        assert_eq!(cells, [ivec2(0, 0), ivec2(1, 0), ivec2(0, 1), ivec2(1, 1)]);
    }

    #[test]
    fn player_faces_first_open_neighbour_in_order_n_e_s_w() {
        assert_eq!(parse(".X\nP.\n").facing, Dir::N);
        assert_eq!(parse("TX\nP.\n").facing, Dir::E);
        assert_eq!(parse("#X\nP#\n.#\n").facing, Dir::S);
        assert_eq!(parse("XR\n.P\n").facing, Dir::W);
        assert_eq!(parse("PX\n").facing, Dir::E);
        assert_eq!(parse("P\nX\n").facing, Dir::S);
    }

    #[test]
    fn player_skips_closed_door_and_items_count_as_open() {
        assert_eq!(parse("DXk\nP..\n").facing, Dir::E);
        assert_eq!(parse("kX\nP#\n").facing, Dir::N);
        assert_eq!(parse("Rk\nPD\n.X\n").facing, Dir::S);
        assert_eq!(
            parse("RX\nPD\n#k\n").facing,
            Dir::N,
            "некуда смотреть — север"
        );
    }

    #[test]
    fn dir_yaw_looks_along_offset() {
        let yaw_forward = |d: Dir| (-d.yaw().sin(), -d.yaw().cos());
        for d in Dir::ALL {
            let o = d.offset().as_vec2();
            let (x, z) = yaw_forward(d);
            assert!((x - o.x).abs() < 1e-6 && (z - o.y).abs() < 1e-6, "{d:?}");
        }
    }

    #[test]
    fn parse_errors_name_the_line() {
        assert_eq!(Map::parse("PX.\n..\n"), Err(MapError::Ragged { line: 2 }));
        assert_eq!(
            Map::parse("PX.\n.?.\n"),
            Err(MapError::Unknown { line: 2, ch: '?' })
        );
        assert_eq!(
            Map::parse(" P\n"),
            Err(MapError::Unknown { line: 1, ch: ' ' })
        );
        assert_eq!(Map::parse("..X\n###\n"), Err(MapError::NoPlayer));
        assert_eq!(Map::parse(""), Err(MapError::NoPlayer));
        assert_eq!(
            Map::parse("PX.\n..P\n"),
            Err(MapError::TwoPlayers { line: 2 })
        );
        assert_eq!(Map::parse("P..\n###\n"), Err(MapError::NoExit));
        assert_eq!(
            Map::parse("PX.\n...\n.X.\n"),
            Err(MapError::TwoExits { line: 3 })
        );
        assert_eq!(
            Map::parse("PX.\n.D.\n..D\n"),
            Err(MapError::DoorWithoutKey { line: 2 })
        );
    }

    #[test]
    fn doors_need_any_key_and_keys_need_no_door() {
        assert_eq!(parse("PXD\n.Dk\n").doors.len(), 2);
        assert_eq!(parse("PXk\n").keys.len(), 1);
    }

    #[test]
    fn errors_print_line_and_cause() {
        let msg = |text| Map::parse(text).unwrap_err().to_string();
        assert_eq!(msg("PX.\n..\n"), "строка 2: длина не как у первой");
        assert_eq!(msg("PX.\n.?.\n"), "строка 2: неизвестный символ '?'");
        assert_eq!(msg(""), "нет P");
        assert_eq!(msg("P\nP\n"), "строка 2: второй P");
        assert_eq!(msg("P\n"), "нет X");
        assert_eq!(msg("PX\nX.\n"), "строка 2: второй X");
        assert_eq!(msg("PX\nD.\n"), "строка 2: дверь без ключа на карте");
    }

    /// Клетки, достижимые от `P` по траве; двери проходимы при `through_doors`.
    fn reachable(m: &Map, through_doors: bool) -> Vec<IVec2> {
        let mut seen = vec![m.player];
        let mut i = 0;
        while let Some(&cell) = seen.get(i) {
            i += 1;
            for d in Dir::ALL {
                let next = cell + d.offset();
                let open = through_doors || !m.doors.contains(&next);
                if m.tile(next) == Some(Grass) && open && !seen.contains(&next) {
                    seen.push(next);
                }
            }
        }
        seen
    }

    #[test]
    fn level01_has_every_kind_and_closed_border() {
        let m = parse(LEVEL01);
        for kind in [Brawler, Gunner, Runner] {
            assert!(m.enemies.iter().any(|&(k, _)| k == kind), "{kind:?}");
        }
        assert!(!m.medkits.is_empty() && !m.ammo.is_empty());
        assert_eq!((m.keys.len(), m.doors.len()), (1, 1));
        let border = |c: IVec2| c.x == 0 || c.y == 0 || c.x == m.size.x - 1 || c.y == m.size.y - 1;
        for c in m.cells().filter(|&c| border(c)) {
            assert_eq!(m.tile(c), Some(Fence), "{c}");
        }
    }

    #[test]
    fn level01_key_before_door_and_exit_behind_it() {
        let m = parse(LEVEL01);
        let closed = reachable(&m, false);
        assert!(closed.contains(&m.keys[0]) && !closed.contains(&m.exit));
        for c in m.medkits.iter().chain(&m.ammo) {
            assert!(reachable(&m, true).contains(c), "{c}");
        }
        assert!(m.medkits.iter().any(|c| closed.contains(c)));
        assert!(m.ammo.iter().any(|c| closed.contains(c)));
        let open = reachable(&m, true);
        let grass = m.cells().filter(|&c| m.tile(c) == Some(Grass)).count();
        assert_eq!(open.len(), grass, "вся трава связана");
        let door = m.doors[0];
        let fence = |d: Dir| m.tile(door + d.offset()) == Some(Fence);
        assert!(fence(Dir::E) && fence(Dir::W), "дверь в заборе");
    }
}
