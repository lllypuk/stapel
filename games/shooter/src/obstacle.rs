use std::collections::VecDeque;

use engine::{IVec2, Vec2};

use crate::map::{Dir, Map, Tile};

/// Единственный запрос препятствий для хода, взгляда, выстрела, снарядов и пути.
///
/// План в клетках: клетка `(x, y)` — квадрат `[x, x+1)×[y, y+1)`. Вне карты всё твёрдо.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Obstacles {
    solid: Vec<bool>,
    size: IVec2,
    /// Закрытые двери.
    doors: Vec<IVec2>,
}

impl Obstacles {
    pub(crate) fn new(map: &Map) -> Self {
        Self {
            solid: map
                .cells()
                .map(|c| map.tile(c).is_some_and(Tile::solid))
                .collect(),
            size: map.size,
            doors: map.doors.clone(),
        }
    }

    /// Дверь перестаёт быть препятствием; игра зовёт это в конце анимации открытия.
    pub(crate) fn open(&mut self, door: IVec2) {
        self.doors.retain(|&d| d != door);
    }

    /// Двери только открываются: смена числа — смена путей.
    pub(crate) fn closed_doors(&self) -> usize {
        self.doors.len()
    }

    pub(crate) fn solid(&self, cell: IVec2) -> bool {
        self.index(cell).is_none_or(|i| self.solid[i]) || self.doors.contains(&cell)
    }

    /// Расстояние вдоль `dir` до первой твёрдой клетки, если оно не больше `max`.
    ///
    /// Шаг — всегда через одну грань: диагональный шаг ровно в угол проскочил бы между
    /// клетками, касающимися углом.
    pub(crate) fn ray(&self, from: Vec2, dir: Vec2, max: f32) -> Option<f32> {
        let mut cell = from.floor().as_ivec2();
        if self.solid(cell) {
            return Some(0.0);
        }
        let dir = dir.normalize_or_zero();
        if dir == Vec2::ZERO {
            return None;
        }
        let step = dir.signum().as_ivec2();
        let delta = dir.recip().abs();
        let mut t = Vec2::new(
            first_edge(from.x, cell.x, dir.x),
            first_edge(from.y, cell.y, dir.y),
        );
        loop {
            let hit = t.x.min(t.y);
            if hit > max {
                return None;
            }
            if t.x < t.y {
                cell.x += step.x;
                t.x += delta.x;
            } else {
                cell.y += step.y;
                t.y += delta.y;
            }
            if self.solid(cell) {
                return Some(hit);
            }
        }
    }

    /// Расстояние от `a` до первого препятствия на отрезке к `b`; `None` — путь свободен.
    pub(crate) fn segment(&self, a: Vec2, b: Vec2) -> Option<f32> {
        self.ray(a, b - a, a.distance(b))
    }

    /// Кратчайший путь по соседям через грань: клетки после `from` до `to` включительно.
    ///
    /// `from` может быть твёрдой: враг, вставший в клетку, не теряет путь.
    pub(crate) fn path(&self, from: IVec2, to: IVec2) -> Option<Vec<IVec2>> {
        let (Some(start), false) = (self.index(from), self.solid(to)) else {
            return None;
        };
        let mut next = vec![None; self.solid.len()];
        next[self.index(to)?] = Some(to);
        let mut queue = VecDeque::from([to]);
        while let Some(cell) = queue.pop_front() {
            if next[start].is_some() {
                break;
            }
            for d in Dir::ALL {
                let n = cell + d.offset();
                if n != from && self.solid(n) {
                    continue;
                }
                let i = self.index(n)?;
                if next[i].is_none() {
                    next[i] = Some(cell);
                    queue.push_back(n);
                }
            }
        }
        let mut path = Vec::new();
        let mut cell = from;
        while cell != to {
            cell = next[self.index(cell)?]?;
            path.push(cell);
        }
        Some(path)
    }

