use std::collections::{BTreeSet, HashMap};

use engine::{
    Aabb3, Assets, Color, Handle, IVec2, Mat4, Model, ModelDraw, StaticWorld, Triangle, Vec3, vec3,
};

use crate::ASSETS;
use crate::pieces::{BARRIER_WALL, CARS, DECOR, KIT_SCALE, PIECES};
use crate::track::{Entry, TrackDraft};

/// Метки треугольников мира.
pub(crate) const ROAD: u32 = 0;
pub(crate) const GRASS: u32 = 1;
pub(crate) const BARRIER: u32 = 2;
pub(crate) const GROUND: u32 = 3;

/// Высота травы деталей уровня 0 и земли, м: корень моделей набора опущен на 0,01 единицы.
pub(crate) const BASE: f32 = -0.01 * KIT_SCALE;
/// Клеток земли от любой клетки дороги.
pub(crate) const GROUND_MARGIN: i32 = 128;
/// Сторона квадрата земли в клетках; квадраты одной сетки стыкуются вершина в вершину.
const CHUNK: i32 = 16;

/// Неизвестный материал — дорога; тест держит список материалов деталей полным.
fn surface(material: &str) -> u32 {
    if material == "grass" { GRASS } else { ROAD }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Road,
    Barrier,
    /// Только рисуется.
    Prop,
}

/// Модель на трассе.
#[derive(Debug)]
pub(crate) struct Placed {
    pub(crate) name: &'static str,
    pub(crate) transform: Mat4,
    pub(crate) kind: Kind,
    /// Барьер приподнятой детали — запись её дороги.
    pub(crate) entry: Entry,
}

