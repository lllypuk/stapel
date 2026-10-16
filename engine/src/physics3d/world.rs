use std::collections::HashMap;

use glam::{IVec2, IVec3, Vec3};

/// `|e1 × e2|` ниже порога — треугольник вырожденный, нормаль из него — NaN.
const DEGENERATE: f32 = 1e-8;

/// Вершины ближе — одна вершина; степень двойки, чтобы граница в тестах была точной.
const WELD: f32 = 1.0 / 1024.0;

/// `n₁·n₂` не меньше — общее ребро внутреннее (угол до ~8°).
const INTERNAL_COS: f32 = 0.99;

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

/// Контакт сферы с треугольником.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// Ближайшая к центру точка треугольника.
    pub point: Vec3,
    /// Единичная, от мира к сфере.
    pub normal: Vec3,
    /// Глубина вдоль `normal`, больше нуля.
    pub depth: f32,
    /// Индекс треугольника во входе `StaticWorld::new`.
    pub tri: u32,
}

#[derive(Clone, Copy, Debug)]
struct Tri {
    verts: [Vec3; 3],
    normal: Vec3,
    tag: u32,
    /// Ребро `k` — `verts[k] → verts[(k + 1) % 3]`.
    internal: [bool; 3],
    /// Все рёбра мира у вершины `k` внутренние.
    inner: [bool; 3],
}

/// Часть треугольника с ближайшей точкой; номер — как у `verts` и рёбер.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Feature {
    Face,
    Edge(usize),
    Vertex(usize),
}

/// Неподвижные треугольники в сетке по XZ.
#[derive(Debug)]
pub struct StaticWorld {
    cell: f32,
    tris: Vec<Tri>,
    grid: HashMap<IVec2, Vec<u32>>,
    /// Занятые ячейки `lo..=hi`: запрос шире перебирает только их.
    extent: (IVec2, IVec2),
}

