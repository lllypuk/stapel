use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};

use crate::mesh::Mesh;
use crate::model::{Model, Pose};
use crate::{Assets, Camera3D, Color, Handle, Texture};

/// Сколько точечных источников доходит до шейдера; остальные отбрасываются.
pub const MAX_LIGHTS: usize = 16;
/// Сколько суставов в палитре доходит до шейдера; скин длиннее не рисуется.
pub const MAX_JOINTS: usize = 64;

/// 3D-кадр: рисуется до спрайтов на всю поверхность.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// Камера; aspect — поверхности.
    pub camera: Camera3D,
    /// Линейный свет без источников; к нему прибавляются `lights`.
    pub ambient: Color,
    /// Туман по расстоянию от камеры.
    pub fog: Fog,
    /// Направленный свет; `None` — без него.
    pub sun: Option<DirectionalLight>,
    /// Точечные источники; рисуются первые `MAX_LIGHTS`.
    pub lights: Vec<PointLight>,
    /// Меши кадра.
    pub meshes: Vec<MeshDraw>,
    /// Модели кадра.
    pub models: Vec<ModelDraw>,
}

/// Свет из бесконечности без затухания, Ламберт; теней нет.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalLight {
    /// Куда идут лучи, в мире; длина не важна, нулевой — без света.
    pub dir: Vec3,
    /// Линейный цвет.
    pub color: Color,
}

/// Точечный источник с затуханием `max(1 - d / range, 0)²` и Ламбертом.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    /// Позиция в мире.
    pub pos: Vec3,
    /// Линейный цвет у самого источника.
    pub color: Color,
    /// Расстояние, где свет гаснет до нуля.
    pub range: f32,
}

/// Линейный переход к `color` от `start` до `end` по расстоянию от камеры.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    /// Цвет полностью затуманенной точки.
    pub color: Color,
    /// Ближе — без тумана.
    pub start: f32,
    /// Дальше — только туман.
    pub end: f32,
}

impl Default for Fog {
    /// Без тумана.
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            start: f32::MAX,
            end: f32::MAX,
        }
    }
}

/// Меш с текстурой в мире.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshDraw {
    /// Что рисовать.
    pub mesh: Handle<Mesh>,
    /// Текстура по UV меша.
    pub texture: Handle<Texture>,
    /// Линейный множитель текстуры.
    pub color: Color,
    /// Меш → мир.
    pub transform: Mat4,
}

/// Модель в мире.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelDraw {
    /// Что рисовать.
    pub model: Handle<Model>,
    /// Модель → мир.
    pub transform: Mat4,
    /// Поза от `Model::pose` этой же модели, иначе паника при раскрытии; `None` — `Model::rest`.
    pub pose: Option<Pose>,
}

/// Меш со скином: вершины смешивают матрицы `palettes[palette]` своего `ExpandedModels`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinnedDraw {
    /// Что рисовать.
    pub mesh: Handle<Mesh>,
    /// Текстура по UV меша.
    pub texture: Handle<Texture>,
    /// Линейный множитель текстуры.
    pub color: Color,
    /// Модель → мир; матрица узла меша не применяется, позу задают суставы.
    pub transform: Mat4,
    /// Индекс в `ExpandedModels::palettes`.
    pub palette: usize,
}

/// Меши моделей кадра: жёсткие идут в общие батчи с `Scene::meshes`, со скином — вызовом на меш.
#[derive(Debug, Default, PartialEq)]
pub struct ExpandedModels {
    /// Жёсткие меши: модель → мир × узел меша.
    pub meshes: Vec<MeshDraw>,
    /// Меши со скином.
    pub skinned: Vec<SkinnedDraw>,
    /// Палитры суставов: своя на каждую пару «модель кадра — скин».
    pub palettes: Vec<Vec<Mat4>>,
}

impl ExpandedModels {
    /// Раскрывает модели кадра.
    pub fn new(draws: &[ModelDraw], assets: &Assets) -> Self {
        let mut out = Self::default();
        for d in draws {
            out.push(assets.model(d.model), d);
        }
        out
    }

    /// Добавляет меши `model`, нарисованной как `draw`.
    pub fn push(&mut self, model: &Model, draw: &ModelDraw) {
        let pose = draw.pose.as_ref().unwrap_or(&model.rest);
        let base = self.palettes.len();
        self.palettes.extend_from_slice(&pose.palettes);
        for (node, global) in model.nodes.iter().zip(&pose.globals) {
            for p in &node.primitives {
                let color = model.materials[p.material as usize].color;
                match node.skin {
                    Some(skin) => self.skinned.push(SkinnedDraw {
                        mesh: p.mesh,
                        texture: p.texture,
                        color,
                        transform: draw.transform,
                        palette: base + skin,
                    }),
                    None => self.meshes.push(MeshDraw {
                        mesh: p.mesh,
                        texture: p.texture,
                        color,
                        transform: draw.transform * *global,
                    }),
                }
            }
        }
    }
}