/// Модели деталей, декора и машин по имени.
pub(crate) struct Models(HashMap<&'static str, Handle<Model>>);

impl Models {
    pub(crate) fn load(assets: &mut Assets) -> Self {
        let names = (PIECES.iter().map(|p| p.name))
            .chain(DECOR.iter().map(|d| d.name))
            .chain(CARS);
        Self(
            names
                .map(|name| (name, assets.load_model(format!("{ASSETS}{name}.glb"))))
                .collect(),
        )
    }

    pub(crate) fn get(&self, name: &str) -> Handle<Model> {
        self.0[name]
    }
}

/// Барьер вдоль оси `[a, b]` в клетках детали, растянутый по длине оси.
fn wall(frame: Mat4, [a, b]: [Vec3; 2]) -> Mat4 {
    let (a, b) = (frame.transform_point3(a), frame.transform_point3(b));
    let x = (b - a).normalize();
    let z = x.cross(Vec3::Y).normalize();
    let basis = Mat4::from_cols(
        x.extend(0.0),
        z.cross(x).extend(0.0),
        z.extend(0.0),
        ((a + b) / 2.0).extend(1.0),
    );
    // Модель барьера — одна единица набора вдоль X.
    basis
        * Mat4::from_scale(vec3(a.distance(b), KIT_SCALE, KIT_SCALE))
        * Mat4::from_translation(BARRIER_WALL.offset)
}

/// Дороги с барьерами приподнятых деталей, затем декор.
pub(crate) fn place(track: &TrackDraft) -> Vec<Placed> {
    let mut out = Vec::new();
    for (i, r) in track.roads.iter().enumerate() {
        out.push(Placed {
            name: r.piece.name,
            transform: r.transform(),
            kind: Kind::Road,
            entry: Entry::Road(i),
        });
        let frame = r.frame();
        out.extend(r.piece.walls.iter().map(|&axis| Placed {
            name: BARRIER_WALL.name,
            transform: wall(frame, axis),
            kind: Kind::Barrier,
            entry: Entry::Road(i),
        }));
    }
    out.extend(track.decor.iter().enumerate().map(|(i, d)| Placed {
        name: d.piece.name,
        transform: d.transform(),
        kind: if d.piece.solid {
            Kind::Barrier
        } else {
            Kind::Prop
        },
        entry: Entry::Decor(i),
    }));
    out
}

/// Треугольники твёрдых моделей в позе покоя; преобразование узла — как у `ExpandedModels`.
fn triangles(placed: &[Placed], models: &Models, assets: &Assets) -> Vec<Triangle> {
    let mut out = Vec::new();
    for p in placed.iter().filter(|p| p.kind != Kind::Prop) {
        let model = assets.model(models.get(p.name));
        for (node, global) in model.nodes.iter().zip(&model.rest.globals) {
            let m = p.transform * *global;
            for prim in &node.primitives {
                let tag = match p.kind {
                    Kind::Road => surface(&model.materials[prim.material as usize].name),
                    Kind::Barrier | Kind::Prop => BARRIER,
                };
                let g = assets.mesh_geometry(prim.mesh);
                out.extend(g.indices.as_chunks::<3>().0.iter().map(|t| Triangle {
                    verts: t.map(|i| m.transform_point3(g.positions[i as usize])),
                    tag,
                }));
            }
        }
    }
    out
}

/// Плоская трава деталей на высоте земли: в мире её заменяет земля.
fn base_grass(t: &Triangle) -> bool {
    t.tag == GRASS && t.verts.iter().all(|v| (v.y - BASE).abs() < 1e-3)
}

/// Квадраты `CHUNK` общей сетки, задевающие `GROUND_MARGIN` вокруг клеток дорог.
fn ground(track: &TrackDraft) -> Vec<Triangle> {
    let mut chunks = BTreeSet::new();
    for (c, _) in track.road_cells() {
        let lo = (c - GROUND_MARGIN).div_euclid(IVec2::splat(CHUNK));
        let hi = (c + GROUND_MARGIN).div_euclid(IVec2::splat(CHUNK));
        for x in lo.x..=hi.x {
            chunks.extend((lo.y..=hi.y).map(|z| (x, z)));
        }
    }
    let side = CHUNK as f32 * KIT_SCALE;
    (chunks.into_iter())
        .flat_map(|(x, z)| {
            let corner = |dx, dz| vec3((x + dx) as f32 * side, BASE, (z + dz) as f32 * side);
            let [a, b, c, d] = [corner(0, 0), corner(0, 1), corner(1, 1), corner(1, 0)];
            [[a, b, c], [a, c, d]].map(|verts| Triangle { verts, tag: GROUND })
        })
        .collect()
}

/// Цвет травы деталей — им же рисуется земля.
pub(crate) fn grass_color(models: &Models, assets: &Assets) -> Color {
    let model = assets.model(models.get("roadStraight"));
    (model.materials.iter())
        .find(|m| m.name == "grass")
        .expect("трава roadStraight")
        .color
}

/// Трасса для рендера и столкновений.
pub(crate) struct Layout {
    pub(crate) draws: Vec<ModelDraw>,
    pub(crate) world: StaticWorld,
    /// Габариты в мире каждой модели в позе покоя, по порядку `draws`.
    pub(crate) bounds: Vec<(Entry, Aabb3)>,
}

impl Layout {
    pub(crate) fn new(track: &TrackDraft, models: &Models, assets: &Assets) -> Self {
        let placed = place(track);
        Self {
            draws: placed
                .iter()
                .map(|p| ModelDraw {
                    model: models.get(p.name),
                    transform: p.transform,
                    pose: None,
                    tint: Color::WHITE,
                })
                .collect(),
            world: StaticWorld::new(
                KIT_SCALE,
                (triangles(&placed, models, assets).into_iter())
                    .filter(|t| !base_grass(t))
                    .chain(ground(track)),
            ),
            bounds: (placed.iter())
                .map(|p| {
                    let model = assets.model(models.get(p.name));
                    let b = model.bounds().expect("модели набора не пусты");
                    (p.entry, b.transformed(p.transform))
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use engine::{ExpandedModels, IVec2, Vec3, ivec2};

    use super::*;
    use crate::pieces::{LEVEL, Piece, Rot};
    use crate::track::RoadEntry;

    const EPS: f32 = 1e-3;

    fn loaded() -> (Assets, Models) {
        let mut assets = Assets::default();
        let models = Models::load(&mut assets);
        (assets, models)
    }

    fn rots() -> [Rot; 4] {
        [0, 90, 180, 270].map(|d| Rot::from_degrees(d).expect("rot"))
    }

    fn track(roads: Vec<RoadEntry>) -> TrackDraft {
        TrackDraft {
            car: "race".into(),
            roads,
            decor: Vec::new(),
        }
    }

    #[test]
    fn every_model_loads_non_empty() {
        let (assets, models) = loaded();
        assert_eq!(models.0.len(), PIECES.len() + DECOR.len() + CARS.len());
        for (name, &h) in &models.0 {
            let model = assets.model(h);
            assert!(
                model.nodes.iter().any(|n| !n.primitives.is_empty()),
                "{name}"
            );
        }
    }

    #[test]
    fn decor_offset_centers_and_grounds_model_bounds() {
        let (assets, models) = loaded();
        for d in &DECOR {
            let b = assets.model(models.get(d.name)).bounds().expect("габарит");
            let center = (b.min + b.max) / 2.0 + d.offset;
            assert!(center.x.abs() < EPS && center.z.abs() < EPS, "{}", d.name);
            assert!((b.min.y + d.offset.y).abs() < EPS, "{}", d.name);
        }
    }

    fn inside(cell: IVec2, v: Vec3, margin: f32) -> bool {
        let lo = cell.as_vec2() * KIT_SCALE - margin;
        let hi = (cell + 1).as_vec2() * KIT_SCALE + margin;
        (lo.x..=hi.x).contains(&v.x) && (lo.y..=hi.y).contains(&v.z)
    }

    #[test]
    fn piece_stays_in_its_cells_and_level_lifts_two_meters() {
        let (assets, models) = loaded();
        for piece in &PIECES {
            // Стойки арки `roadStart` выходят в соседние клетки, см. таблицу деталей.
            let spill = if piece.name == "roadStart" {
                0.13 * KIT_SCALE + EPS
            } else {
                EPS
            };
            let model = assets.model(models.get(piece.name));
            for rot in rots() {
                let road = RoadEntry {
                    piece,
                    cell: ivec2(3, -2),
                    level: 0,
                    rot,
                };
                let cells: Vec<_> = piece.cells(rot).map(|c| road.cell + c).collect();
                for (node, global) in model.nodes.iter().zip(&model.rest.globals) {
                    let m = road.transform() * *global;
                    for prim in &node.primitives {
                        let asphalt = model.materials[prim.material as usize].name == "road";
                        let margin = if asphalt { EPS } else { spill };
                        let g = assets.mesh_geometry(prim.mesh);
                        for &i in &g.indices {
                            let v = m.transform_point3(g.positions[i as usize]);
                            assert!(
                                cells.iter().any(|&c| inside(c, v, margin)),
                                "{} {rot:?}: {v} вне клеток, асфальт {asphalt}",
                                piece.name
                            );
                        }
                    }
                }
                let up = RoadEntry { level: 1, ..road };
                let lift = Mat4::from_translation(vec3(0.0, 2.0, 0.0));
                assert!(
                    up.transform().abs_diff_eq(lift * road.transform(), 1e-4),
                    "{}",
                    piece.name
                );
            }
        }
    }

    const MIXED: &str = r#"
car = "race"

[[road]]
piece = "roadStraightBridge"
cell = [0, 0]
rot = 90

[[road]]
piece = "roadRampLong"
cell = [2, 0]
rot = 180

[[road]]
piece = "roadCornerLarge"
cell = [0, 3]
rot = 270

[[decor]]
piece = "barrierRed"
pos = [10.0, 0.0, 20.0]
yaw = 30.0

[[decor]]
piece = "treeLarge"
pos = [20.0, 0.0, 20.0]
"#;

    #[test]
    fn render_and_world_share_one_transform() {
        let (assets, models) = loaded();
        let track = TrackDraft::parse(MIXED).expect("трасса");
        let placed = place(&track);
        let layout = Layout::new(&track, &models, &assets);
        assert_eq!(layout.draws.len(), placed.len());
        let solid: Vec<ModelDraw> = (layout.draws.iter().zip(&placed))
            .filter(|(_, p)| p.kind != Kind::Prop)
            .map(|(d, p)| {
                assert_eq!((d.model, d.transform), (models.get(p.name), p.transform));
                d.clone()
            })
            .collect();
        let rendered: Vec<[Vec3; 3]> =
            ExpandedModels::new(&solid, &assets)
                .meshes
                .iter()
                .flat_map(|d| {
                    let g = assets.mesh_geometry(d.mesh);
                    g.indices.as_chunks::<3>().0.iter().map(move |t| {
                        t.map(|i| d.transform.transform_point3(g.positions[i as usize]))
                    })
                })
                .collect();
        let world: Vec<_> = triangles(&placed, &models, &assets)
            .iter()
            .map(|t| t.verts)
            .collect();
        assert_ne!(world.len(), 0);
        assert_eq!(rendered, world);
    }

    #[test]
    fn ground_lies_at_base_and_covers_margin_around_roads() {
        let (assets, models) = loaded();
        let up = RoadEntry {
            piece: &PIECES[0],
            cell: ivec2(0, 0),
            level: 1,
            rot: Rot::default(),
        };
        let world = Layout::new(&track(vec![up]), &models, &assets).world;
        let edge = (GROUND_MARGIN + 1) as f32 * KIT_SCALE - 0.5;
        for (x, z) in [
            (0.1, 2.0),
            (edge, 2.0),
            (2.0, edge),
            (-edge + KIT_SCALE, -edge + KIT_SCALE),
        ] {
            let hit = world.raycast(vec3(x, 1.0, z), Vec3::NEG_Y, 2.0, |_| true);
            assert!(
                hit.is_some_and(|h| h.tag == GROUND && (h.point.y - BASE).abs() < 1e-4),
                "{x} {z}: {hit:?}"
            );
        }
        let beyond = (GROUND_MARGIN + CHUNK + 1) as f32 * KIT_SCALE;
        assert_eq!(down(&world, beyond, 2.0), None);
        assert_eq!(down(&world, 2.0, beyond), None);
        assert_eq!(ground(&track(Vec::new())), []);
    }

    #[test]
    fn only_flat_grass_at_base_is_replaced_and_ground_is_green() {
        let at = |tag, dy| Triangle {
            verts: [
                vec3(0.0, BASE, 0.0),
                vec3(0.0, BASE, 1.0),
                vec3(1.0, BASE + dy, 1.0),
            ],
            tag,
        };
        assert!(base_grass(&at(GRASS, 0.0)));
        assert!(!base_grass(&at(ROAD, 0.0)));
        assert!(!base_grass(&at(GRASS, 0.01)));
        let (assets, models) = loaded();
        let c = grass_color(&models, &assets);
        assert!(c.g > c.r && c.g > c.b, "{c:?}");
    }

    #[test]
    fn grass_of_pieces_never_lies_below_base_and_flat_grass_lies_on_it() {
        let (assets, models) = loaded();
        for piece in &PIECES {
            let road = RoadEntry {
                piece,
                cell: IVec2::ZERO,
                level: 0,
                rot: Rot::default(),
            };
            let tris = triangles(&place(&track(vec![road])), &models, &assets);
            let grass: Vec<_> = tris.iter().filter(|t| t.tag == GRASS).collect();
            for t in &grass {
                assert!(
                    t.verts.iter().all(|v| v.y > BASE - 1e-3),
                    "{}: {t:?}",
                    piece.name
                );
            }
            if piece.name == "roadStraight" {
                assert!(grass.iter().any(|t| base_grass(t)), "{}", piece.name);
            }
        }
    }

    fn down(world: &StaticWorld, x: f32, z: f32) -> Option<u32> {
        world
            .raycast(vec3(x, 20.0, z), Vec3::NEG_Y, 40.0, |_| true)
            .map(|h| h.tag)
    }

    #[test]
    fn tags_follow_material_and_props_stay_out_of_world() {
        let (assets, models) = loaded();
        let track = TrackDraft::parse(
            "car = \"race\"\n\
             [[road]]\npiece = \"roadStraight\"\ncell = [0, 0]\n\
             [[decor]]\npiece = \"barrierWhite\"\npos = [10.0, 0.0, 2.0]\n\
             [[decor]]\npiece = \"treeLarge\"\npos = [20.0, 0.0, 2.0]\n\
             [[decor]]\npiece = \"treeSmall\"\npos = [25.0, 0.0, 2.0]\n\
             [[decor]]\npiece = \"grandStand\"\npos = [30.0, 0.0, 2.0]\n\
             [[decor]]\npiece = \"rail\"\npos = [40.0, 0.0, 2.0]\n\
             [[decor]]\npiece = \"cone\"\npos = [45.0, 0.0, 2.0]\n",
        )
        .expect("трасса");
        let world = Layout::new(&track, &models, &assets).world;
        // Трава — x 0..0,082 клетки, её заменяет земля; бортик — до 0,155, дальше асфальт.
        assert_eq!(down(&world, 2.0, 2.0), Some(ROAD));
        assert_eq!(down(&world, 0.48, 2.0), Some(ROAD));
        assert_eq!(down(&world, 0.1, 2.0), Some(GROUND));
        assert_eq!(down(&world, 10.0, 2.0), Some(BARRIER));
        for x in [20.0, 25.0, 30.0, 40.0, 45.0] {
            assert_eq!(down(&world, x, 2.0), Some(GROUND), "x {x}");
        }
    }

    #[test]
    fn piece_materials_are_known() {
        let (assets, models) = loaded();
        for piece in &PIECES {
            for m in &assets.model(models.get(piece.name)).materials {
                assert!(
                    ["road", "grey", "_defaultMat", "wall", "grass"].contains(&m.name.as_str()),
                    "{}: {}",
                    piece.name,
                    m.name
                );
            }
        }
        assert_eq!(surface("grass"), GRASS);
        for name in ["road", "grey", "_defaultMat", "wall"] {
            assert_eq!(surface(name), ROAD, "{name}");
        }
    }

    fn check_walls(piece: &'static Piece, rot: Rot, assets: &Assets, models: &Models) {
        let road = RoadEntry {
            piece,
            cell: ivec2(1, 1),
            level: 0,
            rot,
        };
        let frame = road.frame();
        let world = Layout::new(&track(vec![road]), models, assets).world;
        let side = frame.transform_vector3(Vec3::X).normalize();
        let length = piece.size.y as f32;
        for z in [0.05, 0.5, length - 0.5, length - 0.05] {
            let at = |x| frame.transform_point3(vec3(x, 2.0 * LEVEL + 1.0, z));
            let deck = world
                .raycast(at(0.5), Vec3::NEG_Y, 20.0, |h| h.tag != BARRIER)
                .expect("настил");
            assert_eq!(deck.tag, ROAD);
            let eye = deck.point + Vec3::Y * 0.2;
            for dir in [side, -side] {
                let hit = world.raycast(eye, dir, KIT_SCALE / 2.0, |_| true);
                let what = format!("{} {rot:?} z {z}", piece.name);
                assert_eq!(hit.map(|h| h.tag), Some(BARRIER), "{what}");
                // Грань у основания — на краю асфальта 0,155 клетки, выше скошена наружу на ~6 см.
                let t = hit.map_or(0.0, |h| h.t);
                assert!((t - (0.5 - 0.155) * KIT_SCALE).abs() < 0.1, "{what}: {t}");
            }
            for x in [0.16, 0.84] {
                let edge = world.raycast(at(x), Vec3::NEG_Y, 20.0, |_| true);
                assert_eq!(
                    edge.map(|h| h.tag),
                    Some(ROAD),
                    "{} барьер на асфальте",
                    piece.name
                );
            }
            // Внешняя полоса на 4 см ниже асфальта: луч снаружи над ней упирается в барьер, щели под ним нет.
            for (x, dir) in [(-0.5, side), (1.5, -side)] {
                let mut origin = frame.transform_point3(vec3(x, 0.0, z));
                origin.y = deck.point.y - 0.02;
                let hit = world.raycast(origin, dir, KIT_SCALE, |_| true);
                assert_eq!(
                    hit.map(|h| h.tag),
                    Some(BARRIER),
                    "{} z {z}: щель",
                    piece.name
                );
            }
        }
    }

    #[test]
    fn raised_pieces_get_side_barriers() {
        let (assets, models) = loaded();
        for piece in PIECES.iter().filter(|p| !p.walls.is_empty()) {
            for rot in rots() {
                check_walls(piece, rot, &assets, &models);
            }
        }
        let flat = track(vec![RoadEntry {
            piece: &PIECES[0],
            cell: IVec2::ZERO,
            level: 0,
            rot: Rot::default(),
        }]);
        assert_eq!(place(&flat).len(), 1);
    }

    #[test]
    fn piece_triangles_wind_with_normals() {
        let mut assets = Assets::default();
        Models::load(&mut assets);
        let meshes = assets.take_pending_meshes();
        assert_ne!(meshes.len(), 0);
        for mesh in meshes {
            for t in mesh.indices.as_chunks::<3>().0 {
                let v = t.map(|i| &mesh.vertices[i as usize]);
                let [a, b, c] = v.map(|v| Vec3::from(v.pos));
                let cross = (b - a).cross(c - a);
                if cross.length() < 1e-8 {
                    continue;
                }
                let normal: Vec3 = v.iter().map(|v| Vec3::from(v.normal)).sum();
                assert!(cross.dot(normal) > 0.0, "{:?}: {a} {b} {c}", mesh.handle);
            }
        }
    }
}
