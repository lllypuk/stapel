use glam::camera::rh::proj::directx;
use glam::{Mat4, Vec2};

/// Ортокамера: прямоугольник мира `size` с центром `center`, Y вверх.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera2D {
    /// Центр видимой области в мире.
    pub center: Vec2,
    /// Ширина и высота видимой области в мировых единицах.
    pub size: Vec2,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            center: Vec2::ZERO,
            size: Vec2::splat(2.0),
        }
    }
}

impl Camera2D {
    /// Мир → NDC; `z = 0` попадает в глубину 0.5.
    pub fn view_proj(&self) -> Mat4 {
        let min = self.center - self.size / 2.0;
        let max = self.center + self.size / 2.0;
        directx::orthographic(min.x, max.x, min.y, max.y, -1.0, 1.0)
    }

    /// Отношение ширины видимой области к высоте.
    pub fn aspect(&self) -> f32 {
        self.size.x / self.size.y
    }
}

/// Прямоугольник кадра в пикселях, начало — левый верхний угол.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Левый край.
    pub x: f32,
    /// Верхний край.
    pub y: f32,
    /// Ширина.
    pub w: f32,
    /// Высота.
    pub h: f32,
}

/// Наибольший вьюпорт пропорции `world_aspect`, вписанный по центру в `surface`.
pub fn letterbox(surface: (u32, u32), world_aspect: f32) -> Viewport {
    let (sw, sh) = (surface.0 as f32, surface.1 as f32);
    let (w, h) = if sw / sh > world_aspect {
        (sh * world_aspect, sh)
    } else {
        (sw, sw / world_aspect)
    };
    Viewport {
        x: (sw - w) / 2.0,
        y: (sh - h) / 2.0,
        w,
        h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, vec2};

    const EPS: f32 = 1e-5;

    fn pong() -> Camera2D {
        Camera2D {
            center: Vec2::ZERO,
            size: vec2(640.0, 360.0),
        }
    }

    fn ndc(cam: &Camera2D, p: Vec2) -> Vec3 {
        cam.view_proj().project_point3(p.extend(0.0))
    }

    #[test]
    fn world_corners_map_to_ndc_corners() {
        let cam = pong();
        for (world, expected) in [
            (vec2(-320.0, -180.0), vec2(-1.0, -1.0)),
            (vec2(320.0, -180.0), vec2(1.0, -1.0)),
            (vec2(-320.0, 180.0), vec2(-1.0, 1.0)),
            (vec2(320.0, 180.0), vec2(1.0, 1.0)),
        ] {
            let got = ndc(&cam, world).truncate();
            assert!(got.abs_diff_eq(expected, EPS), "{world} → {got}");
        }
    }

    #[test]
    fn z_zero_is_mid_depth() {
        assert!((ndc(&pong(), Vec2::ZERO).z - 0.5).abs() < EPS);
    }

    #[test]
    fn y_points_up() {
        assert!((ndc(&pong(), vec2(0.0, 180.0)).y - 1.0).abs() < EPS);
    }

    #[test]
    fn aspect_is_width_over_height() {
        assert!((pong().aspect() - 16.0 / 9.0).abs() < EPS);
    }

    #[test]
    fn offset_center_moves_world() {
        let cam = Camera2D {
            center: vec2(100.0, 50.0),
            ..pong()
        };
        assert!(
            ndc(&cam, vec2(100.0, 50.0))
                .truncate()
                .abs_diff_eq(Vec2::ZERO, EPS)
        );
    }

    #[test]
    fn default_camera_is_identity_ndc() {
        let p = vec2(0.3, -0.7);
        assert!(ndc(&Camera2D::default(), p).truncate().abs_diff_eq(p, EPS));
    }

    #[test]
    fn wider_window_gets_side_bars() {
        let v = letterbox((1000, 360), 640.0 / 360.0);
        assert_eq!(
            v,
            Viewport {
                x: 180.0,
                y: 0.0,
                w: 640.0,
                h: 360.0
            }
        );
    }

    #[test]
    fn taller_window_gets_top_bottom_bars() {
        let v = letterbox((640, 560), 640.0 / 360.0);
        assert_eq!(
            v,
            Viewport {
                x: 0.0,
                y: 100.0,
                w: 640.0,
                h: 360.0
            }
        );
    }

    #[test]
    fn same_aspect_fills_frame() {
        let v = letterbox((1280, 720), 640.0 / 360.0);
        assert_eq!(
            v,
            Viewport {
                x: 0.0,
                y: 0.0,
                w: 1280.0,
                h: 720.0
            }
        );
    }

    #[test]
    fn tiny_window_viewport_not_empty() {
        let v = letterbox((1, 1), 640.0 / 360.0);
        assert!(v.w > 0.0 && v.h > 0.0, "{v:?}");
        assert!(v.x >= 0.0 && v.y >= 0.0 && v.x + v.w <= 1.0 && v.y + v.h <= 1.0);
    }
}
