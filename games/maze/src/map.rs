use std::f32::consts::{FRAC_PI_2, PI};
use std::fmt;

use engine::{IVec2, MAX_LIGHTS, Vec3, ivec2, vec3};

/// Ошибка разбора карты; `line` — с единицы.
#[derive(Debug, PartialEq)]
pub(crate) enum MapError {
    Ragged { line: usize },
    Unknown { line: usize, ch: char },
    NoPlayer,
    TwoPlayers { line: usize },
    TooManyTorches { line: usize },
    TorchWithoutWall { line: usize },
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ragged { line } => write!(f, "строка {line}: длина не как у первой"),
            Self::Unknown { line, ch } => write!(f, "строка {line}: неизвестный символ {ch:?}"),
            Self::NoPlayer => write!(f, "нет P"),
            Self::TwoPlayers { line } => write!(f, "строка {line}: второй P"),
            Self::TooManyTorches { line } => {
                write!(f, "строка {line}: факелов больше {MAX_LIGHTS}")
            }
            Self::TorchWithoutWall { line } => write!(f, "строка {line}: T без соседней стены"),
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
    /// Порядок выбора взгляда игрока и стены факела.
    const ALL: [Self; 4] = [Self::N, Self::E, Self::S, Self::W];

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

/// Факел в клетке `cell`, баннер — на стене в стороне `wall`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Torch {
    pub(crate) cell: IVec2,
    pub(crate) wall: Dir,
}

/// Лабиринт из ASCII-карты; клетка `(x, y)` — столбец и строка сверху.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Map {
    pub(crate) size: IVec2,
    /// По строкам сверху вниз.
    walls: Vec<bool>,
    pub(crate) player: IVec2,
    pub(crate) facing: Dir,
    pub(crate) orcs: Vec<IVec2>,
    pub(crate) coins: Vec<IVec2>,
    pub(crate) keys: Vec<IVec2>,
    pub(crate) gates: Vec<IVec2>,
    pub(crate) exits: Vec<IVec2>,
    pub(crate) torches: Vec<Torch>,
}

impl Map {
    pub(crate) fn parse(text: &str) -> Result<Self, MapError> {
        let rows: Vec<Vec<char>> = text.lines().map(|l| l.chars().collect()).collect();
        let width = rows.first().map_or(0, Vec::len);
        let to_i32 = |n: usize| i32::try_from(n).expect("карта меньше 2³¹ клеток");
        let mut map = Self {
            size: ivec2(to_i32(width), to_i32(rows.len())),
            walls: vec![false; width * rows.len()],
            player: IVec2::ZERO,
            facing: Dir::N,
            orcs: Vec::new(),
            coins: Vec::new(),
            keys: Vec::new(),
            gates: Vec::new(),
            exits: Vec::new(),
            torches: Vec::new(),
        };
        let mut player = None;
        let mut torch_lines = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            let line = i + 1;
            if row.len() != width {
                return Err(MapError::Ragged { line });
            }
            for (j, &ch) in row.iter().enumerate() {
                let cell = ivec2(to_i32(j), to_i32(i));
                match ch {
                    '#' => map.walls[i * width + j] = true,
                    'P' if player.is_some() => return Err(MapError::TwoPlayers { line }),
                    'P' => player = Some(cell),
                    'E' => map.orcs.push(cell),
                    'o' => map.coins.push(cell),
                    'k' => map.keys.push(cell),
                    'D' => map.gates.push(cell),
                    'X' => map.exits.push(cell),
                    'T' if torch_lines.len() == MAX_LIGHTS => {
                        return Err(MapError::TooManyTorches { line });
                    }
                    'T' => torch_lines.push((cell, line)),
                    '.' => {}
                    ch => return Err(MapError::Unknown { line, ch }),
                }
            }
        }
        // Стены всей карты известны только после обхода: соседняя может быть строкой ниже.
        for (cell, line) in torch_lines {
            let wall = Dir::ALL
                .into_iter()
                .find(|d| map.wall(cell + d.offset()))
                .ok_or(MapError::TorchWithoutWall { line })?;
            map.torches.push(Torch { cell, wall });
        }
        map.player = player.ok_or(MapError::NoPlayer)?;
        map.facing = Dir::ALL
            .into_iter()
            .find(|d| map.floor(map.player + d.offset()))
            .unwrap_or(Dir::N);
        Ok(map)
    }

    /// Вне карты стены нет.
    pub(crate) fn wall(&self, cell: IVec2) -> bool {
        self.inside(cell) && self.walls[to_index(cell, self.size.x)]
    }

    /// Клетка внутри карты без стены.
    pub(crate) fn floor(&self, cell: IVec2) -> bool {
        self.inside(cell) && !self.walls[to_index(cell, self.size.x)]
    }

    fn inside(&self, cell: IVec2) -> bool {
        cell.cmpge(IVec2::ZERO).all() && cell.cmplt(self.size).all()
    }

    pub(crate) fn cells(&self) -> impl Iterator<Item = IVec2> + '_ {
        (0..self.size.y).flat_map(move |y| (0..self.size.x).map(move |x| ivec2(x, y)))
    }
}

