use std::fmt;

use engine::{Aabb, Axis, IVec2, ivec2};

use crate::TILE;

/// Ошибка разбора карты; `line` — с единицы, первая строка — верх.
#[derive(Debug, PartialEq)]
pub(crate) enum LevelError {
    Ragged { line: usize },
    Unknown { line: usize, ch: char },
    NoPlayer,
    TwoPlayers { line: usize },
}

impl fmt::Display for LevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ragged { line } => write!(f, "строка {line}: длина не как у первой"),
            Self::Unknown { line, ch } => write!(f, "строка {line}: неизвестный символ {ch:?}"),
            Self::NoPlayer => write!(f, "нет P"),
            Self::TwoPlayers { line } => write!(f, "строка {line}: второй P"),
        }
    }
}

/// Платформа из подряд идущих `=` (по X) или `H` (по Y); `cells` — в клетках.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlatformSpawn {
    pub(crate) cells: Aabb,
    pub(crate) axis: Axis,
}

/// Уровень из ASCII-карты; клетки — от левой нижней, Y вверх.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Level {
    pub(crate) size: IVec2,
    /// По строкам снизу вверх.
    pub(crate) solid: Vec<bool>,
    pub(crate) player: IVec2,
    pub(crate) enemies: Vec<IVec2>,
    pub(crate) coins: Vec<IVec2>,
    pub(crate) spikes: Vec<IVec2>,
    pub(crate) exits: Vec<IVec2>,
    pub(crate) stops: Vec<IVec2>,
    pub(crate) platforms: Vec<PlatformSpawn>,
}

impl Level {
    pub(crate) fn parse(text: &str) -> Result<Self, LevelError> {
        let rows: Vec<Vec<char>> = text.lines().map(|l| l.chars().collect()).collect();
        let width = rows.first().map_or(0, Vec::len);
        let to_i32 = |n: usize| i32::try_from(n).expect("карта меньше 2³¹ клеток");
        let size = ivec2(to_i32(width), to_i32(rows.len()));
        let mut level = Self {
            size,
            solid: vec![false; width * rows.len()],
            player: IVec2::ZERO,
            enemies: Vec::new(),
            coins: Vec::new(),
            spikes: Vec::new(),
            exits: Vec::new(),
            stops: Vec::new(),
            platforms: Vec::new(),
        };
        let mut player = None;
        for (i, row) in rows.iter().enumerate() {
            let line = i + 1;
            if row.len() != width {
                return Err(LevelError::Ragged { line });
            }
            let y = size.y - 1 - to_i32(i);
            for (j, &ch) in row.iter().enumerate() {
                let cell = ivec2(to_i32(j), y);
                match ch {
                    '#' => level.solid[to_index(cell, size.x)] = true,
                    'P' if player.is_some() => return Err(LevelError::TwoPlayers { line }),
                    'P' => player = Some(cell),
                    'E' => level.enemies.push(cell),
                    'o' => level.coins.push(cell),
                    '^' => level.spikes.push(cell),
                    'X' => level.exits.push(cell),
                    '!' => level.stops.push(cell),
                    '=' if j == 0 || row[j - 1] != '=' => {
                        let len = row[j..].iter().take_while(|&&c| c == '=').count();
                        level.platforms.push(PlatformSpawn {
                            cells: Aabb {
                                min: cell,
                                size: ivec2(to_i32(len), 1),
                            },
                            axis: Axis::X,
                        });
                    }
                    'H' if i == 0 || rows[i - 1][j] != 'H' => {
                        let len = rows[i..]
                            .iter()
                            .take_while(|r| r.get(j) == Some(&'H'))
                            .count();
                        level.platforms.push(PlatformSpawn {
                            cells: Aabb {
                                min: ivec2(cell.x, y + 1 - to_i32(len)),
                                size: ivec2(1, to_i32(len)),
                            },
                            axis: Axis::Y,
                        });
                    }
                    ' ' | '.' | '=' | 'H' => {}
                    ch => return Err(LevelError::Unknown { line, ch }),
                }
            }
        }
        level.player = player.ok_or(LevelError::NoPlayer)?;
        Ok(level)
    }

