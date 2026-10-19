use glam::camera::rh::proj::directx;
use glam::camera::rh::view::look_to_mat4;
use glam::{Mat4, Vec2, Vec3};

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

/// Предел `|pitch|`: на `π/2` взгляд параллелен «верху» и вид вырождается.
pub const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

/// 3D-камера, Y вверх; yaw 0 смотрит по −Z, положительный — влево.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera3D {
    /// Позиция глаза.
    pub pos: Vec3,
    /// Поворот вокруг +Y, радианы.
    pub yaw: f32,
    /// Наклон вверх, радианы; зажимается в `±MAX_PITCH`.
    pub pitch: f32,
    /// Вертикальный угол обзора, радианы.
    pub fov_y: f32,
    /// Ближняя плоскость отсечения.
    pub near: f32,
    /// Дальняя плоскость отсечения.
    pub far: f32,
    /// Видимая высота кадра в мире для ортографии; `None` — перспектива по `fov_y`.
    pub ortho: Option<f32>,
}

impl Default for Camera3D {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 70f32.to_radians(),
            near: 0.05,
            far: 100.0,
            ortho: None,
        }
    }
}

impl Camera3D {
    /// Единичное направление взгляда.
    pub fn forward(&self) -> Vec3 {
        let pitch = self.pitch.clamp(-MAX_PITCH, MAX_PITCH);
        Vec3::new(
            -self.yaw.sin() * pitch.cos(),
            pitch.sin(),
            -self.yaw.cos() * pitch.cos(),
        )
    }

    /// Ставит глаз в `pos` взглядом на `target`; цель строго сверху или снизу оставляет `yaw`.
    pub fn look_at(&mut self, pos: Vec3, target: Vec3) {
        self.pos = pos;
        let d = target - pos;
        let flat = d.x.hypot(d.z);
        if flat > 0.0 {
            self.yaw = (-d.x).atan2(-d.z);
        }
        self.pitch = d.y.atan2(flat);
    }

    /// Мир → NDC с глубиной `near` → 0, `far` → 1.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let proj = match self.ortho {
            Some(height) => {
                let (w, h) = (height * aspect / 2.0, height / 2.0);
                directx::orthographic(-w, w, -h, h, self.near, self.far)
            }
            None => directx::perspective(self.fov_y, aspect, self.near, self.far),
        };
        proj * look_to_mat4(self.pos, self.forward(), Vec3::Y)
    }

    /// Луч из точки `ndc` экрана (Y вверх) от ближней плоскости вглубь.
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> Ray {
        let inv = self.view_proj(aspect).inverse();
        let near = inv.project_point3(ndc.extend(0.0));
        let far = inv.project_point3(ndc.extend(1.0));
        Ray {
            origin: near,
            dir: (far - near).normalize(),
        }
    }
}

/// Луч `origin + dir · t`, `dir` единичный.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// Начало.
    pub origin: Vec3,
    /// Направление.
    pub dir: Vec3,
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
    let w = sw.min(sh * world_aspect);
    let h = sh.min(sw / world_aspect);
    Viewport {
        x: (sw - w) / 2.0,
        y: (sh - h) / 2.0,
        w,
        h,
    }
}

/// Вьюпорт мира `world` с целым масштабом и целыми отступами; окно меньше мира — `letterbox`.
pub fn letterbox_pixel(surface: (u32, u32), world: Vec2) -> Viewport {
    let (sw, sh) = (surface.0 as f32, surface.1 as f32);
    let k = (sw / world.x).min(sh / world.y).floor();
    if k < 1.0 {
        return letterbox(surface, world.x / world.y);
    }
    let (w, h) = (world.x * k, world.y * k);
    Viewport {
        x: ((sw - w) / 2.0).floor(),
        y: ((sh - h) / 2.0).floor(),
        w,
        h,
    }
}

/// Пиксель окна → точка мира через `camera`, показанную в `viewport`; вне вьюпорта — `None`.
pub fn screen_to_world(viewport: Viewport, camera: &Camera2D, pos: Vec2) -> Option<Vec2> {
    let t = (pos - Vec2::new(viewport.x, viewport.y)) / Vec2::new(viewport.w, viewport.h);
    if !(0.0..=1.0).contains(&t.x) || !(0.0..=1.0).contains(&t.y) {
        return None;
    }
    Some(camera.center + Vec2::new(t.x - 0.5, 0.5 - t.y) * camera.size)
}