/// Центр клетки на полу: клетка — 1 м, столбцы по +X, строки по +Z.
pub(crate) fn cell_center(cell: IVec2) -> Vec3 {
    vec3(cell.x as f32 + 0.5, 0.0, cell.y as f32 + 0.5)
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

    fn parse(text: &str) -> Map {
        Map::parse(text).unwrap()
    }

    #[test]
    fn first_line_is_north_and_hash_is_wall() {
        let m = parse("#..\n.P#\n");
        assert_eq!((m.size, m.player), (ivec2(3, 2), ivec2(1, 1)));
        assert_eq!(m.walls, [true, false, false, false, false, true]);
        assert!(m.wall(ivec2(0, 0)) && m.wall(ivec2(2, 1)));
        assert!(!m.wall(ivec2(1, 1)) && m.floor(ivec2(1, 1)));
        assert!(!m.floor(ivec2(0, 0)));
    }

    #[test]
    fn outside_map_is_neither_wall_nor_floor() {
        let m = parse("###\n#P#\n");
        for cell in [ivec2(-1, 0), ivec2(3, 0), ivec2(0, -1), ivec2(0, 2)] {
            assert!(!m.wall(cell) && !m.floor(cell), "{cell}");
        }
    }

    #[test]
    fn spawns_are_cells_from_top() {
        let m = parse("Eo.#\nkDXP\n");
        assert_eq!(m.orcs, [ivec2(0, 0)]);
        assert_eq!(m.coins, [ivec2(1, 0)]);
        assert_eq!(m.keys, [ivec2(0, 1)]);
        assert_eq!(m.gates, [ivec2(1, 1)]);
        assert_eq!(m.exits, [ivec2(2, 1)]);
        assert_eq!(m.player, ivec2(3, 1));
    }

    #[test]
    fn cells_go_row_by_row() {
        let m = parse("P.\n..\n");
        let cells: Vec<_> = m.cells().collect();
        assert_eq!(cells, [ivec2(0, 0), ivec2(1, 0), ivec2(0, 1), ivec2(1, 1)]);
    }

    #[test]
    fn cell_center_is_middle_of_metre_on_floor() {
        assert_eq!(cell_center(ivec2(0, 0)), vec3(0.5, 0.0, 0.5));
        assert_eq!(cell_center(ivec2(3, 2)), vec3(3.5, 0.0, 2.5));
    }

    #[test]
    fn player_faces_first_floor_neighbour_in_order_n_e_s_w() {
        assert_eq!(parse(".\nP\n").facing, Dir::N);
        assert_eq!(parse("#.\nP.\n").facing, Dir::E);
        assert_eq!(parse("##\nP#\n.#\n").facing, Dir::S);
        assert_eq!(parse("##\n.P\n").facing, Dir::W);
        assert_eq!(parse("P\n").facing, Dir::N);
    }

    #[test]
    fn dir_yaw_looks_along_offset() {
        let yaw_forward = |d: Dir| vec3(-d.yaw().sin(), 0.0, -d.yaw().cos());
        for d in Dir::ALL {
            let o = d.offset().as_vec2();
            assert!(
                yaw_forward(d).abs_diff_eq(vec3(o.x, 0.0, o.y), 1e-6),
                "{d:?}"
            );
        }
    }

    #[test]
    fn torch_banner_goes_on_first_wall_in_order_n_e_s_w() {
        let m = parse("P#.\n.T.\n...\n");
        assert_eq!(
            m.torches,
            [Torch {
                cell: ivec2(1, 1),
                wall: Dir::N
            }]
        );
        let m = parse(".P.\n.T#\n.#.\n");
        assert_eq!(m.torches[0].wall, Dir::E);
        let m = parse(".P.\n#T.\n.#.\n");
        assert_eq!(m.torches[0].wall, Dir::S);
        let m = parse(".P.\n#T.\n...\n");
        assert_eq!(m.torches[0].wall, Dir::W);
    }

    #[test]
    fn sixteen_torches_parse_seventeenth_is_an_error() {
        let row = |n| format!("{}\n", "T".repeat(n));
        let walls = "#".repeat(MAX_LIGHTS + 1);
        let ok = format!("{walls}\n{}P\n", row(MAX_LIGHTS).trim_end());
        assert_eq!(parse(&ok).torches.len(), MAX_LIGHTS);
        let bad = format!("{walls}\n{}", row(MAX_LIGHTS + 1));
        assert_eq!(Map::parse(&bad), Err(MapError::TooManyTorches { line: 2 }));
    }

    #[test]
    fn parse_errors_name_the_line() {
        assert_eq!(Map::parse("P..\n..\n"), Err(MapError::Ragged { line: 2 }));
        assert_eq!(
            Map::parse("P..\n.?.\n"),
            Err(MapError::Unknown { line: 2, ch: '?' })
        );
        assert_eq!(
            Map::parse(" P\n"),
            Err(MapError::Unknown { line: 1, ch: ' ' })
        );
        assert_eq!(Map::parse("...\n###\n"), Err(MapError::NoPlayer));
        assert_eq!(Map::parse(""), Err(MapError::NoPlayer));
        assert_eq!(
            Map::parse("P..\n..P\n"),
            Err(MapError::TwoPlayers { line: 2 })
        );
        assert_eq!(
            Map::parse("P..\n...\n.T.\n"),
            Err(MapError::TorchWithoutWall { line: 3 })
        );
    }

    #[test]
    fn torch_on_map_edge_has_no_wall_outside() {
        assert_eq!(
            Map::parse("TP\n"),
            Err(MapError::TorchWithoutWall { line: 1 })
        );
    }

    #[test]
    fn errors_print_line_and_cause() {
        let msg = |text| Map::parse(text).unwrap_err().to_string();
        assert_eq!(msg("P..\n..\n"), "строка 2: длина не как у первой");
        assert_eq!(msg("P..\n.?.\n"), "строка 2: неизвестный символ '?'");
        assert_eq!(msg(""), "нет P");
        assert_eq!(msg("P\nP\n"), "строка 2: второй P");
        let many = format!("#\n{}", "T\n".repeat(MAX_LIGHTS + 1));
        assert_eq!(msg(&many), "строка 18: факелов больше 16");
        assert_eq!(msg("P.T\n"), "строка 1: T без соседней стены");
    }

    /// Клетки, достижимые от `P` по полу; ворота проходимы при `through_gates`.
    fn reachable(m: &Map, through_gates: bool) -> Vec<IVec2> {
        let mut seen = vec![m.player];
        let mut i = 0;
        while let Some(&cell) = seen.get(i) {
            i += 1;
            for d in Dir::ALL {
                let next = cell + d.offset();
                let open = through_gates || !m.gates.contains(&next);
                // `inside` держит обход конечным, даже если `floor` врёт.
                if m.inside(next) && m.floor(next) && open && !seen.contains(&next) {
                    seen.push(next);
                }
            }
        }
        seen
    }

    #[test]
    fn level01_key_before_gate_and_exit_behind_it() {
        let m = parse(LEVEL01);
        assert!(!m.orcs.is_empty() && !m.coins.is_empty() && !m.torches.is_empty());
        assert_eq!((m.keys.len(), m.gates.len(), m.exits.len()), (1, 1, 1));
        let closed = reachable(&m, false);
        assert!(closed.contains(&m.keys[0]) && !closed.contains(&m.exits[0]));
        let open = reachable(&m, true);
        let floor = m.cells().filter(|&c| m.floor(c)).count();
        assert_eq!(open.len(), floor, "весь пол связан");
    }
}