/// Данные одного меша в буфере инстансов.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct MeshInstance {
    /// Меш → мир, по столбцам.
    pub model: [[f32; 4]; 4],
    /// Обратная транспонированная к `model` без переноса, по столбцам.
    pub normal: [[f32; 3]; 3],
    /// Линейный множитель текстуры.
    pub tint: [f32; 4],
}

impl MeshInstance {
    /// Инстанс меша с `transform` и множителем `color`.
    pub fn new(transform: Mat4, color: Color) -> Self {
        Self {
            model: transform.to_cols_array_2d(),
            normal: Mat3::from_mat4(transform)
                .inverse()
                .transpose()
                .to_cols_array_2d(),
            tint: rgba(color),
        }
    }
}

#[expect(clippy::cast_possible_truncation, reason = "GPU берёт цвет в f32")]
pub(crate) fn rgba(c: Color) -> [f32; 4] {
    [c.r as f32, c.g as f32, c.b as f32, c.a as f32]
}

/// Подряд идущие инстансы одного меша с одной текстурой — один draw call.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshBatch {
    /// Меш батча.
    pub mesh: Handle<Mesh>,
    /// Текстура батча.
    pub texture: Handle<Texture>,
    /// Индексы инстансов батча.
    pub range: Range<u32>,
}

