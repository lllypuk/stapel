use std::collections::HashMap;

use glam::{IVec2, Vec3};

/// `|e1 × e2|` ниже порога — треугольник вырожденный, нормаль из него — NaN.
const DEGENERATE: f32 = 1e-8;

/// Треугольник статического мира.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    /// Вершины в мире; обмотка против часовой со стороны лица.
    pub verts: [Vec3; 3],
    /// Метка игры: поверхность, барьер.
    pub tag: u32,
}

/// Попадание луча.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// `point = origin + dir·t`.
    pub t: f32,
    /// Точка попадания в мире.
    pub point: Vec3,
    /// Единичная нормаль лица треугольника, с какой стороны ни пришёл луч.
    pub normal: Vec3,
    /// Индекс треугольника во входе `StaticWorld::new`.
    pub tri: u32,
    /// `Triangle::tag`.
    pub tag: u32,
}

#[derive(Clone, Copy, Debug)]
struct Tri {
    verts: [Vec3; 3],
    normal: Vec3,
    tag: u32,
}

/// Неподвижные треугольники в сетке по XZ.
#[derive(Debug)]
pub struct StaticWorld {
    cell: f32,
    tris: Vec<Tri>,
    grid: HashMap<IVec2, Vec<u32>>,
}

impl StaticWorld {
    /// Мир из треугольников с ячейкой сетки `cell`; вырожденные в сетку не попадают, но держат свой индекс.
    pub fn new(cell: f32, triangles: impl IntoIterator<Item = Triangle>) -> Self {
        let mut world = Self {
            cell,
            tris: Vec::new(),
            grid: HashMap::new(),
        };
        for (i, t) in (0u32..).zip(triangles) {
            let [a, b, c] = t.verts;
            let cross = (b - a).cross(c - a);
            world.tris.push(Tri {
                verts: t.verts,
                normal: cross.normalize_or_zero(),
                tag: t.tag,
            });
            if cross.length() < DEGENERATE {
                continue;
            }
            let (lo, hi) = world.cells(a.min(b).min(c), a.max(b).max(c));
            for x in lo.x..=hi.x {
                for z in lo.y..=hi.y {
                    world.grid.entry(IVec2::new(x, z)).or_default().push(i);
                }
            }
        }
        world
    }