    fn index(&self, cell: IVec2) -> Option<usize> {
        let inside = cell.cmpge(IVec2::ZERO).all() && cell.cmplt(self.size).all();
        inside.then(|| usize::try_from(cell.y * self.size.x + cell.x).expect("клетка внутри"))
    }
}

/// Расстояние вдоль луча до первой грани клетки по оси; без движения по оси — бесконечность.
fn first_edge(p: f32, cell: i32, d: f32) -> f32 {
    if d == 0.0 {
        return f32::INFINITY;
    }
    let edge = cell as f32 + f32::from(u8::from(d.is_sign_positive()));
    (edge - p) / d
}

#[cfg(test)]
mod tests {
    use engine::{ivec2, vec2};

    use super::*;

    fn obstacles(text: &str) -> Obstacles {
        Obstacles::new(&Map::parse(text).unwrap())
    }

    fn near(a: Option<f32>, b: f32) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-5)
    }

    const ROOM: &str = "\
#####
#P..#
#..X#
#####
";

    #[test]
    fn solid_is_obstacle_tile_closed_door_or_outside() {
        let mut o = obstacles("PTRk\n#.DX\n");
        let solid: Vec<_> = (0..2)
            .flat_map(|y| (0..4).map(move |x| ivec2(x, y)))
            .filter(|&c| o.solid(c))
            .collect();
        assert_eq!(solid, [ivec2(1, 0), ivec2(2, 0), ivec2(0, 1), ivec2(2, 1)]);
        for c in [ivec2(-1, 0), ivec2(4, 0), ivec2(0, -1), ivec2(0, 2)] {
            assert!(o.solid(c), "{c}");
        }
        assert_eq!(o.closed_doors(), 1);
        o.open(ivec2(2, 1));
        assert!(!o.solid(ivec2(2, 1)));
        assert_eq!(o.closed_doors(), 0);
    }

    #[test]
    fn ray_stops_at_solid_cell_in_every_direction() {
        let o = obstacles(ROOM);
        let from = vec2(1.5, 1.5);
        assert!(near(o.ray(from, vec2(1.0, 0.0), 10.0), 2.5));
        assert!(near(o.ray(from, vec2(-1.0, 0.0), 10.0), 0.5));
        assert!(near(o.ray(from, vec2(0.0, -3.0), 10.0), 0.5));
        assert!(near(o.ray(from, vec2(0.0, 1.0), 10.0), 1.5));
        assert!(near(o.ray(from, vec2(2.0, 1.0), 10.0), 1.25 * 5f32.sqrt()));
        assert!(near(
            o.ray(vec2(3.5, 2.5), vec2(-2.0, -1.0), 10.0),
            1.25 * 5f32.sqrt()
        ));
    }

    #[test]
    fn ray_hits_within_max_inclusive_and_leaves_map_as_solid() {
        let o = obstacles(ROOM);
        let from = vec2(1.5, 1.5);
        assert!(near(o.ray(from, Vec2::X, 2.5), 2.5));
        assert_eq!(o.ray(from, Vec2::X, 2.4), None);
        assert!(near(
            obstacles("P.X\n").ray(vec2(0.5, 0.5), Vec2::X, 10.0),
            2.5
        ));
    }

    #[test]
    fn ray_from_solid_is_zero_and_without_direction_is_none() {
        let o = obstacles(ROOM);
        assert_eq!(o.ray(vec2(0.5, 1.5), Vec2::X, 10.0), Some(0.0));
        assert_eq!(o.ray(vec2(1.5, 1.5), Vec2::ZERO, 10.0), None);
    }

    #[test]
    fn ray_stops_at_closed_door_and_passes_open_one() {
        let mut o = obstacles("#######\n#P.D.k#\n#....X#\n#######\n");
        let from = vec2(1.5, 1.5);
        assert!(near(o.ray(from, Vec2::X, 10.0), 1.5));
        o.open(ivec2(3, 1));
        assert!(near(o.ray(from, Vec2::X, 10.0), 4.5));
    }

    #[test]
    fn ray_into_corner_does_not_slip_between_cells() {
        let o = obstacles("####\n#P##\n##.#\n#X.#\n####\n");
        let to = |from: Vec2| o.ray(from, vec2(2.0, 2.0) - from, 10.0);
        assert!(near(to(vec2(1.5, 1.5)), 0.5 * 2f32.sqrt()));
        assert!(near(to(vec2(1.25, 1.75)), vec2(0.75, 0.25).length()));
        assert!(near(to(vec2(1.75, 1.25)), vec2(0.25, 0.75).length()));
    }

    #[test]
    fn segment_catches_thin_wall_on_long_step() {
        let o = obstacles("#######\n#P.#.X#\n#..#..#\n#######\n");
        assert!(near(o.segment(vec2(1.5, 1.5), vec2(4.5, 1.5)), 1.5));
        assert!(o.segment(vec2(1.2, 2.5), vec2(4.8, 1.2)).is_some());
        assert_eq!(o.segment(vec2(1.2, 1.5), vec2(2.8, 2.5)), None);
        assert_eq!(o.segment(vec2(1.5, 1.5), vec2(1.5, 1.5)), None);
    }

    #[test]
    fn segment_through_door_only_after_open() {
        let mut o = obstacles("#####\n#PDX#\n#k###\n#####\n");
        let (a, b) = (vec2(1.5, 1.5), vec2(3.5, 1.5));
        assert!(near(o.segment(a, b), 0.5));
        o.open(ivec2(2, 1));
        assert_eq!(o.segment(a, b), None);
    }

    /// Путь идёт по свободным соседям через грань от `from`.
    fn walkable(o: &Obstacles, from: IVec2, path: &[IVec2]) -> bool {
        let mut prev = from;
        path.iter().all(|&c| {
            let ok = (c - prev).abs().element_sum() == 1 && !o.solid(c);
            prev = c;
            ok
        })
    }

    #[test]
    fn path_goes_around_closed_door_and_through_it_after_open() {
        let mut o = obstacles("#######\n#P.D.X#\n#.###.#\n#....k#\n#######\n");
        let (from, to) = (ivec2(1, 1), ivec2(5, 1));
        let around = o.path(from, to).unwrap();
        assert_eq!(around.len(), 8);
        assert!(walkable(&o, from, &around) && around.last() == Some(&to));
        o.open(ivec2(3, 1));
        let through = o.path(from, to).unwrap();
        assert_eq!(through, [ivec2(2, 1), ivec2(3, 1), ivec2(4, 1), to]);
    }

    #[test]
    fn path_behind_only_door_appears_after_open() {
        let mut o = obstacles("#####\n#PDX#\n#k###\n#####\n");
        let (from, to) = (ivec2(1, 1), ivec2(3, 1));
        assert_eq!(o.path(from, to), None);
        assert_eq!(o.path(ivec2(2, 1), to), Some(vec![to]), "из твёрдой клетки");
        o.open(ivec2(2, 1));
        assert_eq!(o.path(from, to), Some(vec![ivec2(2, 1), to]));
    }

    #[test]
    fn path_to_itself_is_empty_and_to_solid_or_from_outside_is_none() {
        let o = obstacles(ROOM);
        assert_eq!(o.path(ivec2(1, 1), ivec2(1, 1)), Some(vec![]));
        assert_eq!(o.path(ivec2(1, 1), ivec2(0, 1)), None);
        assert_eq!(o.path(ivec2(-1, 1), ivec2(1, 1)), None);
        assert_eq!(o.path(ivec2(5, 1), ivec2(1, 1)), None);
        assert_eq!(o.path(ivec2(3, 2), ivec2(1, 1)).map(|p| p.len()), Some(3));
    }
}