/// Сортирует по `(mesh, texture)` и режет на батчи; цвет в ключ не входит — он в инстансе.
pub fn batch_meshes<'a>(
    draws: impl IntoIterator<Item = &'a MeshDraw>,
) -> (Vec<MeshInstance>, Vec<MeshBatch>) {
    let mut order: Vec<&MeshDraw> = draws.into_iter().collect();
    order.sort_by_key(|d| (d.mesh, d.texture));

    let mut instances = Vec::with_capacity(order.len());
    let mut batches: Vec<MeshBatch> = Vec::new();
    for d in order {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "u32::MAX мешей не поместится в память"
        )]
        let n = instances.len() as u32;
        match batches.last_mut() {
            Some(b) if (b.mesh, b.texture) == (d.mesh, d.texture) => b.range.end = n + 1,
            _ => batches.push(MeshBatch {
                mesh: d.mesh,
                texture: d.texture,
                range: n..n + 1,
            }),
        }
        instances.push(MeshInstance::new(d.transform, d.color));
    }
    (instances, batches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::{patched, testdata};
    use glam::{Vec3, vec3};

    fn draw(mesh: u32, texture: u32, x: f32) -> MeshDraw {
        MeshDraw {
            mesh: Handle::new(mesh),
            texture: Handle::new(texture),
            color: Color::WHITE,
            transform: Mat4::from_translation(vec3(x, 0.0, 0.0)),
        }
    }

    #[test]
    fn normal_matrix_is_inverse_transpose() {
        let transform = Mat4::from_translation(vec3(5.0, 6.0, 7.0))
            * Mat4::from_rotation_y(0.7)
            * Mat4::from_scale(vec3(4.0, 1.0, 0.5));
        let inst = MeshInstance::new(transform, Color::WHITE);
        assert_eq!(inst.model, transform.to_cols_array_2d());

        // Грань наклонена 45° в плоскости XY; растяжение по X её поворачивает, нормаль должна остаться перпендикулярной.
        let tangent = vec3(1.0, -1.0, 0.0);
        let normal = vec3(1.0, 1.0, 0.0);
        let world_tangent = transform.transform_vector3(tangent);
        let world_normal = Mat3::from_cols_array_2d(&inst.normal) * normal;
        assert!(
            world_tangent.dot(world_normal).abs() < 1e-5,
            "{world_normal} not perpendicular to {world_tangent}"
        );
        let through_model = transform.transform_vector3(normal);
        assert!(world_tangent.dot(through_model).abs() > 0.1);
    }

    #[test]
    fn translation_does_not_reach_normal() {
        let inst = MeshInstance::new(draw(0, 0, 9.0).transform, Color::WHITE);
        assert_eq!(Mat3::from_cols_array_2d(&inst.normal) * Vec3::Y, Vec3::Y);
    }

    #[test]
    fn batches_by_mesh_then_texture_keeping_call_order() {
        let draws = [
            draw(1, 0, 0.0),
            draw(0, 2, 1.0),
            draw(0, 1, 2.0),
            draw(1, 0, 3.0),
            draw(0, 2, 4.0),
        ];
        let (instances, batches) = batch_meshes(&draws);
        let keys: Vec<_> = batches
            .iter()
            .map(|b| (b.mesh.index, b.texture.index, b.range.clone()))
            .collect();
        assert_eq!(keys, [(0, 1, 0..1), (0, 2, 1..3), (1, 0, 3..5)]);
        let xs: Vec<f32> = instances.iter().map(|i| i.model[3][0]).collect();
        assert_eq!(xs, [2.0, 1.0, 4.0, 0.0, 3.0]);
    }

    #[test]
    fn colors_share_a_batch_and_reach_tint() {
        let red = Color {
            r: 0.5,
            g: 0.25,
            b: 0.125,
            a: 0.75,
        };
        let draws = [
            MeshDraw {
                color: red,
                ..draw(0, 0, 0.0)
            },
            draw(0, 0, 1.0),
        ];
        let (instances, batches) = batch_meshes(&draws);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].range, 0..2);
        let tints: Vec<_> = instances.iter().map(|i| i.tint).collect();
        assert_eq!(tints, [[0.5, 0.25, 0.125, 0.75], [1.0; 4]]);
    }

    #[test]
    fn no_draws_no_batches() {
        assert_eq!(batch_meshes(&[]), (Vec::new(), Vec::new()));
    }

    fn model_draw(model: Handle<Model>, transform: Mat4, pose: Option<Pose>) -> ModelDraw {
        ModelDraw {
            model,
            transform,
            pose,
        }
    }

    #[test]
    fn rigid_mesh_takes_node_from_pose_or_rest() {
        let mut assets = Assets::default();
        let h = assets.load_model(testdata("gate.glb"));
        let gate = assets.model(h);
        let mut open = Pose::default();
        gate.pose(0, 1.0, false, &mut open);
        let at = Mat4::from_translation(vec3(5.0, 0.0, 0.0));
        let out = ExpandedModels::new(
            &[model_draw(h, at, None), model_draw(h, at, Some(open))],
            &assets,
        );
        let door = gate.nodes[1].primitives[0];
        let [rest, opened] = &out.meshes[..] else {
            panic!("{:?}", out.meshes)
        };
        assert_eq!(
            (rest.mesh, rest.texture, rest.color),
            (door.mesh, door.texture, Color::WHITE)
        );
        assert_eq!(rest.transform, at * gate.rest.globals[1]);
        let lift = opened.transform.w_axis - rest.transform.w_axis;
        assert!((lift.y - 0.75).abs() < 1e-3, "{lift}");
        assert_eq!(out.skinned, []);
        assert_eq!(out.palettes, Vec::<Vec<Mat4>>::new());
    }

    #[test]
    fn skinned_mesh_ignores_its_node_transform() {
        let bytes = patched(
            "character-orc.glb",
            &[
                (
                    r#""skin":0,"mesh":0,"#,
                    r#""skin":0,"mesh":0,"translation":[0,5,0],"#,
                ),
                (
                    r#""metallicFactor":0.0"#,
                    r#""baseColorFactor":[0.5,0.25,1,1]"#,
                ),
            ],
        );
        let mut assets = Assets::default();
        let orc = crate::model::parse(&bytes)
            .unwrap()
            .upload(&mut assets, std::path::Path::new(""));
        assert_ne!(orc.rest.globals[8], Mat4::IDENTITY);
        let at = Mat4::from_rotation_y(1.0);
        let mut out = ExpandedModels::default();
        out.push(&orc, &model_draw(Handle::new(0), at, None));
        assert_eq!(out.meshes, []);
        let body = orc.nodes[8].primitives[0];
        assert_eq!(
            out.skinned[0],
            SkinnedDraw {
                mesh: body.mesh,
                texture: body.texture,
                color: Color {
                    r: 0.5,
                    g: 0.25,
                    b: 1.0,
                    a: 1.0,
                },
                transform: at,
                palette: 0,
            }
        );
        assert_eq!(out.palettes, orc.rest.palettes);
    }

    #[test]
    fn rigid_mesh_takes_its_material_color() {
        let mut assets = Assets::default();
        let h = assets.load_model(testdata("racing/roadStraight.glb"));
        let road = assets.model(h);
        let out = ExpandedModels::new(&[model_draw(h, Mat4::IDENTITY, None)], &assets);
        let colors: Vec<_> = out.meshes.iter().map(|d| d.color).collect();
        let materials: Vec<_> = road.materials.iter().map(|m| m.color).collect();
        assert_eq!(colors, materials);
        assert_ne!(colors[0], colors[2]);
    }

    #[test]
    fn each_instance_gets_its_own_palettes() {
        let mut assets = Assets::default();
        let h = assets.load_model(testdata("character-orc.glb"));
        let orc = assets.model(h);
        let walk = orc.animations.iter().position(|a| a.name == "walk");
        let mut pose = Pose::default();
        orc.pose(walk.unwrap(), 0.2, true, &mut pose);
        let out = ExpandedModels::new(
            &[
                model_draw(h, Mat4::IDENTITY, None),
                model_draw(h, Mat4::IDENTITY, Some(pose.clone())),
            ],
            &assets,
        );
        let palettes: Vec<_> = out.skinned.iter().map(|d| d.palette).collect();
        assert_eq!(palettes, [0, 1, 2, 3]);
        assert_eq!(out.palettes[..2], orc.rest.palettes);
        assert_eq!(out.palettes[2..], pose.palettes);
        assert_ne!(out.palettes[0], out.palettes[2]);
    }
}
