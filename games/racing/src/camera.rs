use engine::{Camera3D, Quat, Vec3};

/// За машиной по горизонтали, м.
const BEHIND: f32 = 6.0;
/// Над центром масс, м.
const ABOVE: f32 = 2.5;
/// Точка взгляда — над капотом: вперёд и вверх от центра масс, м.
const LOOK_AHEAD: f32 = 1.5;
const LOOK_UP: f32 = 1.2;
/// Собственная частота пружины, рад/с; затухание критическое.
const OMEGA: f32 = 6.0;
const FAR: f32 = 400.0;

/// Нос машины в горизонтальной плоскости; нос строго вверх или вниз — +Z.
fn heading(rot: Quat) -> Vec3 {
    let f = rot * Vec3::Z;
    Vec3::new(f.x, 0.0, f.z).try_normalize().unwrap_or(Vec3::Z)
}

/// Куда тянется глаз камеры за машиной с центром масс `pos`.
pub(crate) fn target(pos: Vec3, rot: Quat) -> Vec3 {
    pos - heading(rot) * BEHIND + Vec3::Y * ABOVE
}

/// Камера на пружине за машиной; глаз шагает тиком, `camera` интерполирует его.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Chase {
    pub(crate) eye: Vec3,
    prev: Vec3,
    vel: Vec3,
}

impl Chase {
    /// Сразу на месте, без разгона.
    pub(crate) fn new(pos: Vec3, rot: Quat) -> Self {
        let eye = target(pos, rot);
        Self {
            eye,
            prev: eye,
            vel: Vec3::ZERO,
        }
    }

    pub(crate) fn tick(&mut self, pos: Vec3, rot: Quat, dt: f32) {
        self.prev = self.eye;
        let x = self.eye - target(pos, rot);
        self.vel += (-OMEGA * OMEGA * x - 2.0 * OMEGA * self.vel) * dt;
        self.eye += self.vel * dt;
    }

    /// Глаз между тиками, взгляд — над капотом машины в позе кадра.
    pub(crate) fn camera(&self, alpha: f32, pos: Vec3, rot: Quat) -> Camera3D {
        let mut camera = Camera3D {
            far: FAR,
            ..Camera3D::default()
        };
        let look = pos + heading(rot) * LOOK_AHEAD + Vec3::Y * LOOK_UP;
        camera.look_at(self.prev.lerp(self.eye, alpha), look);
        camera
    }
}

#[cfg(test)]
mod tests {
    use engine::vec3;

    use super::*;

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn starts_behind_and_above_looking_over_hood() {
        let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let pos = vec3(10.0, 1.0, 5.0);
        let chase = Chase::new(pos, rot);
        assert!(
            chase
                .eye
                .abs_diff_eq(vec3(10.0 - BEHIND, 1.0 + ABOVE, 5.0), 1e-5)
        );
        let camera = chase.camera(0.0, pos, rot);
        let look = vec3(10.0 + LOOK_AHEAD, 1.0 + LOOK_UP, 5.0);
        assert!(
            camera
                .forward()
                .abs_diff_eq((look - chase.eye).normalize(), 1e-5)
        );
        assert_eq!(camera.far, FAR);
    }

    #[test]
    fn pitched_nose_keeps_camera_level_behind() {
        let up = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        assert!((up * Vec3::Z).abs_diff_eq(Vec3::Y, 1e-6));
        assert_eq!(target(Vec3::ZERO, up), vec3(0.0, ABOVE, -BEHIND));
        let tilted = Quat::from_rotation_y(1.0) * Quat::from_rotation_x(-0.5);
        let t = target(Vec3::ZERO, tilted);
        assert!((t.y - ABOVE).abs() < 1e-6);
        assert!((t.x.hypot(t.z) - BEHIND).abs() < 1e-5, "{t}");
    }

    #[test]
    fn spring_catches_up_with_moving_car_without_overshoot() {
        let rot = Quat::IDENTITY;
        let mut chase = Chase::new(Vec3::ZERO, rot);
        let pos = vec3(0.0, 0.0, 20.0);
        let goal = target(pos, rot);
        let mut last = chase.eye.distance(goal);
        for tick in 0..2 * 60 {
            chase.tick(pos, rot, DT);
            let d = chase.eye.distance(goal);
            assert!(d <= last, "{tick}: {d} > {last}");
            assert!(
                chase.eye.z <= goal.z + 1e-4,
                "{tick}: перелёт {}",
                chase.eye
            );
            last = d;
        }
        assert!(last < 0.05, "{last}");
        // Шаг 20 м — пружина отстаёт, а не прыгает.
        let mut chase = Chase::new(Vec3::ZERO, rot);
        chase.tick(pos, rot, DT);
        assert!(
            chase.eye.z - target(Vec3::ZERO, rot).z < 1.0,
            "{}",
            chase.eye
        );
    }

    #[test]
    fn camera_interpolates_eye_between_ticks() {
        let rot = Quat::IDENTITY;
        let mut chase = Chase::new(Vec3::ZERO, rot);
        let before = chase.eye;
        chase.tick(vec3(0.0, 0.0, 20.0), rot, DT);
        let pos = Vec3::ZERO;
        assert_eq!(chase.camera(0.0, pos, rot).pos, before);
        assert_eq!(chase.camera(1.0, pos, rot).pos, chase.eye);
        let mid = chase.camera(0.5, pos, rot).pos;
        assert!(mid.abs_diff_eq(before.lerp(chase.eye, 0.5), 1e-6));
    }
}