/// Вьюпорт камеры: `letterbox_pixel` при `pixel_perfect`, иначе `letterbox`.
pub fn viewport(surface: (u32, u32), camera: &Camera2D, pixel_perfect: bool) -> Viewport {
    if pixel_perfect {
        letterbox_pixel(surface, camera.size)
    } else {
        letterbox(surface, camera.aspect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{vec2, vec3};
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

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

    const PIXEL_WORLD: Vec2 = vec2(320.0, 180.0);

    fn vp(x: f32, y: f32, w: f32, h: f32) -> Viewport {
        Viewport { x, y, w, h }
    }

    #[test]
    fn pixel_exact_multiple_fills_frame() {
        assert_eq!(
            letterbox_pixel((1280, 720), PIXEL_WORLD),
            vp(0.0, 0.0, 1280.0, 720.0)
        );
    }

    #[test]
    fn pixel_scale_is_floored_and_centered() {
        assert_eq!(
            letterbox_pixel((1366, 768), PIXEL_WORLD),
            vp(43.0, 24.0, 1280.0, 720.0)
        );
    }

    #[test]
    fn pixel_scale_limited_by_narrow_side() {
        assert_eq!(
            letterbox_pixel((2000, 400), PIXEL_WORLD),
            vp(680.0, 20.0, 640.0, 360.0)
        );
        assert_eq!(
            letterbox_pixel((700, 2000), PIXEL_WORLD),
            vp(30.0, 820.0, 640.0, 360.0)
        );
    }

    #[test]
    fn pixel_odd_remainder_gives_whole_offsets() {
        assert_eq!(
            letterbox_pixel((1285, 727), PIXEL_WORLD),
            vp(2.0, 3.0, 1280.0, 720.0)
        );
        assert_eq!(
            letterbox_pixel((321, 181), PIXEL_WORLD),
            vp(0.0, 0.0, 320.0, 180.0)
        );
    }

    #[test]
    fn pixel_window_smaller_than_world_falls_back() {
        let surface = (200, 150);
        assert_eq!(
            letterbox_pixel(surface, PIXEL_WORLD),
            letterbox(surface, 320.0 / 180.0)
        );
    }

    #[test]
    fn viewport_picks_mode() {
        let cam = Camera2D {
            center: Vec2::ZERO,
            size: PIXEL_WORLD,
        };
        let surface = (1366, 768);
        assert_eq!(
            viewport(surface, &cam, true),
            letterbox_pixel(surface, PIXEL_WORLD)
        );
        assert_eq!(
            viewport(surface, &cam, false),
            letterbox(surface, cam.aspect())
        );
        assert_ne!(
            viewport(surface, &cam, true),
            viewport(surface, &cam, false)
        );
    }

    fn cam_at(center: Vec2) -> Camera2D {
        Camera2D {
            center,
            size: PIXEL_WORLD,
        }
    }

    fn world(surface: (u32, u32), pixel: bool, pos: Vec2) -> Option<Vec2> {
        let cam = cam_at(vec2(100.0, 50.0));
        screen_to_world(viewport(surface, &cam, pixel), &cam, pos)
    }

    fn assert_near(got: Option<Vec2>, expected: Vec2) {
        let got = got.expect("inside viewport");
        assert!(got.abs_diff_eq(expected, 1e-3), "{got} != {expected}");
    }

    #[test]
    fn viewport_center_is_camera_center() {
        assert_near(
            world((1366, 768), true, vec2(683.0, 384.0)),
            vec2(100.0, 50.0),
        );
    }

    #[test]
    fn screen_down_is_world_down() {
        let above = world((1280, 720), false, vec2(640.0, 100.0)).unwrap();
        let below = world((1280, 720), false, vec2(640.0, 600.0)).unwrap();
        assert!(above.y > below.y, "{above} vs {below}");
    }

    #[test]
    fn viewport_corners_are_camera_edges() {
        // pixel: вьюпорт (43, 24, 1280, 720), масштаб 4.
        assert_near(
            world((1366, 768), true, vec2(43.0, 24.0)),
            vec2(-60.0, 140.0),
        );
        assert_near(
            world((1366, 768), true, vec2(1323.0, 744.0)),
            vec2(260.0, -40.0),
        );
        assert_near(
            world((1366, 768), true, vec2(47.0, 28.0)),
            vec2(-59.0, 139.0),
        );
    }

    #[test]
    fn letterbox_bars_are_outside_world() {
        // letterbox: вьюпорт (180, 0, 640, 360).
        for pos in [vec2(179.0, 100.0), vec2(821.0, 100.0)] {
            assert_eq!(world((1000, 360), false, pos), None, "{pos}");
        }
        assert_near(
            world((1000, 360), false, vec2(180.0, 0.0)),
            vec2(-60.0, 140.0),
        );
    }

    #[test]
    fn letterbox_pixel_margins_are_outside_world() {
        for pos in [
            vec2(42.0, 100.0),
            vec2(1324.0, 100.0),
            vec2(600.0, 23.0),
            vec2(600.0, 745.0),
        ] {
            assert_eq!(world((1366, 768), true, pos), None, "{pos}");
        }
    }

    fn eye() -> Camera3D {
        Camera3D {
            pos: vec3(1.0, 0.7, 2.0),
            yaw: 0.6,
            pitch: -0.3,
            ..Camera3D::default()
        }
    }

    fn ndc3(cam: &Camera3D, aspect: f32, p: Vec3) -> Vec3 {
        cam.view_proj(aspect).project_point3(p)
    }

    #[test]
    fn near_is_depth_zero_far_is_one() {
        let cam = eye();
        let near = ndc3(&cam, 1.5, cam.pos + cam.forward() * cam.near);
        let far = ndc3(&cam, 1.5, cam.pos + cam.forward() * cam.far);
        assert!(near.abs_diff_eq(Vec3::ZERO, 1e-4), "{near}");
        assert!(far.abs_diff_eq(Vec3::Z, 1e-4), "{far}");
    }

    #[test]
    fn yaw_zero_looks_down_negative_z() {
        let cam = Camera3D::default();
        assert!(cam.forward().abs_diff_eq(Vec3::NEG_Z, EPS));
        let p = ndc3(&cam, 1.0, vec3(0.0, 0.0, -5.0));
        assert!(p.truncate().abs_diff_eq(Vec2::ZERO, EPS), "{p}");
        assert!(p.z > 0.0 && p.z < 1.0, "{p}");
    }

    #[test]
    fn forward_follows_yaw_and_pitch() {
        let at = |yaw, pitch| {
            Camera3D {
                yaw,
                pitch,
                ..Camera3D::default()
            }
            .forward()
        };
        let h = FRAC_PI_4.sin();
        for (yaw, pitch, expected) in [
            (FRAC_PI_2, 0.0, Vec3::NEG_X),
            (-FRAC_PI_2, 0.0, Vec3::X),
            (0.0, FRAC_PI_4, vec3(0.0, h, -h)),
            (FRAC_PI_2, -FRAC_PI_4, vec3(-h, -h, 0.0)),
        ] {
            let got = at(yaw, pitch);
            assert!(got.abs_diff_eq(expected, EPS), "{yaw} {pitch} → {got}");
        }
    }

    #[test]
    fn center_ray_goes_along_forward_from_near_plane() {
        let cam = eye();
        let ray = cam.ray(Vec2::ZERO, 1.5);
        assert!(ray.dir.abs_diff_eq(cam.forward(), 1e-4), "{ray:?}");
        let near = cam.pos + cam.forward() * cam.near;
        assert!(ray.origin.abs_diff_eq(near, 1e-4), "{ray:?}");
    }

    #[test]
    fn corner_rays_follow_fov_and_aspect() {
        let cam = Camera3D::default();
        let aspect = 1.6;
        let t = (cam.fov_y / 2.0).tan();
        for corner in [vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, -1.0)] {
            let expected = vec3(corner.x * aspect * t, corner.y * t, -1.0).normalize();
            let got = cam.ray(corner, aspect).dir;
            assert!(got.abs_diff_eq(expected, 1e-4), "{corner} → {got}");
        }
    }

    #[test]
    fn ray_points_project_back_to_their_ndc() {
        let cam = eye();
        for ndc in [vec2(0.3, -0.7), vec2(-0.9, 0.5), vec2(0.0, 1.0)] {
            let ray = cam.ray(ndc, 1.5);
            let back = ndc3(&cam, 1.5, ray.origin + ray.dir * 7.0);
            assert!(back.truncate().abs_diff_eq(ndc, 1e-4), "{ndc} → {back}");
            assert!((ray.dir.length() - 1.0).abs() < EPS, "{ray:?}");
        }
    }

    #[test]
    fn pitch_is_clamped() {
        for (pitch, y) in [(10.0, MAX_PITCH.sin()), (-10.0, -MAX_PITCH.sin())] {
            let cam = Camera3D {
                pitch,
                ..Camera3D::default()
            };
            assert!((cam.forward().y - y).abs() < EPS, "{pitch}");
            assert!(
                cam.forward().z < 0.0,
                "взгляд перевалил через вертикаль: {pitch}"
            );
            assert!(cam.view_proj(1.0).is_finite(), "{pitch}");
        }
    }

    #[test]
    fn look_at_faces_target() {
        let mut cam = eye();
        for target in [
            vec3(5.0, -1.0, 3.0),
            vec3(-2.0, 4.0, -7.0),
            vec3(1.0, 0.7, 9.0),
            vec3(-3.0, 0.7, 2.0),
        ] {
            cam.look_at(vec3(1.0, 0.7, 2.0), target);
            let to = (target - cam.pos).normalize();
            assert!(
                cam.forward().abs_diff_eq(to, EPS),
                "{target}: {}",
                cam.forward()
            );
        }
    }

    #[test]
    fn look_at_straight_up_or_down_keeps_yaw_and_clamps_pitch() {
        for (dy, y) in [(3.0, MAX_PITCH.sin()), (-3.0, -MAX_PITCH.sin())] {
            let mut cam = eye();
            cam.look_at(Vec3::ONE, Vec3::ONE + vec3(0.0, dy, 0.0));
            assert_eq!(cam.yaw, eye().yaw, "{dy}");
            assert!((cam.forward().y - y).abs() < EPS, "{dy}");
            assert!(cam.view_proj(1.0).is_finite(), "{dy}");
        }
    }

    #[test]
    fn look_at_own_position_is_finite() {
        let mut cam = eye();
        cam.look_at(Vec3::ONE, Vec3::ONE);
        assert_eq!(cam.pos, Vec3::ONE);
        assert!(cam.view_proj(1.0).is_finite(), "{cam:?}");
    }

    #[test]
    fn x_is_right_y_is_up() {
        let cam = Camera3D::default();
        let right = ndc3(&cam, 1.0, vec3(1.0, 0.0, -5.0));
        let up = ndc3(&cam, 1.0, vec3(0.0, 1.0, -5.0));
        assert!(right.x > 0.0 && right.y.abs() < EPS, "{right}");
        assert!(up.y > 0.0 && up.x.abs() < EPS, "{up}");
    }

    #[test]
    fn wider_aspect_narrows_x() {
        let cam = Camera3D::default();
        let p = vec3(1.0, 1.0, -5.0);
        let square = ndc3(&cam, 1.0, p);
        let wide = ndc3(&cam, 2.0, p);
        assert!((wide.x - square.x / 2.0).abs() < EPS, "{square} {wide}");
        assert!((wide.y - square.y).abs() < EPS, "{square} {wide}");
    }

    fn iso() -> Camera3D {
        Camera3D {
            ortho: Some(4.0),
            ..eye()
        }
    }

    /// Точка в `up` метрах над осью взгляда и `along` метрах вдоль неё.
    fn off_axis(cam: &Camera3D, along: f32, up: f32) -> Vec3 {
        let right = cam.forward().cross(Vec3::Y).normalize();
        let up_dir = right.cross(cam.forward());
        cam.pos + cam.forward() * along + up_dir * up
    }

    #[test]
    fn ortho_edge_stays_at_ndc_edge_at_any_distance() {
        let cam = iso();
        for along in [1.0, 7.0, 60.0] {
            let p = ndc3(&cam, 1.5, off_axis(&cam, along, 2.0));
            assert!((p.y - 1.0).abs() < 1e-4, "{along}: {p}");
            assert!(p.x.abs() < 1e-4, "{along}: {p}");
        }
        let right = cam.forward().cross(Vec3::Y).normalize();
        let p = ndc3(&cam, 1.5, cam.pos + cam.forward() * 5.0 + right * 3.0);
        assert!((p.x - 1.0).abs() < 1e-4, "width = height · aspect: {p}");
    }

    #[test]
    fn perspective_edge_shrinks_with_distance() {
        let cam = eye();
        let [near, far] = [2.0, 4.0].map(|along| ndc3(&cam, 1.5, off_axis(&cam, along, 2.0)).y);
        assert!((near - 2.0 * far).abs() < 1e-4, "{near} {far}");
    }

    #[test]
    fn ortho_near_is_depth_zero_far_is_one() {
        let cam = iso();
        let near = ndc3(&cam, 1.5, cam.pos + cam.forward() * cam.near);
        let far = ndc3(&cam, 1.5, cam.pos + cam.forward() * cam.far);
        assert!(near.abs_diff_eq(Vec3::ZERO, 1e-4), "{near}");
        assert!(far.abs_diff_eq(Vec3::Z, 1e-4), "{far}");
    }

    #[test]
    fn ortho_rays_are_parallel_from_spread_origins() {
        let cam = iso();
        let (a, b) = (cam.ray(vec2(-0.5, 0.2), 1.5), cam.ray(vec2(0.5, -0.8), 1.5));
        assert!(a.dir.abs_diff_eq(cam.forward(), 1e-4), "{a:?}");
        assert!(b.dir.abs_diff_eq(cam.forward(), 1e-4), "{b:?}");
        let gap = b.origin - a.origin;
        assert!(gap.dot(cam.forward()).abs() < 1e-4, "{gap}");
        // ΔNDC (1, −1) при высоте 4 и aspect 1,5 — (3, −2) в мире.
        assert!(
            (gap.length() - vec2(3.0, 2.0).length()).abs() < 1e-3,
            "{gap}"
        );
    }

    #[test]
    fn eye_position_moves_view() {
        let cam = eye();
        let p = ndc3(&cam, 1.5, cam.pos + cam.forward() * 3.0);
        assert!(p.truncate().abs_diff_eq(Vec2::ZERO, 1e-4), "{p}");
    }
}