    /// Ближайшее попадание отрезка `origin..origin + dir·max`, прошедшее `filter`; при равных `t` — меньший `tri`.
    pub fn raycast(
        &self,
        origin: Vec3,
        dir: Vec3,
        max: f32,
        mut filter: impl FnMut(&Hit) -> bool,
    ) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for i in self.candidates(origin, dir, max) {
            let tri = &self.tris[i as usize];
            let Some(t) = intersect(origin, dir, tri.verts) else {
                continue;
            };
            if t > max || best.is_some_and(|b| t >= b.t) {
                continue;
            }
            let hit = Hit {
                t,
                point: origin + dir * t,
                normal: tri.normal,
                tri: i,
                tag: tri.tag,
            };
            if filter(&hit) {
                best = Some(hit);
            }
        }
        best
    }

    /// Треугольники ячеек AABB отрезка, по возрастанию индекса без повторов.
    fn candidates(&self, origin: Vec3, dir: Vec3, max: f32) -> Vec<u32> {
        let end = origin + dir * max;
        let (lo, hi) = self.cells(origin.min(end), origin.max(end));
        let mut ids = Vec::new();
        for x in lo.x..=hi.x {
            for z in lo.y..=hi.y {
                if let Some(cell) = self.grid.get(&IVec2::new(x, z)) {
                    ids.extend_from_slice(cell);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn cells(&self, min: Vec3, max: Vec3) -> (IVec2, IVec2) {
        (self.cell_of(min), self.cell_of(max))
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "координаты мира далеко внутри i32 ячеек"
    )]
    fn cell_of(&self, p: Vec3) -> IVec2 {
        IVec2::new(
            (p.x / self.cell).floor() as i32,
            (p.z / self.cell).floor() as i32,
        )
    }
}

/// `t ≥ 0` пересечения прямой `origin + dir·t` с треугольником (Мёллер — Трумбор), обе стороны.
/// Рёбра включительно: строгие границы пропустили бы луч точно в шов двух треугольников.
fn intersect(origin: Vec3, dir: Vec3, [v0, v1, v2]: [Vec3; 3]) -> Option<f32> {
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let pv = dir.cross(e2);
    let det = e1.dot(pv);
    if det == 0.0 {
        return None;
    }
    let inv = det.recip();
    let sv = origin - v0;
    let u = sv.dot(pv) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qv = sv.cross(e1);
    let v = dir.dot(qv) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(qv) * inv;
    (t >= 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOWN: Vec3 = Vec3::NEG_Y;

    /// Квадрат `[x0, x0+size]×[z0, z0+size]` на высоте `y` двумя треугольниками с нормалью вверх.
    fn quad(x0: f32, z0: f32, size: f32, y: f32, tag: u32) -> [Triangle; 2] {
        let p = |x: f32, z: f32| Vec3::new(x0 + x, y, z0 + z);
        [
            Triangle {
                verts: [p(0.0, 0.0), p(0.0, size), p(size, 0.0)],
                tag,
            },
            Triangle {
                verts: [p(size, 0.0), p(0.0, size), p(size, size)],
                tag,
            },
        ]
    }

    fn any(_: &Hit) -> bool {
        true
    }

    #[test]
    fn ray_hits_triangle_with_t_point_normal_tri_tag() {
        let world = StaticWorld::new(4.0, quad(-1.0, -1.0, 2.0, 0.5, 7));
        let hit = world
            .raycast(Vec3::new(0.5, 3.0, 0.5), DOWN, 10.0, any)
            .expect("hit");
        assert_eq!(hit.t, 2.5);
        assert_eq!(hit.point, Vec3::new(0.5, 0.5, 0.5));
        assert_eq!(hit.normal, Vec3::Y);
        assert_eq!(hit.tri, 1);
        assert_eq!(hit.tag, 7);
    }

    #[test]
    fn upward_ray_hits_back_side_with_same_normal() {
        let world = StaticWorld::new(4.0, quad(0.0, 0.0, 2.0, 1.0, 0));
        let hit = world
            .raycast(Vec3::new(0.5, 0.0, 0.5), Vec3::Y, 2.0, any)
            .expect("hit");
        assert_eq!(hit.t, 1.0);
        assert_eq!(hit.normal, Vec3::Y);
    }

    #[test]
    fn ray_misses_beside_beyond_max_and_behind() {
        let world = StaticWorld::new(4.0, quad(0.0, 0.0, 2.0, 0.0, 0));
        let miss = |origin: Vec3, dir: Vec3, max: f32| world.raycast(origin, dir, max, any);
        assert_eq!(miss(Vec3::new(2.5, 1.0, 0.5), DOWN, 5.0), None);
        assert_eq!(miss(Vec3::new(0.5, 1.0, -0.5), DOWN, 5.0), None);
        assert_eq!(miss(Vec3::new(0.5, 1.0, 0.5), DOWN, 0.99), None);
        assert_eq!(miss(Vec3::new(0.5, -1.0, 0.5), DOWN, 5.0), None);
        assert!(miss(Vec3::new(0.5, 1.0, 0.5), DOWN, 1.0).is_some());
    }

    #[test]
    fn ray_misses_past_each_edge() {
        let world = StaticWorld::new(4.0, quad(1.0, 1.0, 2.0, 0.0, 0));
        for (x, z) in [(0.5, 1.5), (3.5, 1.5), (1.5, 0.5), (1.5, 3.5)] {
            let hit = world.raycast(Vec3::new(x, 1.0, z), DOWN, 5.0, any);
            assert_eq!(hit, None, "({x}, {z})");
        }
    }

    #[test]
    fn ray_on_edge_or_vertex_hits() {
        let world = StaticWorld::new(4.0, [quad(1.0, 1.0, 2.0, 0.0, 0)[0]]);
        for (x, z) in [(1.0, 1.5), (1.5, 1.0), (2.0, 2.0), (1.0, 1.0)] {
            let hit = world.raycast(Vec3::new(x, 1.0, z), DOWN, 5.0, any);
            assert!(hit.is_some(), "({x}, {z})");
        }
    }

    #[test]
    fn candidates_come_only_from_cells_under_segment() {
        let tris = [
            quad(0.5, 0.5, 3.0, 0.0, 0),
            quad(4.5, 4.5, 3.0, 0.0, 0),
            quad(8.5, 8.5, 3.0, 0.0, 0),
            quad(-3.5, -3.5, 3.0, 0.0, 0),
        ];
        let world = StaticWorld::new(4.0, tris.concat());
        let origin = Vec3::new(0.6, 1.0, 0.6);
        let diagonal = Vec3::new(3.0, 0.0, 3.0);
        assert_eq!(world.candidates(origin, diagonal, 3.0), [0, 1, 2, 3, 4, 5]);
        assert_eq!(
            world.candidates(Vec3::new(3.9, 1.0, 3.9), DOWN, 2.0),
            [0, 1]
        );
        assert_eq!(
            world.candidates(Vec3::new(-0.1, 1.0, -0.1), DOWN, 2.0),
            [6, 7]
        );
        assert_eq!(
            world.candidates(Vec3::new(4.1, 1.0, 0.0), DOWN, 2.0),
            [] as [u32; 0]
        );
    }

    #[test]
    fn ray_parallel_to_triangle_misses() {
        let world = StaticWorld::new(4.0, quad(0.0, 0.0, 2.0, 0.0, 0));
        let hit = world.raycast(Vec3::new(-1.0, 0.0, 0.5), Vec3::X, 5.0, any);
        assert_eq!(hit, None);
    }

    #[test]
    fn filter_skips_bridge_and_finds_road_below() {
        let mut tris = Vec::from(quad(0.0, 0.0, 4.0, 0.0, 1));
        tris.extend(quad(0.0, 0.0, 4.0, 2.0, 2));
        let world = StaticWorld::new(4.0, tris);
        let origin = Vec3::new(1.0, 5.0, 1.0);
        let near = world.raycast(origin, DOWN, 10.0, any).expect("bridge");
        assert_eq!((near.tag, near.t), (2, 3.0));
        let far = world
            .raycast(origin, DOWN, 10.0, |h| h.tag != 2)
            .expect("road");
        assert_eq!((far.tag, far.t, far.tri), (1, 5.0, 0));
    }

    #[test]
    fn equal_t_picks_lower_tri() {
        let [a, _] = quad(0.0, 0.0, 2.0, 0.0, 0);
        let b = Triangle { tag: 1, ..a };
        let hit = |world: &StaticWorld| {
            world
                .raycast(Vec3::new(0.5, 1.0, 0.5), DOWN, 5.0, any)
                .expect("hit")
        };
        assert_eq!(hit(&StaticWorld::new(4.0, [a, b])).tag, 0);
        assert_eq!(hit(&StaticWorld::new(4.0, [b, a])).tag, 1);
    }

    #[test]
    fn triangle_in_many_cells_is_tested_once() {
        let world = StaticWorld::new(1.0, [quad(-2.5, -2.5, 5.0, 0.0, 0)[0]]);
        let mut calls = 0;
        let hit = world.raycast(
            Vec3::new(-2.0, 1.0, -2.0),
            Vec3::new(2.0, -1.0, 1.0),
            2.0,
            |_| {
                calls += 1;
                true
            },
        );
        let point = hit.expect("hit").point;
        assert!(
            point.distance(Vec3::new(0.0, 0.0, -1.0)) < 1e-6,
            "{point:?}"
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn triangle_is_found_in_every_cell_it_spans() {
        let world = StaticWorld::new(1.0, [quad(-2.5, -2.5, 5.0, 0.0, 0)[0]]);
        for (x, z) in [
            (-2.4, -2.4),
            (-2.4, 2.3),
            (2.3, -2.4),
            (-0.5, 0.4),
            (0.4, -0.5),
        ] {
            let hit = world.raycast(Vec3::new(x, 1.0, z), DOWN, 2.0, any);
            assert!(hit.is_some(), "({x}, {z})");
        }
    }

    #[test]
    fn degenerate_triangle_gives_no_hit() {
        let sliver = Triangle {
            verts: [Vec3::ZERO, Vec3::new(1.0, 0.0, 1e-9), Vec3::X],
            tag: 0,
        };
        let road = quad(0.0, -1.0, 2.0, -1.0, 1)[0];
        let world = StaticWorld::new(4.0, [sliver, road]);
        let hit = world
            .raycast(Vec3::new(0.5, 1.0, 0.25e-9), DOWN, 5.0, any)
            .expect("road");
        assert_eq!((hit.tri, hit.tag), (1, 1));
        assert!(hit.normal.is_finite());
    }

    #[test]
    fn empty_world_gives_none() {
        let world = StaticWorld::new(4.0, []);
        assert_eq!(world.raycast(Vec3::Y, DOWN, 5.0, any), None);
    }
}
