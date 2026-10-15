use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};

use crate::mesh::Mesh;
use crate::{Camera3D, Color, Handle, Texture};

/// Сколько точечных источников доходит до шейдера; остальные отбрасываются.
pub const MAX_LIGHTS: usize = 16;

/// 3D-кадр: рисуется до спрайтов на всю поверхность.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// Камера; aspect — поверхности.
    pub camera: Camera3D,
    /// Линейный свет без источников; к нему прибавляются `lights`.
    pub ambient: Color,
    /// Туман по расстоянию от камеры.
    pub fog: Fog,
    /// Точечные источники; рисуются первые `MAX_LIGHTS`.
    pub lights: Vec<PointLight>,
    /// Меши кадра.
    pub meshes: Vec<MeshDraw>,
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
    /// Меш → мир.
    pub transform: Mat4,
}

/// Данные одного меша в буфере инстансов.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct MeshInstance {
    /// Меш → мир, по столбцам.
    pub model: [[f32; 4]; 4],
    /// Обратная транспонированная к `model` без переноса, по столбцам.
    pub normal: [[f32; 3]; 3],
}

impl From<&MeshDraw> for MeshInstance {
    fn from(d: &MeshDraw) -> Self {
        Self {
            model: d.transform.to_cols_array_2d(),
            normal: Mat3::from_mat4(d.transform)
                .inverse()
                .transpose()
                .to_cols_array_2d(),
        }
    }
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

/// Сортирует по `(mesh, texture)` и режет на батчи; порядок внутри 3D решает глубина.
pub fn batch_meshes(draws: &[MeshDraw]) -> (Vec<MeshInstance>, Vec<MeshBatch>) {
    let mut order: Vec<&MeshDraw> = draws.iter().collect();
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
        instances.push(MeshInstance::from(d));
    }
    (instances, batches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, vec3};

    fn draw(mesh: u32, texture: u32, x: f32) -> MeshDraw {
        MeshDraw {
            mesh: Handle::new(mesh),
            texture: Handle::new(texture),
            transform: Mat4::from_translation(vec3(x, 0.0, 0.0)),
        }
    }

    #[test]
    fn normal_matrix_is_inverse_transpose() {
        let transform = Mat4::from_translation(vec3(5.0, 6.0, 7.0))
            * Mat4::from_rotation_y(0.7)
            * Mat4::from_scale(vec3(4.0, 1.0, 0.5));
        let inst = MeshInstance::from(&MeshDraw {
            transform,
            ..draw(0, 0, 0.0)
        });
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
        let inst = MeshInstance::from(&draw(0, 0, 9.0));
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
    fn no_draws_no_batches() {
        assert_eq!(batch_meshes(&[]), (Vec::new(), Vec::new()));
    }
}