    /// Вне карты — не твёрдое.
    pub(crate) fn solid_at(&self, cell: IVec2) -> bool {
        cell.cmpge(IVec2::ZERO).all()
            && cell.cmplt(self.size).all()
            && self.solid[to_index(cell, self.size.x)]
    }

    pub(crate) fn bounds(&self) -> IVec2 {
        self.size * TILE
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
    use crate::{LEVEL01, VIEW};

    fn parse(text: &str) -> Level {
        Level::parse(text).unwrap()
    }

    #[test]
    fn first_line_is_top_and_hash_is_solid() {
        let l = parse("#..\n.P#\n");
        assert_eq!((l.size, l.player), (ivec2(3, 2), ivec2(1, 0)));
        assert_eq!(l.solid, [false, false, true, true, false, false]);
        assert!(l.solid_at(ivec2(0, 1)) && l.solid_at(ivec2(2, 0)));
        assert!(!l.solid_at(ivec2(1, 0)));
    }

    #[test]
    fn solid_at_outside_map_is_empty() {
        let l = parse("###\n#P#\n");
        for cell in [ivec2(-1, 0), ivec2(3, 0), ivec2(0, -1), ivec2(0, 2)] {
            assert!(!l.solid_at(cell), "{cell}");
        }
    }

    #[test]
    fn spawns_are_cells_from_bottom() {
        let l = parse("E o.^\nX!P  \n");
        assert_eq!(l.enemies, [ivec2(0, 1)]);
        assert_eq!(l.coins, [ivec2(2, 1)]);
        assert_eq!(l.spikes, [ivec2(4, 1)]);
        assert_eq!(l.exits, [ivec2(0, 0)]);
        assert_eq!(l.stops, [ivec2(1, 0)]);
        assert_eq!(l.player, ivec2(2, 0));
        assert!(l.solid.iter().all(|&s| !s));
    }

    #[test]
    fn runs_of_eq_and_h_are_one_platform_each() {
        let l = parse("==.=\n=..H\nP.HH\n");
        let p = |min, size, axis| PlatformSpawn {
            cells: Aabb { min, size },
            axis,
        };
        assert_eq!(
            l.platforms,
            [
                p(ivec2(0, 2), ivec2(2, 1), Axis::X),
                p(ivec2(3, 2), ivec2(1, 1), Axis::X),
                p(ivec2(0, 1), ivec2(1, 1), Axis::X),
                p(ivec2(3, 0), ivec2(1, 2), Axis::Y),
                p(ivec2(2, 0), ivec2(1, 1), Axis::Y),
            ]
        );
    }

    #[test]
    fn h_run_starts_at_top_row() {
        let l = parse("H\nH\nP\n");
        assert_eq!(l.platforms[0].cells.min, ivec2(0, 1));
        assert_eq!(l.platforms[0].cells.size, ivec2(1, 2));
    }

    #[test]
    fn parse_errors_name_the_line() {
        assert_eq!(
            Level::parse("P..\n..\n"),
            Err(LevelError::Ragged { line: 2 })
        );
        assert_eq!(
            Level::parse("P..\n.?.\n"),
            Err(LevelError::Unknown { line: 2, ch: '?' })
        );
        assert_eq!(Level::parse("...\n###\n"), Err(LevelError::NoPlayer));
        assert_eq!(Level::parse(""), Err(LevelError::NoPlayer));
        assert_eq!(
            Level::parse("P..\n..P\n"),
            Err(LevelError::TwoPlayers { line: 2 })
        );
    }

    #[test]
    fn errors_print_line_and_cause() {
        let msg = |text| Level::parse(text).unwrap_err().to_string();
        assert_eq!(msg("P..\n..\n"), "строка 2: длина не как у первой");
        assert_eq!(msg("P..\n.?.\n"), "строка 2: неизвестный символ '?'");
        assert_eq!(msg(""), "нет P");
        assert_eq!(msg("P\nP\n"), "строка 2: второй P");
    }

    #[test]
    fn level01_is_at_least_four_screens_wide() {
        let l = parse(LEVEL01);
        assert!(l.bounds().x >= 4 * VIEW.x, "{}", l.bounds());
        assert!(l.bounds().y >= VIEW.y, "{}", l.bounds());
        assert!(!l.exits.is_empty() && !l.platforms.is_empty());
    }
}
