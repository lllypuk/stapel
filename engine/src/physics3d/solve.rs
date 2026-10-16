use std::cmp::Ordering;

use glam::{Mat3, Vec2, Vec3};

use super::{Contact, RigidBody};

/// Контакты ближе этого и с почти той же нормалью — один контакт (шов двух треугольников).
const MERGE_DIST: f32 = 0.01;
const MERGE_COS: f32 = 0.99;

/// Параметры `solve_contacts`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactParams {
    /// Коэффициент Кулона μ.
    pub friction: f32,
    /// Доля скорости сближения, возвращаемая отскоком.
    pub restitution: f32,
    /// Скорость сближения, м/с, ниже которой отскока нет.
    pub restitution_threshold: f32,
    /// Глубина, м, которую выталкивание оставляет.
    pub slop: f32,
    /// Потолок сдвига `pos` за вызов, м.
    pub max_correction: f32,
    /// Проходы последовательных импульсов и выталкивания.
    pub iterations: u32,
}

#[derive(Clone, Copy, Debug)]
struct Row {
    point: Vec3,
    normal: Vec3,
    tangents: [Vec3; 2],
    depth: f32,
    normal_mass: f32,
    tangent_mass: [f32; 2],
    target: f32,
    jn: f32,
    jt: Vec2,
}

/// Гасит сближение тела с миром по контактам импульсами и выталкивает `pos` из глубины.
/// Выталкивание скорость не меняет; зовётся между `integrate_velocity` и `integrate_position`.
pub fn solve_contacts(body: &mut RigidBody, contacts: &[Contact], params: &ContactParams) {
    let inv_inertia = body.inv_inertia_world();
    let mut rows: Vec<Row> = reduce(contacts)
        .into_iter()
        .map(|c| {
            let r = c.point - body.pos;
            let (t0, t1) = c.normal.any_orthonormal_pair();
            let approach = body.velocity_at(c.point).dot(c.normal);
            let target = if approach < -params.restitution_threshold {
                -params.restitution * approach
            } else {
                0.0
            };
            Row {
                point: c.point,
                normal: c.normal,
                tangents: [t0, t1],
                depth: c.depth,
                normal_mass: effective_mass(body, &inv_inertia, r, c.normal),
                tangent_mass: [
                    effective_mass(body, &inv_inertia, r, t0),
                    effective_mass(body, &inv_inertia, r, t1),
                ],
                target,
                jn: 0.0,
                jt: Vec2::ZERO,
            }
        })
        .collect();

    for _ in 0..params.iterations {
        for row in &mut rows {
            let v = body.velocity_at(row.point);
            let dt = -Vec2::new(v.dot(row.tangents[0]), v.dot(row.tangents[1]))
                * Vec2::from(row.tangent_mass);
            let old = row.jt;
            row.jt = (old + dt).clamp_length_max(params.friction * row.jn);
            let d = row.jt - old;
            let impulse = row.tangents[0] * d.x + row.tangents[1] * d.y;
            body.apply_impulse_at(impulse, row.point);

            let vn = body.velocity_at(row.point).dot(row.normal);
            let old = row.jn;
            row.jn = (old + (row.target - vn) * row.normal_mass).max(0.0);
            body.apply_impulse_at(row.normal * (row.jn - old), row.point);
        }
    }

    let mut shift = Vec3::ZERO;
    for _ in 0..params.iterations {
        for row in &rows {
            let left = row.depth - params.slop - shift.dot(row.normal);
            if left > 0.0 {
                shift += row.normal * left;
            }
        }
    }
    body.pos += shift.clamp_length_max(params.max_correction);
}

/// `1 / (m⁻¹ + (r×d)·I⁻¹(r×d))` — масса, которую импульс вдоль `d` в точке `r` видит с вращением.
fn effective_mass(body: &RigidBody, inv_inertia: &Mat3, r: Vec3, d: Vec3) -> f32 {
    let rd = r.cross(d);
    (body.inv_mass + rd.dot(*inv_inertia * rd)).recip()
}

