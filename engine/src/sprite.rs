use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Vec2;

use crate::{Color, TextureId};

/// Осевой прямоугольник с текстурой; `pos` — центр в мировых единицах.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub texture: TextureId,
    pub pos: Vec2,
    pub size: Vec2,
    /// Линейный, не премультиплицированный.
    pub color: Color,
    pub layer: i32,
}

/// Данные одного спрайта в буфере инстансов.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Instance {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub color: [f32; 4],
}

/// Подряд идущие инстансы одной текстуры — один draw call.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub texture: TextureId,
    pub range: Range<u32>,
}

impl From<&Sprite> for Instance {
    fn from(s: &Sprite) -> Self {
        let c = s.color;
        Self {
            pos: s.pos.into(),
            size: s.size.into(),
            color: [c.r as f32, c.g as f32, c.b as f32, c.a as f32],
        }
    }
}

/// Сортирует по `(layer, texture)` с сохранением порядка вызовов и режет на батчи.
pub fn batch(sprites: &[Sprite]) -> (Vec<Instance>, Vec<Batch>) {
    let mut order: Vec<usize> = (0..sprites.len()).collect();
    order.sort_by_key(|&i| (sprites[i].layer, sprites[i].texture));

    let mut instances = Vec::with_capacity(order.len());
    let mut batches: Vec<Batch> = Vec::new();
    for i in order {
        let s = &sprites[i];
        let n = instances.len() as u32;
        match batches.last_mut() {
            Some(b) if b.texture == s.texture => b.range.end = n + 1,
            _ => batches.push(Batch {
                texture: s.texture,
                range: n..n + 1,
            }),
        }
        instances.push(Instance::from(s));
    }
    (instances, batches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::vec2;

    fn sprite(texture: u32, layer: i32, x: f32) -> Sprite {
        Sprite {
            texture: TextureId(texture),
            pos: vec2(x, 0.0),
            size: Vec2::ONE,
            color: Color::BLACK,
            layer,
        }
    }

    fn keys(sprites: &[Sprite]) -> Vec<(i32, u32, f32)> {
        let (instances, batches) = batch(sprites);
        batches
            .iter()
            .flat_map(|b| {
                instances[b.range.start as usize..b.range.end as usize]
                    .iter()
                    .map(|i| {
                        let s = sprites.iter().find(|s| s.pos.x == i.pos[0]).unwrap();
                        (s.layer, b.texture.0, i.pos[0])
                    })
            })
            .collect()
    }

    #[test]
    fn sorts_by_layer_first() {
        let s = [sprite(0, 2, 0.0), sprite(1, 1, 1.0)];
        assert_eq!(keys(&s), [(1, 1, 1.0), (2, 0, 0.0)]);
    }

    #[test]
    fn sorts_by_texture_within_layer() {
        let s = [sprite(2, 0, 0.0), sprite(1, 0, 1.0)];
        assert_eq!(keys(&s), [(0, 1, 1.0), (0, 2, 0.0)]);
    }

    #[test]
    fn equal_keys_keep_call_order() {
        let s = [sprite(1, 0, 3.0), sprite(1, 0, 1.0), sprite(1, 0, 2.0)];
        assert_eq!(keys(&s), [(0, 1, 3.0), (0, 1, 1.0), (0, 1, 2.0)]);
    }

    #[test]
    fn ranges_split_on_texture_change_and_cover_all() {
        let s = [
            sprite(1, 0, 0.0),
            sprite(2, 0, 1.0),
            sprite(1, 0, 2.0),
            sprite(1, 1, 3.0),
            sprite(2, 1, 4.0),
        ];
        let (instances, batches) = batch(&s);
        let got: Vec<_> = batches.iter().map(|b| (b.texture.0, b.range.clone())).collect();
        assert_eq!(got, [(1, 0..2), (2, 2..3), (1, 3..4), (2, 4..5)]);
        assert_eq!(instances.len(), s.len());
    }

    #[test]
    fn empty_input_gives_nothing() {
        assert_eq!(batch(&[]), (vec![], vec![]));
    }

    #[test]
    fn instance_carries_color_as_f32() {
        let mut s = sprite(0, 0, 5.0);
        s.size = vec2(2.0, 3.0);
        s.color = Color {
            r: 0.25,
            g: 0.5,
            b: 0.75,
            a: 0.125,
        };
        let (instances, _) = batch(&[s]);
        assert_eq!(
            instances,
            [Instance {
                pos: [5.0, 0.0],
                size: [2.0, 3.0],
                color: [0.25, 0.5, 0.75, 0.125],
            }]
        );
    }
}
