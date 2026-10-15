use bytemuck::{Pod, Zeroable};

use crate::Handle;

/// Маркер хэндла меша.
pub enum Mesh {}

/// Вершина меша; `joints` и `weights` читает только скиннинг.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    /// Позиция в пространстве меша.
    pub pos: [f32; 3],
    /// Нормаль в пространстве меша.
    pub normal: [f32; 3],
    /// Координаты текстуры, `(0, 0)` — левый верхний угол.
    pub uv: [f32; 2],
    /// Индексы суставов скина.
    pub joints: [u16; 4],
    /// Веса суставов, в сумме 1.
    pub weights: [f32; 4],
}

/// Меш, ждущий заливки на GPU.
#[derive(Debug, PartialEq)]
pub struct PendingMesh {
    /// Чей это хэндл.
    pub handle: Handle<Mesh>,
    /// Вершины.
    pub vertices: Vec<Vertex>,
    /// Треугольники по три индекса.
    pub indices: Vec<u32>,
}