/// Из близких контактов с почти той же нормалью остаётся самый глубокий, при равной глубине — первый.
/// Остальные — в порядке точек: импульс между избыточными опорами делится неоднозначно, и порядок входа менял бы рыскание.
fn reduce(contacts: &[Contact]) -> Vec<Contact> {
    let near = |a: &Contact, b: &Contact| {
        a.point.distance(b.point) < MERGE_DIST && a.normal.dot(b.normal) > MERGE_COS
    };
    let mut out = contacts
        .iter()
        .enumerate()
        .filter(|&(i, c)| {
            !contacts.iter().enumerate().any(|(j, o)| {
                j != i && near(c, o) && o.depth.total_cmp(&c.depth).then(i.cmp(&j)).is_gt()
            })
        })
        .map(|(_, c)| *c)
        .collect::<Vec<_>>();
    out.sort_by(|a, b| {
        let key = |c: &Contact| {
            [
                c.point.x, c.point.y, c.point.z, c.normal.x, c.normal.y, c.normal.z,
            ]
        };
        key(a)
            .iter()
            .zip(key(b))
            .map(|(x, y)| x.total_cmp(&y))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics3d::{StaticWorld, Triangle, box_inertia};
    use glam::Quat;

    const DT: f32 = 1.0 / 240.0;
    const G: Vec3 = Vec3::new(0.0, -9.81, 0.0);
    const R: f32 = 0.3;

    const PARAMS: ContactParams = ContactParams {
        friction: 0.8,
        restitution: 0.0,
        restitution_threshold: 1.0,
        slop: 0.005,
        max_correction: 0.2,
        iterations: 8,
    };

    /// Прямоугольник `c ± u ± v` двумя треугольниками с нормалью вдоль `u × v`.
    fn rect(c: Vec3, u: Vec3, v: Vec3) -> [Triangle; 2] {
        [
            Triangle {
                verts: [c - u - v, c + u - v, c - u + v],
                tag: 0,
            },
            Triangle {
                verts: [c + u - v, c + u + v, c - u + v],
                tag: 0,
            },
        ]
    }

    /// Плоскость через начало координат с нормалью `q·Y`.
    fn plane(q: Quat) -> StaticWorld {
        StaticWorld::new(
            4.0,
            rect(Vec3::ZERO, q * Vec3::X * 50.0, q * Vec3::NEG_Z * 50.0),
        )
    }

    /// Сферы кузова в осях тела, нижняя точка — на 0,5 под центром масс.
    const CORNERS: [Vec3; 4] = [
        Vec3::new(-0.7, -0.2, -1.6),
        Vec3::new(0.7, -0.2, -1.6),
        Vec3::new(-0.7, -0.2, 1.6),
        Vec3::new(0.7, -0.2, 1.6),
    ];

    fn car(pos: Vec3, rot: Quat) -> RigidBody {
        let mut b = RigidBody::new(pos, 1000.0, box_inertia(1000.0, Vec3::new(1.8, 1.0, 4.0)));
        b.rot = rot;
        b
    }

    fn ball(pos: Vec3) -> RigidBody {
        RigidBody::new(pos, 1.0, Vec3::splat(0.4 * R * R))
    }

    fn contacts(world: &StaticWorld, b: &RigidBody, spheres: &[Vec3]) -> Vec<Contact> {
        let mut out = Vec::new();
        for s in spheres {
            world.sphere_contacts(b.pos + b.rot * *s, R, &mut out);
        }
        out
    }

    fn step(world: &StaticWorld, b: &mut RigidBody, spheres: &[Vec3], params: &ContactParams) {
        b.integrate_velocity(DT, G);
        let cs = contacts(world, b, spheres);
        solve_contacts(b, &cs, params);
        b.integrate_position(DT);
    }

    fn energy(b: &RigidBody) -> f32 {
        let m = b.inv_mass.recip();
        let l = b.inv_inertia_world().inverse() * b.ang_vel;
        0.5 * m * b.vel.length_squared() + 0.5 * b.ang_vel.dot(l) - m * G.y * b.pos.y
    }

    fn contact(point: Vec3, normal: Vec3, depth: f32) -> Contact {
        Contact {
            point,
            normal,
            depth,
            tri: 0,
        }
    }

    #[test]
    fn box_on_plane_rests_without_drift_or_energy_gain() {
        let world = plane(Quat::IDENTITY);
        let mut b = car(Vec3::new(0.0, 0.5, 0.0), Quat::IDENTITY);
        let e0 = energy(&b);
        for _ in 0..2400 {
            step(&world, &mut b, &CORNERS, &PARAMS);
            assert!(energy(&b) <= e0 + 1e-3, "{b:?}");
        }
        assert!(Vec2::new(b.pos.x, b.pos.z).length() < 1e-4, "{b:?}");
        assert!(b.pos.y < 0.5 && b.pos.y > 0.5 - PARAMS.slop - 1e-3, "{b:?}");
        assert!(b.vel.length() < 1e-2, "{b:?}");
        assert!(b.ang_vel.length() < 1e-2, "{b:?}");
        assert!(b.rot.angle_between(Quat::IDENTITY) < 1e-3, "{b:?}");
    }

    fn on_incline(friction: f32) -> (RigidBody, RigidBody) {
        let q = Quat::from_rotation_z(20f32.to_radians());
        let world = plane(q);
        let start = car(q * Vec3::new(0.0, 0.5, 0.0), q);
        let mut b = start;
        let params = ContactParams { friction, ..PARAMS };
        for _ in 0..480 {
            step(&world, &mut b, &CORNERS, &params);
        }
        (start, b)
    }

    #[test]
    fn incline_holds_with_high_friction() {
        let (start, b) = on_incline(0.8);
        assert!(b.pos.distance(start.pos) < 0.01, "{b:?}");
        assert!(b.vel.length() < 1e-2, "{b:?}");
    }

    #[test]
    fn incline_slides_with_low_friction() {
        let (_, b) = on_incline(0.1);
        let a = 9.81 * (20f32.to_radians().sin() - 0.1 * 20f32.to_radians().cos());
        let speed = b.vel.length();
        assert!(
            (speed - a * 2.0).abs() < 0.1 * a * 2.0,
            "{speed} vs {}",
            a * 2.0
        );
        assert!(b.vel.x < 0.0 && b.vel.y < 0.0, "{b:?}");
    }

    #[test]
    fn friction_is_a_circle_not_a_square() {
        let world = plane(Quat::IDENTITY);
        let mut b = ball(Vec3::new(0.0, R - 0.001, 0.0));
        b.vel = Vec3::new(3.0, 0.0, 3.0);
        let params = ContactParams {
            friction: 0.5,
            ..PARAMS
        };
        b.integrate_velocity(DT, G);
        let cs = contacts(&world, &b, &[Vec3::ZERO]);
        solve_contacts(&mut b, &cs, &params);
        let p = b.pos + Vec3::NEG_Y * R;
        let slip = b.velocity_at(p);
        let lost = Vec3::new(3.0, 0.0, 3.0).length() - slip.length();
        // Шар закручивается трением, точка контакта тормозит быстрее центра: m_eff = 2/7 m.
        let expected = 0.5 * 9.81 * DT * 3.5;
        assert!((lost - expected).abs() < 1e-3, "{lost} vs {expected}");
        assert!((slip.x - slip.z).abs() < 1e-5, "{slip:?}");
    }

    fn drop_ball(height: f32, params: &ContactParams) -> (f32, f32) {
        let world = plane(Quat::IDENTITY);
        let mut b = ball(Vec3::new(0.0, R + height, 0.0));
        let mut impact = 0.0f32;
        let mut rebound = f32::NEG_INFINITY;
        for _ in 0..480 {
            let before = b.vel.y;
            step(&world, &mut b, &[Vec3::ZERO], params);
            if b.vel.y > before + 1e-6 && impact == 0.0 {
                impact = before;
            }
            if impact != 0.0 {
                rebound = rebound.max(b.vel.y);
            }
        }
        (impact, rebound)
    }

    #[test]
    fn fall_without_restitution_does_not_bounce() {
        let (impact, rebound) = drop_ball(1.0, &PARAMS);
        assert!(impact < -4.0, "{impact}");
        assert!(rebound <= 1e-5, "{rebound}");
    }

    #[test]
    fn restitution_only_above_threshold() {
        let params = ContactParams {
            restitution: 0.5,
            ..PARAMS
        };
        let (impact, rebound) = drop_ball(1.0, &params);
        assert!((rebound + 0.5 * impact).abs() < 0.1, "{impact} {rebound}");
        let (impact, rebound) = drop_ball(0.0127, &params);
        assert!(impact > -0.6 && impact < -0.4, "{impact}");
        assert!(rebound <= 1e-5, "{rebound}");
    }

    #[test]
    fn approach_exactly_at_threshold_does_not_bounce() {
        let mut b = ball(Vec3::new(0.0, R, 0.0));
        b.vel = Vec3::new(0.0, -1.0, 0.0);
        let params = ContactParams {
            restitution: 0.5,
            ..PARAMS
        };
        let c = contact(Vec3::ZERO, Vec3::Y, 0.0);
        solve_contacts(&mut b, &[c], &params);
        assert!(b.vel.y.abs() < 1e-6, "{b:?}");
        b.vel = Vec3::new(0.0, -1.01, 0.0);
        solve_contacts(&mut b, &[c], &params);
        assert!((b.vel.y - 0.505).abs() < 1e-5, "{b:?}");
    }

    #[test]
    fn push_out_moves_pos_but_not_velocity() {
        let world = plane(Quat::IDENTITY);
        let mut b = ball(Vec3::new(0.0, R - 0.1, 0.0));
        b.vel = Vec3::new(0.0, -0.5, 0.0);
        let cs = contacts(&world, &b, &[Vec3::ZERO]);
        solve_contacts(&mut b, &cs, &PARAMS);
        assert!((b.pos.y - (R - PARAMS.slop)).abs() < 1e-6, "{b:?}");
        assert!(b.vel.y <= 0.0 && b.vel.y > -1e-6, "{b:?}");
        assert_eq!(b.ang_vel, Vec3::ZERO);
    }

    #[test]
    fn push_out_leaves_slop_and_is_capped() {
        let up = |depth: f32, max_correction: f32| {
            let mut b = ball(Vec3::ZERO);
            let params = ContactParams {
                max_correction,
                ..PARAMS
            };
            solve_contacts(&mut b, &[contact(Vec3::NEG_Y, Vec3::Y, depth)], &params);
            b.pos.y
        };
        assert_eq!(up(0.004, 1.0), 0.0);
        assert!((up(0.5, 1.0) - 0.495).abs() < 1e-6);
        assert!((up(0.5, 0.05) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn push_out_of_floor_and_wall_sums_orthogonal_depths() {
        let mut b = ball(Vec3::ZERO);
        let cs = [
            contact(Vec3::NEG_Y, Vec3::Y, 0.105),
            contact(Vec3::X, Vec3::NEG_X, 0.055),
        ];
        solve_contacts(&mut b, &cs, &PARAMS);
        assert!(b.pos.distance(Vec3::new(-0.05, 0.1, 0.0)) < 1e-6, "{b:?}");
    }

    #[test]
    fn push_out_under_overhang_converges_over_iterations() {
        let n = Vec3::new(1.0, -1.0, 0.0).normalize();
        let shift = |iterations: u32| {
            let mut b = ball(Vec3::ZERO);
            let cs = [contact(Vec3::NEG_Y, Vec3::Y, 0.105), contact(-n, n, 0.105)];
            let params = ContactParams {
                iterations,
                max_correction: 1.0,
                ..PARAMS
            };
            solve_contacts(&mut b, &cs, &params);
            b.pos
        };
        let s = shift(8);
        assert!(
            (s.y - 0.1).abs() < 2e-3 && (s.dot(n) - 0.1).abs() < 2e-3,
            "{s:?}"
        );
        assert!(shift(1).distance(s) > 0.01, "{:?}", shift(1));
    }

    #[test]
    fn effective_mass_includes_rotation() {
        let mut b = car(Vec3::new(3.0, 1.0, -2.0), Quat::from_rotation_y(0.3));
        b.vel = Vec3::new(0.0, -2.0, 0.0);
        b.ang_vel = Vec3::new(0.5, 0.0, 0.2);
        let p = b.pos + Vec3::new(0.9, -0.5, 1.7);
        let params = ContactParams {
            friction: 0.0,
            iterations: 1,
            ..PARAMS
        };
        solve_contacts(&mut b, &[contact(p, Vec3::Y, 0.0)], &params);
        assert!(b.velocity_at(p).y.abs() < 1e-5, "{b:?}");
        assert!(b.ang_vel.length() > 0.1, "{b:?}");
    }

    #[test]
    fn separating_contact_pulls_nothing() {
        let mut b = ball(Vec3::ZERO);
        b.vel = Vec3::new(0.0, 1.0, 0.0);
        solve_contacts(&mut b, &[contact(Vec3::NEG_Y, Vec3::Y, 0.0)], &PARAMS);
        assert_eq!(b.vel, Vec3::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn fast_ball_does_not_pass_thin_wall() {
        let mut tris = rect(Vec3::new(5.0, 0.0, 0.0), Vec3::Z * 2.0, Vec3::Y * 2.0).to_vec();
        tris.extend(rect(
            Vec3::new(5.48, 0.0, 0.0),
            Vec3::Y * 2.0,
            Vec3::Z * 2.0,
        ));
        let world = StaticWorld::new(4.0, tris);
        let mut b = ball(Vec3::ZERO);
        b.vel = Vec3::new(50.0, 0.0, 0.0);
        let mut touched = false;
        for _ in 0..240 {
            b.integrate_velocity(DT, Vec3::ZERO);
            let cs = contacts(&world, &b, &[Vec3::ZERO]);
            touched |= !cs.is_empty();
            solve_contacts(&mut b, &cs, &PARAMS);
            b.integrate_position(DT);
            assert!(b.pos.x < 5.0 - R + 0.21, "{b:?}");
        }
        assert!(touched);
        assert!(b.vel.x <= 1e-5, "{b:?}");
    }

    #[test]
    fn contact_order_does_not_change_result() {
        let world = plane(Quat::IDENTITY);
        let rot = Quat::from_rotation_x(0.05) * Quat::from_rotation_z(-0.04);
        let mut b = car(Vec3::new(0.0, 0.38, 0.0), rot);
        b.vel = Vec3::new(4.0, -3.0, 1.0);
        b.ang_vel = Vec3::new(0.3, 1.0, -0.4);
        let cs = contacts(&world, &b, &CORNERS);
        assert!(cs.len() >= 3, "{cs:?}");
        let mut forward = b;
        solve_contacts(&mut forward, &cs, &PARAMS);
        let mut reversed = b;
        let rev: Vec<Contact> = cs.iter().rev().copied().collect();
        solve_contacts(&mut reversed, &rev, &PARAMS);
        assert_eq!(forward, reversed);
    }

    #[test]
    fn close_contacts_with_same_normal_merge_into_deepest() {
        let a = contact(Vec3::ZERO, Vec3::Y, 0.02);
        let b = contact(Vec3::new(0.009, 0.0, 0.0), Vec3::Y, 0.03);
        let far = contact(Vec3::new(0.011, 0.0, 0.0), Vec3::Y, 0.02);
        let tilted = contact(Vec3::ZERO, Vec3::new(0.0, 1.0, 0.15).normalize(), 0.02);
        assert_eq!(reduce(&[a, b]), vec![b]);
        assert_eq!(reduce(&[b, a]), vec![b]);
        assert_eq!(reduce(&[a, far]), vec![a, far]);
        assert_eq!(reduce(&[a, tilted]), vec![tilted, a]);
        assert_eq!(reduce(&[far, a]), vec![a, far]);
        assert_eq!(reduce(&[a, a]), vec![a]);
        let edge = contact(Vec3::new(MERGE_DIST, 0.0, 0.0), Vec3::Y, 0.02);
        assert_eq!(reduce(&[a, edge]), vec![a, edge]);
        let skew = contact(Vec3::ZERO, Vec3::new(0.0, MERGE_COS, 0.1), 0.02);
        assert_eq!(reduce(&[a, skew]), vec![skew, a]);
    }
}
