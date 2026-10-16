use glam::{Mat3, Quat, Vec3};

/// Твёрдое тело: центр масс, ориентация, скорости в мире и накопленные за шаг сила и момент.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidBody {
    /// Центр масс в мире.
    pub pos: Vec3,
    /// Тело → мир.
    pub rot: Quat,
    /// Скорость центра масс.
    pub vel: Vec3,
    /// Угловая скорость в мире, рад/с.
    pub ang_vel: Vec3,
    /// `1 / масса`.
    pub inv_mass: f32,
    /// Диагональ обратного тензора инерции в осях тела; компоненты положительные.
    pub inv_inertia: Vec3,
    /// Угловое демпфирование λ: за шаг `ω *= exp(−λ·dt)`.
    pub angular_damping: f32,
    force: Vec3,
    torque: Vec3,
}

impl RigidBody {
    /// Покоящееся тело в `pos` с массой `mass` и главными моментами `inertia`.
    pub fn new(pos: Vec3, mass: f32, inertia: Vec3) -> Self {
        Self {
            pos,
            rot: Quat::IDENTITY,
            vel: Vec3::ZERO,
            ang_vel: Vec3::ZERO,
            inv_mass: mass.recip(),
            inv_inertia: inertia.recip(),
            angular_damping: 0.0,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
        }
    }

    /// Обратный тензор инерции в мире `R·I⁻¹·Rᵀ`.
    pub fn inv_inertia_world(&self) -> Mat3 {
        let r = Mat3::from_quat(self.rot);
        r * Mat3::from_diagonal(self.inv_inertia) * r.transpose()
    }

    /// Скорость точки тела `p` (мир).
    pub fn velocity_at(&self, p: Vec3) -> Vec3 {
        self.vel + self.ang_vel.cross(p - self.pos)
    }

    /// Сила в точке `p` (мир) до следующего `integrate_velocity`.
    pub fn apply_force_at(&mut self, force: Vec3, p: Vec3) {
        self.force += force;
        self.torque += (p - self.pos).cross(force);
    }

    /// Мгновенный импульс в точке `p` (мир).
    pub fn apply_impulse_at(&mut self, impulse: Vec3, p: Vec3) {
        self.vel += impulse * self.inv_mass;
        self.ang_vel += self.inv_inertia_world() * (p - self.pos).cross(impulse);
    }

    /// Скорости за `dt`: тяжесть, накопленные сила и момент, гироскопический член, демпфирование; накопленное обнуляется.
    pub fn integrate_velocity(&mut self, dt: f32, gravity: Vec3) {
        self.vel += (gravity + self.force * self.inv_mass) * dt;
        let w = gyroscopic_step(
            self.rot.inverse() * self.ang_vel,
            self.inv_inertia.recip(),
            dt,
        );
        self.ang_vel = self.rot * w + self.inv_inertia_world() * self.torque * dt;
        self.ang_vel *= (-self.angular_damping * dt).exp();
        self.force = Vec3::ZERO;
        self.torque = Vec3::ZERO;
    }

    /// Позиция и ориентация за `dt` по текущим скоростям.
    pub fn integrate_position(&mut self, dt: f32) {
        self.pos += self.vel * dt;
        let spin = Quat::from_xyzw(self.ang_vel.x, self.ang_vel.y, self.ang_vel.z, 0.0) * self.rot;
        self.rot = (self.rot + spin * (0.5 * dt)).normalize();
    }
}

/// Неявный шаг `I·ω̇ = −ω×(I·ω)` в осях тела одной итерацией Ньютона: явный накачивает энергию.
fn gyroscopic_step(w: Vec3, inertia: Vec3, dt: f32) -> Vec3 {
    let iw = inertia * w;
    let i = Mat3::from_diagonal(inertia);
    let jacobian = i + dt * (skew(w) * i - skew(iw));
    w - jacobian.inverse() * (dt * w.cross(iw))
}

/// Матрица `a × ·`.
fn skew(a: Vec3) -> Mat3 {
    Mat3::from_cols(
        Vec3::new(0.0, a.z, -a.y),
        Vec3::new(-a.z, 0.0, a.x),
        Vec3::new(a.y, -a.x, 0.0),
    )
}

