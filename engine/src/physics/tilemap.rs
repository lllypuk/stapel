use glam::IVec2;

use super::Aabb;

/// Сетка твёрдых клеток `tile × tile`, строка 0 — нижняя.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tilemap {
    tile: i32,
    width: i32,
    height: i32,
    solid: Vec<bool>,
}

impl Tilemap {
    /// Карта `width × height` клеток, `solid` — по строкам снизу вверх.
    ///
    /// # Panics
    /// Если `tile` не положительный или длина `solid` не `width × height`.
    pub fn new(tile: i32, width: i32, height: i32, solid: Vec<bool>) -> Self {
        assert!(tile > 0, "tile {tile}");
        assert_eq!(
            usize::try_from(width * height).ok(),
            Some(solid.len()),
            "{width}×{height}"
        );
        Self {
            tile,
            width,
            height,
            solid,
        }
    }

    /// Задевает ли `aabb` твёрдую клетку; вне карты слева и справа твёрдо, сверху и снизу пусто.
    pub fn overlaps_solid(&self, aabb: &Aabb) -> bool {
        let lo = aabb.min.div_euclid(IVec2::splat(self.tile));
        let hi = (aabb.max() - 1).div_euclid(IVec2::splat(self.tile));
        (lo.y..=hi.y).any(|cy| (lo.x..=hi.x).any(|cx| self.solid_at(cx, cy)))
    }

    #[expect(
        clippy::cast_sign_loss,
        reason = "индекс неотрицателен после проверок границ"
    )]
    fn solid_at(&self, cx: i32, cy: i32) -> bool {
        if cx < 0 || cx >= self.width {
            return true;
        }
        if cy < 0 || cy >= self.height {
            return false;
        }
        self.solid[(cy * self.width + cx) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::ivec2;

    const T: i32 = 18;

    /// 3×2: снизу `#..`, сверху `..#`.
    fn map() -> Tilemap {
        Tilemap::new(T, 3, 2, vec![true, false, false, false, false, true])
    }

    fn cell_box(cx: i32, cy: i32) -> Aabb {
        Aabb {
            min: ivec2(cx * T, cy * T),
            size: ivec2(T, T),
        }
    }

    #[test]
    fn solid_cells_follow_rows_bottom_up() {
        let m = map();
        for (cx, cy, solid) in [
            (0, 0, true),
            (1, 0, false),
            (2, 0, false),
            (0, 1, false),
            (1, 1, false),
            (2, 1, true),
        ] {
            assert_eq!(m.overlaps_solid(&cell_box(cx, cy)), solid, "({cx}, {cy})");
        }
    }

    #[test]
    fn box_spanning_cells_hits_any_solid_inside() {
        let b = Aabb {
            min: ivec2(T + 1, 1),
            size: ivec2(2 * T, 2 * T - 2),
        };
        assert!(map().overlaps_solid(&b));
    }

    #[test]
    fn box_up_to_cell_border_misses_neighbor() {
        let m = map();
        let up_to_wall = Aabb {
            min: ivec2(T, T),
            size: ivec2(T, T),
        };
        assert!(!m.overlaps_solid(&up_to_wall));
        assert!(m.overlaps_solid(&Aabb {
            size: ivec2(T + 1, T),
            ..up_to_wall
        }));
        let above_floor = Aabb {
            min: ivec2(0, T),
            size: ivec2(T, T),
        };
        assert!(!m.overlaps_solid(&above_floor));
        assert!(m.overlaps_solid(&Aabb {
            min: ivec2(0, T - 1),
            ..above_floor
        }));
    }

    #[test]
    fn negative_coords_floor_into_outside_cells() {
        let m = Tilemap::new(T, 1, 1, vec![false]);
        let left = Aabb {
            min: ivec2(-1, 0),
            size: ivec2(1, 1),
        };
        assert!(m.overlaps_solid(&left));
        let floor = Tilemap::new(T, 1, 1, vec![true]);
        let below = Aabb {
            min: ivec2(0, -1),
            size: ivec2(1, 1),
        };
        assert!(!floor.overlaps_solid(&below));
    }

    #[test]
    fn outside_left_and_right_is_solid_corners_too() {
        let m = map();
        for (cx, cy) in [(-1, 0), (-1, 1), (3, 0), (3, 1), (-1, -1), (3, 5), (-2, 7)] {
            assert!(m.overlaps_solid(&cell_box(cx, cy)), "({cx}, {cy})");
        }
    }

    #[test]
    fn outside_above_and_below_is_empty() {
        let m = map();
        for (cx, cy) in [(0, -1), (1, -3), (2, 2), (0, 9)] {
            assert!(!m.overlaps_solid(&cell_box(cx, cy)), "({cx}, {cy})");
        }
    }

    #[test]
    #[should_panic(expected = "3×2")]
    fn wrong_cell_count_panics() {
        let _ = Tilemap::new(T, 3, 2, vec![false; 5]);
    }

    #[test]
    #[should_panic(expected = "tile 0")]
    fn zero_tile_panics() {
        let _ = Tilemap::new(0, 1, 1, vec![false]);
    }
}