impl StaticWorld {
    /// Мир из треугольников с ячейкой сетки `cell`; вырожденные в сетку не попадают, но держат свой индекс.
    /// Ребро внутреннее, если после склейки вершин его делят ровно два почти компланарных треугольника;
    /// T-стык не склеивается, и ребро на нём остаётся внешним.
    pub fn new(cell: f32, triangles: impl IntoIterator<Item = Triangle>) -> Self {
        let mut world = Self {
            cell,
            tris: Vec::new(),
            grid: HashMap::new(),
            extent: (IVec2::MAX, IVec2::MIN),
        };
        let mut welder = Welder::default();
        let mut ids = Vec::new();
        let mut edges: HashMap<(u32, u32), Vec<(u32, usize)>> = HashMap::new();
        for (i, t) in (0u32..).zip(triangles) {
            let [a, b, c] = t.verts;
            let cross = (b - a).cross(c - a);
            world.tris.push(Tri {
                verts: t.verts,
                normal: cross.normalize_or_zero(),
                tag: t.tag,
                internal: [false; 3],
                inner: [false; 3],
            });
            if cross.length() < DEGENERATE {
                continue;
            }
            let tri_ids = t.verts.map(|v| welder.id(v));
            for k in 0..3 {
                let (from, to) = (tri_ids[k], tri_ids[(k + 1) % 3]);
                edges
                    .entry((from.min(to), from.max(to)))
                    .or_default()
                    .push((i, k));
            }
            ids.push((i, tri_ids));
            let (lo, hi) = world.cells(a.min(b).min(c), a.max(b).max(c));
            world.extent = (world.extent.0.min(lo), world.extent.1.max(hi));
            for x in lo.x..=hi.x {
                for z in lo.y..=hi.y {
                    world.grid.entry(IVec2::new(x, z)).or_default().push(i);
                }
            }
        }
        let mut rim = vec![false; welder.verts.len()];
        for (&(from, to), sides) in &edges {
            if let &[(i, ki), (j, kj)] = sides.as_slice()
                && world.tris[i as usize]
                    .normal
                    .dot(world.tris[j as usize].normal)
                    >= INTERNAL_COS
            {
                world.tris[i as usize].internal[ki] = true;
                world.tris[j as usize].internal[kj] = true;
            } else {
                rim[from as usize] = true;
                rim[to as usize] = true;
            }
        }
        for (i, tri_ids) in ids {
            world.tris[i as usize].inner = tri_ids.map(|id| !rim[id as usize]);
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

    /// Дописывает в `out` контакты сферы с треугольниками, к которым центр обращён лицом.
    /// На внутреннем ребре и вершине, все рёбра которой внутренние, нормаль — грани, глубина — до её плоскости.
    pub fn sphere_contacts(&self, center: Vec3, r: f32, out: &mut Vec<Contact>) {
        let reach = Vec3::splat(r);
        for i in self.ids_in(center - reach, center + reach) {
            let tri = &self.tris[i as usize];
            let above = (center - tri.verts[0]).dot(tri.normal);
            if above < 0.0 {
                continue;
            }
            let (point, feature) = closest_point(center, tri.verts, tri.normal);
            let dist = center.distance(point);
            if dist >= r {
                continue;
            }
            let face = match feature {
                Feature::Face => true,
                Feature::Edge(k) => tri.internal[k],
                Feature::Vertex(k) => tri.inner[k],
            };
            let (normal, depth) = if face {
                (tri.normal, r - above)
            } else {
                let normal = (center - point).try_normalize().unwrap_or(tri.normal);
                (normal, r - dist)
            };
            out.push(Contact {
                point,
                normal,
                depth,
                tri: i,
            });
        }
    }

    /// Треугольники ячеек AABB отрезка, по возрастанию индекса без повторов.
    fn candidates(&self, origin: Vec3, dir: Vec3, max: f32) -> Vec<u32> {
        let end = origin + dir * max;
        self.ids_in(origin.min(end), origin.max(end))
    }

    /// Треугольники ячеек под AABB `min..max`, по возрастанию индекса без повторов.
    fn ids_in(&self, min: Vec3, max: Vec3) -> Vec<u32> {
        let (lo, hi) = self.cells(min, max);
        let (lo, hi) = (lo.max(self.extent.0), hi.min(self.extent.1));
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

/// Склейка вершин в пределах `WELD` через сетку с шагом `WELD`.
#[derive(Default)]
struct Welder {
    verts: Vec<Vec3>,
    grid: HashMap<IVec3, Vec<u32>>,
}

impl Welder {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "координаты мира далеко внутри i32 ячеек склейки"
    )]
    fn id(&mut self, v: Vec3) -> u32 {
        let key = (v / WELD).floor().as_ivec3();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let near = self.grid.get(&key.saturating_add(IVec3::new(dx, dy, dz)));
                    if let Some(&id) = near
                        .into_iter()
                        .flatten()
                        .find(|&&id| self.verts[id as usize].distance(v) <= WELD)
                    {
                        return id;
                    }
                }
            }
        }
        let id = self.verts.len() as u32;
        self.verts.push(v);
        self.grid.entry(key).or_default().push(id);
        id
    }
}

/// Ближайшая к `p` точка треугольника с единичной нормалью `n`.
fn closest_point(p: Vec3, verts: [Vec3; 3], n: Vec3) -> (Vec3, Feature) {
    let inside = (0..3).all(|k| {
        let a = verts[k];
        (verts[(k + 1) % 3] - a).cross(p - a).dot(n) >= 0.0
    });
    if inside {
        return (p - n * n.dot(p - verts[0]), Feature::Face);
    }
    let mut best = closest_on_edge(p, verts, 0);
    for k in 1..3 {
        let next = closest_on_edge(p, verts, k);
        if next.0.distance_squared(p) < best.0.distance_squared(p) {
            best = next;
        }
    }
    best
}