/// Главные моменты инерции сплошного бруска массы `mass` с полными размерами `size`.
pub fn box_inertia(mass: f32, size: Vec3) -> Vec3 {
    let s = size * size;
    mass / 12.0 * Vec3::new(s.y + s.z, s.x + s.z, s.x + s.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    const DT: f32 = 1.0 / 240.0;
    const G: Vec3 = Vec3::new(0.0, -9.81, 0.0);

    fn body() -> RigidBody {
        RigidBody::new(Vec3::new(1.0, 2.0, 3.0), 2.0, Vec3::new(1.0, 2.0, 3.0))
    }

    fn step(b: &mut RigidBody, gravity: Vec3) {
        b.integrate_velocity(DT, gravity);
        b.integrate_position(DT);
    }

    fn close(a: Vec3, b: Vec3, eps: f32) -> bool {
        (a - b).length() <= eps
    }

    fn momentum(b: &RigidBody) -> Vec3 {
        b.inv_inertia_world().inverse() * b.ang_vel
    }

    fn energy(b: &RigidBody) -> f32 {
        0.5 * b.ang_vel.dot(momentum(b))
    }

    #[test]
    fn new_inverts_mass_and_inertia() {
        let b = body();
        assert_eq!(b.inv_mass, 0.5);
        assert_eq!(b.inv_inertia, Vec3::new(1.0, 0.5, 1.0 / 3.0));
        assert_eq!(b.rot, Quat::IDENTITY);
    }

    #[test]
    fn free_fall_matches_analytic_within_step() {
        let mut b = body();
        b.vel = Vec3::new(2.0, 5.0, 0.0);
        let start = b.pos;
        let n = 480;
        for _ in 0..n {
            step(&mut b, G);
        }
        let t = n as f32 * DT;
        let exact = start + Vec3::new(2.0, 5.0, 0.0) * t + 0.5 * G * t * t;
        assert!(
            close(b.vel, Vec3::new(2.0, 5.0, 0.0) + G * t, 1e-3),
            "{b:?}"
        );
        // Полунеявный Эйлер обгоняет аналитику ровно на g·dt·t/2.
        let lead = 0.5 * G * DT * t;
        assert!(
            close(b.pos, exact + lead, 1e-3),
            "{:?} vs {:?}",
            b.pos,
            exact
        );
        assert_eq!(b.ang_vel, Vec3::ZERO);
    }

    #[test]
    fn force_at_center_of_mass_does_not_spin() {
        let mut b = body();
        b.apply_force_at(Vec3::new(4.0, 0.0, 0.0), b.pos);
        b.integrate_velocity(DT, Vec3::ZERO);
        assert!(close(b.vel, Vec3::new(4.0 * 0.5 * DT, 0.0, 0.0), 1e-7));
        assert_eq!(b.ang_vel, Vec3::ZERO);
    }

    #[test]
    fn force_on_lever_gives_r_cross_f_torque() {
        let mut b = body();
        b.rot = Quat::from_rotation_y(0.7);
        let r = Vec3::new(0.0, 0.5, 1.0);
        let f = Vec3::new(3.0, 0.0, -1.0);
        b.apply_force_at(f, b.pos + r);
        let expected = b.inv_inertia_world() * r.cross(f) * DT;
        b.integrate_velocity(DT, Vec3::ZERO);
        assert!(
            close(b.ang_vel, expected, 1e-6),
            "{:?} vs {expected:?}",
            b.ang_vel
        );
        assert!(close(b.vel, f * 0.5 * DT, 1e-7));
    }

    #[test]
    fn accumulated_force_is_spent_by_one_step() {
        let mut b = body();
        b.apply_force_at(Vec3::X, b.pos + Vec3::Y);
        b.apply_force_at(Vec3::X, b.pos + Vec3::Y);
        b.integrate_velocity(DT, Vec3::ZERO);
        let (vel, ang_vel) = (b.vel, b.ang_vel);
        assert!(close(vel, Vec3::X * DT, 1e-7));
        b.integrate_velocity(DT, Vec3::ZERO);
        assert!(close(b.vel, vel, 1e-7) && close(b.ang_vel, ang_vel, 1e-6));
    }

    #[test]
    fn inertia_world_swaps_axes_after_quarter_turn() {
        let mut b = body();
        b.rot = Quat::from_rotation_z(FRAC_PI_2);
        let m = b.inv_inertia_world();
        let diag = Vec3::new(m.x_axis.x, m.y_axis.y, m.z_axis.z);
        assert!(close(diag, Vec3::new(0.5, 1.0, 1.0 / 3.0), 1e-6), "{m:?}");
        assert!(m.x_axis.y.abs() < 1e-6 && m.y_axis.x.abs() < 1e-6);
    }

    #[test]
    fn impulse_changes_velocity_and_spin() {
        let mut b = body();
        b.rot = Quat::from_rotation_x(0.4);
        let r = Vec3::new(1.0, 0.0, 0.5);
        let j = Vec3::new(0.0, 2.0, 0.0);
        let spin = b.inv_inertia_world() * r.cross(j);
        b.apply_impulse_at(j, b.pos + r);
        assert!(close(b.vel, Vec3::new(0.0, 1.0, 0.0), 1e-7));
        assert!(close(b.ang_vel, spin, 1e-7));
    }

    #[test]
    fn velocity_at_is_v_plus_omega_cross_r() {
        let mut b = body();
        b.vel = Vec3::new(1.0, 0.0, 0.0);
        b.ang_vel = Vec3::new(0.0, 2.0, 0.0);
        let p = b.pos + Vec3::new(0.0, 0.0, 1.0);
        assert!(close(b.velocity_at(p), Vec3::new(3.0, 0.0, 0.0), 1e-7));
    }

    #[test]
    fn position_follows_velocity_and_rotation_follows_spin() {
        let mut b = body();
        let start = b.pos;
        b.vel = Vec3::new(0.0, 0.0, 2.0);
        b.ang_vel = Vec3::new(0.0, 1.0, 0.0);
        for _ in 0..240 {
            b.integrate_position(DT);
        }
        assert!(close(b.pos, start + Vec3::new(0.0, 0.0, 2.0), 1e-4));
        let angle = b.rot.angle_between(Quat::from_rotation_y(1.0));
        assert!(angle < 1e-3, "{:?}", b.rot);
    }

    #[test]
    fn quaternion_stays_unit_after_many_steps() {
        let mut b = body();
        b.ang_vel = Vec3::new(3.0, -7.0, 5.0);
        for _ in 0..10_000 {
            b.integrate_position(DT);
        }
        assert!((b.rot.length() - 1.0).abs() < 1e-5, "{}", b.rot.length());
    }

    #[test]
    fn spin_about_principal_axis_keeps_omega() {
        // Промежуточная ось Y неустойчива: ошибка округления опрокидывает её.
        for axis in [Vec3::X, Vec3::Z] {
            let mut b = body();
            b.rot = Quat::from_rotation_x(0.3);
            b.ang_vel = b.rot * axis * 4.0;
            let w = b.ang_vel;
            for _ in 0..2400 {
                step(&mut b, Vec3::ZERO);
            }
            assert!(
                close(b.ang_vel, w, 1e-3),
                "{axis}: {:?} vs {w:?}",
                b.ang_vel
            );
        }
    }

    #[test]
    fn spin_about_intermediate_axis_tumbles() {
        let mut b = body();
        b.ang_vel = Vec3::new(0.01, 3.0, 0.01);
        let mut min_y = f32::MAX;
        for _ in 0..2400 {
            step(&mut b, Vec3::ZERO);
            min_y = min_y.min((b.rot.inverse() * b.ang_vel).y);
        }
        assert!(min_y < 0.0, "{min_y}");
    }

    #[test]
    fn free_rotation_does_not_pump_momentum_or_energy() {
        for w in [Vec3::new(0.01, 3.0, 0.01), Vec3::new(2.0, -1.0, 4.0)] {
            let mut b = body();
            b.rot = Quat::from_rotation_z(0.5);
            b.ang_vel = w;
            let (l, e) = (momentum(&b).length(), energy(&b));
            for _ in 0..2400 {
                step(&mut b, Vec3::ZERO);
            }
            let (l1, e1) = (momentum(&b).length(), energy(&b));
            // Неявный шаг гасит: за 10 с |L| теряет до 3 %, энергия — до 7 %.
            assert!(l1 < l * 1.01 && l1 > l * 0.95, "{w}: |L| {l} → {l1}");
            assert!(e1 < e * 1.01 && e1 > e * 0.9, "{w}: E {e} → {e1}");
        }
    }

    #[test]
    fn gyroscopic_step_solves_implicit_equation() {
        let inertia = Vec3::new(1.0, 2.0, 3.0);
        let w0 = Vec3::new(2.0, -1.0, 4.0);
        let dt = 0.05;
        let w1 = gyroscopic_step(w0, inertia, dt);
        let residual = inertia * (w1 - w0) + dt * w1.cross(inertia * w1);
        // Верный якобиан — квадратичная сходимость Ньютона: невязка 0,0025; неверный — 0,1.
        assert!(residual.length() < 0.01, "{residual}");
    }

    #[test]
    fn angular_damping_decays_spin_exponentially() {
        let mut b = body();
        b.angular_damping = 2.0;
        b.ang_vel = Vec3::new(0.0, 4.0, 0.0);
        for _ in 0..240 {
            b.integrate_velocity(DT, Vec3::ZERO);
        }
        assert!(close(
            b.ang_vel,
            Vec3::new(0.0, 4.0 * (-2.0f32).exp(), 0.0),
            1e-4
        ));
    }

    #[test]
    fn cube_inertia_is_mass_side_squared_over_six() {
        assert_eq!(box_inertia(6.0, Vec3::splat(2.0)), Vec3::splat(4.0));
        let slab = box_inertia(12.0, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(slab, Vec3::new(13.0, 10.0, 5.0));
    }
}
