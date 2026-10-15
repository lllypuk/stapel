use glam::IVec2;

/// Прямоугольник `[min, min + size)` в целых мировых пикселях, `size` положительный.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aabb {
    /// Левый нижний угол, включительно.
    pub min: IVec2,
    /// Ширина и высота.
    pub size: IVec2,
}

impl Aabb {
    /// Правый верхний угол, исключительно.
    pub fn max(&self) -> IVec2 {
        self.min + self.size
    }

    /// Строгое пересечение: касание гранью или углом — не пересечение.
    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.min.cmplt(other.max()).all() && other.min.cmplt(self.max()).all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::ivec2;

    fn square(x: i32, y: i32) -> Aabb {
        Aabb {
            min: ivec2(x, y),
            size: ivec2(10, 10),
        }
    }

    #[test]
    fn max_is_min_plus_size() {
        assert_eq!(square(3, -4).max(), ivec2(13, 6));
    }

    #[test]
    fn overlapping_boxes_overlap_both_ways() {
        let (a, b) = (square(0, 0), square(9, 9));
        assert!(a.overlaps(&b) && b.overlaps(&a));
    }

    #[test]
    fn contained_box_overlaps() {
        let inner = Aabb {
            min: ivec2(2, 2),
            size: ivec2(1, 1),
        };
        assert!(square(0, 0).overlaps(&inner) && inner.overlaps(&square(0, 0)));
    }

    #[test]
    fn edge_touch_is_not_overlap() {
        let a = square(0, 0);
        for b in [square(10, 0), square(-10, 0), square(0, 10), square(0, -10)] {
            assert!(!a.overlaps(&b) && !b.overlaps(&a), "{b:?}");
        }
    }

    #[test]
    fn corner_touch_is_not_overlap() {
        let a = square(0, 0);
        for b in [
            square(10, 10),
            square(-10, -10),
            square(10, -10),
            square(-10, 10),
        ] {
            assert!(!a.overlaps(&b) && !b.overlaps(&a), "{b:?}");
        }
    }

    #[test]
    fn overlap_on_one_axis_only_is_not_overlap() {
        assert!(!square(0, 0).overlaps(&square(5, 20)));
        assert!(!square(0, 0).overlaps(&square(20, 5)));
    }
}