fn closest_on_edge(p: Vec3, verts: [Vec3; 3], k: usize) -> (Vec3, Feature) {
    let (start, end) = (verts[k], verts[(k + 1) % 3]);
    let along = end - start;
    let t = (p - start).dot(along) / along.length_squared();
    if t <= 0.0 {
        (start, Feature::Vertex(k))
    } else if t >= 1.0 {
        (end, Feature::Vertex((k + 1) % 3))
    } else {
        (start + along * t, Feature::Edge(k))
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

    /// Без обрезки по занятым ячейкам бесконечный AABB перебирал бы 2³² × 2³² ячеек.
    #[test]
    fn infinite_query_visits_only_occupied_cells() {
        let tris = [quad(0.5, 0.5, 3.0, 0.0, 0), quad(-3.5, 8.5, 3.0, 0.0, 0)];
        let world = StaticWorld::new(4.0, tris.concat());
        let inf = Vec3::splat(f32::INFINITY);
        assert_eq!(world.ids_in(-inf, inf), [0, 1, 2, 3]);
        assert_eq!(StaticWorld::new(4.0, []).ids_in(-inf, inf), [] as [u32; 0]);
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

    /// Стенка `x = x0` высотой `[0, 1]` и шириной `z ∈ [0, 1]` с нормалью +X.
    fn wall_x(x0: f32) -> [Triangle; 2] {
        let p = |y: f32, z: f32| Vec3::new(x0, y, z);
        [
            Triangle {
                verts: [p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)],
                tag: 0,
            },
            Triangle {
                verts: [p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)],
                tag: 0,
            },
        ]
    }

    fn contacts(world: &StaticWorld, center: Vec3, r: f32) -> Vec<Contact> {
        let mut out = Vec::new();
        world.sphere_contacts(center, r, &mut out);
        out
    }

    fn assert_contact(c: &Contact, point: Vec3, normal: Vec3, depth: f32) {
        assert!(c.point.distance(point) < 1e-5, "{c:?}");
        assert!(c.normal.distance(normal) < 1e-5, "{c:?}");
        assert!((c.depth - depth).abs() < 1e-5, "{c:?}");
    }

    #[test]
    fn sphere_over_face_edge_and_vertex_touches_closest_point() {
        let world = StaticWorld::new(4.0, [quad(0.0, 0.0, 2.0, 0.0, 3)[0]]);
        let one = |center: Vec3, r: f32| {
            let found = contacts(&world, center, r);
            assert_eq!(found.len(), 1, "{center:?}: {found:?}");
            assert_eq!(found[0].tri, 0);
            found[0]
        };
        let face = one(Vec3::new(0.5, 0.3, 0.5), 0.5);
        assert_contact(&face, Vec3::new(0.5, 0.0, 0.5), Vec3::Y, 0.2);
        let edge = one(Vec3::new(-0.3, 0.4, 1.0), 0.6);
        assert_contact(
            &edge,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(-0.6, 0.8, 0.0),
            0.1,
        );
        let hypotenuse = one(Vec3::new(1.3, 0.4, 1.3), 0.6);
        assert_contact(
            &hypotenuse,
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(0.6, 0.8, 0.6).normalize(),
            0.6 - 0.34_f32.sqrt(),
        );
        let back = one(Vec3::new(1.0, 0.4, -0.3), 0.6);
        assert_contact(
            &back,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.8, -0.6),
            0.1,
        );
        let vertex = one(Vec3::new(-0.2, 0.4, -0.4), 0.7);
        assert_contact(&vertex, Vec3::ZERO, Vec3::new(-1.0, 2.0, -2.0) / 3.0, 0.1);
        let far_vertex = one(Vec3::new(2.4, 0.3, 0.0), 0.6);
        assert_contact(
            &far_vertex,
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.8, 0.6, 0.0),
            0.1,
        );
        assert_eq!(contacts(&world, Vec3::new(0.5, 0.5, 0.5), 0.5), []);
    }

    #[test]
    fn sphere_behind_face_gives_no_contact() {
        let world = StaticWorld::new(4.0, quad(0.0, 0.0, 2.0, 0.0, 0));
        assert_eq!(contacts(&world, Vec3::new(0.5, -0.05, 0.5), 0.3), []);
        let on_plane = contacts(&world, Vec3::new(0.5, 0.0, 0.5), 0.3);
        assert_eq!(on_plane.len(), 1, "{on_plane:?}");
        assert_contact(&on_plane[0], Vec3::new(0.5, 0.0, 0.5), Vec3::Y, 0.3);
    }

    #[test]
    fn coplanar_seam_of_two_pieces_pushes_straight_up() {
        let world = StaticWorld::new(
            4.0,
            [quad(0.0, 0.0, 1.0, 0.0, 0), quad(1.0, 0.0, 1.0, 0.0, 0)].concat(),
        );
        for x in [0.9, 0.95, 1.0, 1.05, 1.1] {
            let found = contacts(&world, Vec3::new(x, 0.45, 0.5), 0.5);
            assert_ne!(found, [], "{x}");
            for c in &found {
                assert_eq!(c.normal, Vec3::Y, "{x}: {c:?}");
                assert!((c.depth - 0.05).abs() < 1e-6, "{x}: {c:?}");
            }
        }
    }

    #[test]
    fn inner_vertex_of_seam_pushes_up_and_outer_corner_does_not() {
        let tris = [
            quad(0.0, 0.0, 1.0, 0.0, 0),
            quad(1.0, 0.0, 1.0, 0.0, 0),
            quad(0.0, 1.0, 1.0, 0.0, 0),
            quad(1.0, 1.0, 1.0, 0.0, 0),
        ];
        let world = StaticWorld::new(4.0, tris.concat());
        for (x, z) in [(1.05, 1.05), (0.95, 1.05), (1.05, 0.95), (0.95, 0.95)] {
            for c in contacts(&world, Vec3::new(x, 0.45, z), 0.5) {
                assert_eq!(c.normal, Vec3::Y, "({x}, {z}): {c:?}");
            }
        }
        let corner = contacts(&world, Vec3::new(1.1, 0.4, -0.2), 0.5);
        assert_ne!(corner, []);
        for c in &corner {
            assert!(c.normal.z < -0.1, "{c:?}");
        }
    }

    #[test]
    fn vertices_within_weld_merge_and_farther_do_not() {
        let normals = |gap: f32| {
            let tris = [
                quad(0.0, 0.0, 1.0, 0.0, 0),
                quad(1.0 + gap, 0.0, 1.0, 0.0, 0),
            ];
            let world = StaticWorld::new(4.0, tris.concat());
            contacts(&world, Vec3::new(1.1, 0.45, 0.5), 0.5)
                .iter()
                .map(|c| c.normal)
                .collect::<Vec<_>>()
        };
        assert!(
            normals(WELD).iter().all(|&n| n == Vec3::Y),
            "{:?}",
            normals(WELD)
        );
        assert!(
            normals(2.0 * WELD).iter().any(|n| n.x > 0.1),
            "{:?}",
            normals(2.0 * WELD)
        );
    }

    #[test]
    fn fold_beyond_internal_angle_keeps_edge_normal() {
        let tilt = |deg: f32| {
            let (sin, cos) = deg.to_radians().sin_cos();
            let [a, b] = quad(0.0, 0.0, 1.0, 0.0, 0);
            let p = |x: f32, z: f32| Vec3::new(1.0 + x * cos, -x * sin, z);
            let slope = [
                Triangle {
                    verts: [p(0.0, 0.0), p(0.0, 1.0), p(1.0, 0.0)],
                    tag: 0,
                },
                Triangle {
                    verts: [p(1.0, 0.0), p(0.0, 1.0), p(1.0, 1.0)],
                    tag: 0,
                },
            ];
            let world = StaticWorld::new(4.0, [a, b, slope[0], slope[1]]);
            contacts(&world, Vec3::new(1.2, 0.4, 0.5), 0.5)
                .iter()
                .filter(|c| c.tri < 2)
                .map(|c| c.normal)
                .collect::<Vec<_>>()
        };
        let gentle = tilt(5.0);
        assert!(
            !gentle.is_empty() && gentle.iter().all(|&n| n == Vec3::Y),
            "{gentle:?}"
        );
        let steep = tilt(15.0);
        assert!(
            !steep.is_empty() && steep.iter().all(|n| n.x > 0.1),
            "{steep:?}"
        );
    }

    #[test]
    fn edge_shared_by_three_triangles_is_not_internal() {
        let mut tris = Vec::from(quad(0.0, 0.0, 1.0, 0.0, 0));
        tris.extend(quad(1.0, 0.0, 1.0, 0.0, 0));
        tris.push(quad(1.0, 0.0, 1.0, 0.0, 0)[0]);
        let world = StaticWorld::new(4.0, tris);
        let found = contacts(&world, Vec3::new(1.1, 0.45, 0.5), 0.5);
        assert!(found.iter().any(|c| c.normal.x > 0.1), "{found:?}");
    }

    #[test]
    fn barrier_top_edge_contact_is_at_closest_point() {
        let world = StaticWorld::new(4.0, [quad(0.0, 0.0, 1.0, 1.0, 0), wall_x(1.0)].concat());
        let found = contacts(&world, Vec3::new(1.3, 1.3, 0.5), 0.5);
        assert!(found.len() >= 2, "{found:?}");
        for c in &found {
            assert_contact(
                c,
                Vec3::new(1.0, 1.0, 0.5),
                Vec3::new(1.0, 1.0, 0.0).normalize(),
                0.5 - 0.18_f32.sqrt(),
            );
        }
        let beside = contacts(&world, Vec3::new(1.3, 0.5, 0.5), 0.5);
        assert_ne!(beside, []);
        for c in &beside {
            assert!(c.tri >= 2, "{c:?}");
            assert_contact(c, Vec3::new(1.0, 0.5, 0.5), Vec3::X, 0.2);
        }
        let above = contacts(&world, Vec3::new(0.9, 1.4, 0.5), 0.5);
        assert_ne!(above, []);
        for c in &above {
            assert!(c.tri < 2, "{c:?}");
            assert_eq!(c.normal, Vec3::Y, "{c:?}");
            assert!((c.depth - 0.1).abs() < 1e-6, "{c:?}");
        }
    }

    #[test]
    fn degenerate_triangle_gives_no_contact() {
        let sliver = Triangle {
            verts: [Vec3::ZERO, Vec3::new(1.0, 0.0, 1e-9), Vec3::X],
            tag: 0,
        };
        let world = StaticWorld::new(4.0, [sliver]);
        assert_eq!(contacts(&world, Vec3::new(0.5, 0.2, 0.0), 0.5), []);
    }

    #[test]
    fn contacts_are_appended_from_every_cell_under_sphere() {
        let world = StaticWorld::new(
            1.0,
            [
                quad(-0.5, -0.5, 0.45, 0.0, 0),
                quad(0.05, 0.05, 0.45, 0.0, 1),
            ]
            .concat(),
        );
        let mut out = vec![Contact {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            depth: 1.0,
            tri: 9,
        }];
        world.sphere_contacts(Vec3::new(0.0, 0.1, 0.0), 0.2, &mut out);
        let tris: Vec<u32> = out.iter().map(|c| c.tri).collect();
        assert_eq!(tris, [9, 1, 2]);
    }

    #[test]
    fn welder_buckets_vertex_by_weld_step() {
        let mut welder = Welder::default();
        welder.id(Vec3::new(1.0, -0.5, 0.25));
        let keys: Vec<_> = welder.grid.keys().copied().collect();
        assert_eq!(keys, [IVec3::new(1024, -512, 256)]);
    }

    #[test]
    fn welder_beyond_i32_keys_saturates_without_overflow() {
        let mut welder = Welder::default();
        let far = Vec3::new(3.0e6, -3.0e6, 0.0);
        assert_eq!(welder.id(far), welder.id(far));
    }
}
