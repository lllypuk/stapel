use engine::physics3d::Contact;
use std::f32::consts::TAU;

use engine::{
    ContactParams, Hit, Mat3, Mat4, Model, Pose, Quat, RigidBody, StaticWorld, Vec2, Vec3,
    box_inertia, solve_contacts,
};

use crate::car_config::{CarConfig, CarError, Drive, Wheel, Wheels, wheels};
use crate::layout::{BARRIER, GRASS};

/// Подшагов физики на тик 60 Гц.
pub(crate) const SUBSTEPS: u32 = 4;
/// Подшаг, с.
pub(crate) const SUBSTEP: f32 = 1.0 / 240.0;
pub(crate) const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);
/// Опора круче ~53° к оси подвески — не опора.
const MIN_GROUND_COS: f32 = 0.6;
/// Ограничитель хода жёстче пружины во столько раз.
const BUMP_STOP: f32 = 10.0;
/// Сопротивление качению — доля нагрузки колеса.
const ROLLING: f32 = 0.015;
/// Ниже этой скорости тяга не растёт: `power / v` уходит в бесконечность.
const MIN_DRIVE_SPEED: f32 = 1.0;
/// Сцепление травы относительно асфальта.
const GRASS_GRIP: f32 = 0.5;
pub(crate) const BODY_RADIUS: f32 = 0.25;
const BODY_CONTACT: ContactParams = ContactParams {
    friction: 0.4,
    restitution: 0.3,
    restitution_threshold: 2.0,
    slop: 0.005,
    max_correction: 0.05,
    iterations: 8,
};

/// Управление на тик.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Controls {
    /// 0..1.
    pub(crate) throttle: f32,
    /// 0..1.
    pub(crate) brake: f32,
    /// −1..1, плюс — влево.
    pub(crate) steer: f32,
    /// Ручник блокирует задние колёса.
    pub(crate) handbrake: bool,
}

/// Колесо после подшага.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct WheelState {
    /// Сжатие подвески от полного хода вниз, 0..`travel`.
    pub(crate) compression: f32,
    /// Угол поворота, рад, плюс — влево.
    pub(crate) steer: f32,
    /// `None` — колесо в воздухе.
    pub(crate) ground: Option<Hit>,
    /// Сила подвески, Н: величина — по ходу вдоль оси, направление — нормаль опоры.
    pub(crate) load: f32,
    /// Продольная сила шины после круга трения, Н, плюс — вперёд.
    pub(crate) traction: f32,
}

/// Машина: тело, колёса на лучах и сферы кузова против мира.
pub(crate) struct Car {
    pub(crate) body: RigidBody,
    pub(crate) wheels: [WheelState; 4],
    pub(crate) config: CarConfig,
    /// Центр масс и поворот перед последним тиком — для интерполяции.
    pub(crate) prev: (Vec3, Quat),
    /// Угол вращения колёс по пройденному пути, рад, 0..2π.
    pub(crate) spin: [f32; 4],
    mounts: Wheels,
    /// Центр масс в пространстве модели.
    com: Vec3,
    /// Центры сфер кузова от центра масс в осях тела.
    spheres: [Vec3; 6],
    contacts: Vec<Contact>,
}

/// Множитель сцепления поверхности по `tag`.
fn surface_grip(tag: u32) -> f32 {
    if tag == GRASS { GRASS_GRIP } else { 1.0 }
}

/// Угол колёс по рулю: от `steer_max` на месте до `steer_max·steer_at_speed` на `max_speed`.
pub(crate) fn steer_angle(config: &CarConfig, steer: f32, speed: f32) -> f32 {
    let k = (speed.abs() / config.max_speed).min(1.0);
    steer * config.steer_max.to_radians() * (1.0 + (config.steer_at_speed - 1.0) * k)
}

/// Сила мотора на машину при скорости `speed` вперёд, Н.
fn drive_force(config: &CarConfig, throttle: f32, speed: f32) -> f32 {
    if speed >= config.max_speed {
        return 0.0;
    }
    throttle * config.power / speed.max(MIN_DRIVE_SPEED)
}

fn driven(drive: Drive, wheel: usize) -> bool {
    match drive {
        Drive::Fwd => wheel < 2,
        Drive::Rwd => wheel >= 2,
        Drive::Awd => true,
    }
}

/// Оси шины в плоскости опоры с нормалью `n`: вперёд по `heading` и влево.
fn tire_axes(heading: Vec3, n: Vec3) -> (Vec3, Vec3) {
    let fwd = (heading - n * heading.dot(n)).normalize_or_zero();
    (fwd, n.cross(fwd))
}

/// Масса, которую видит импульс вдоль `dir` в точке `p`.
fn effective_mass(body: &RigidBody, inv_inertia: Mat3, p: Vec3, dir: Vec3) -> f32 {
    let rn = (p - body.pos).cross(dir);
    (body.inv_mass + rn.dot(inv_inertia * rn)).recip()
}

impl Car {
    /// Машина на земле в `pos` (начало модели) с рысканием `yaw`; колёса касаются земли на полном ходе вниз.
    pub(crate) fn new(
        config: CarConfig,
        model: &Model,
        pos: Vec3,
        yaw: f32,
    ) -> Result<Self, CarError> {
        let mounts = wheels(model)?;
        let b = model.bounds().expect("у колёс есть габариты");
        let com = (b.min + b.max) / 2.0 + Vec3::from(config.com_offset);
        let body = RigidBody::new(
            Vec3::ZERO,
            config.mass,
            box_inertia(config.mass, b.max - b.min),
        );
        let at = |x: f32, z: f32| Vec3::new(x, mounts[0].mount.y, z) - com;
        let (left, right) = (b.max.x - BODY_RADIUS, b.min.x + BODY_RADIUS);
        let (front, back) = (b.max.z - BODY_RADIUS, b.min.z + BODY_RADIUS);
        let mid = f32::midpoint(front, back);
        let spheres = [
            at(left, front),
            at(right, front),
            at(left, mid),
            at(right, mid),
            at(left, back),
            at(right, back),
        ];
        let mut car = Self {
            body,
            wheels: [WheelState::default(); 4],
            config,
            prev: (Vec3::ZERO, Quat::IDENTITY),
            spin: [0.0; 4],
            mounts,
            com,
            spheres,
            contacts: Vec::new(),
        };
        car.place(pos, yaw);
        Ok(car)
    }

    /// Ставит неподвижной в `pos` с рысканием `yaw`, как `new`; интерполяции из прежнего места нет.
    pub(crate) fn place(&mut self, pos: Vec3, yaw: f32) {
        let rot = Quat::from_rotation_y(yaw);
        let body = &mut self.body;
        body.pos = pos + rot * self.com + Vec3::Y * self.config.travel;
        body.rot = rot;
        body.vel = Vec3::ZERO;
        body.ang_vel = Vec3::ZERO;
        self.wheels = [WheelState::default(); 4];
        self.prev = (body.pos, body.rot);
    }

    /// Центр масс и поворот между `prev` и текущим состоянием.
    pub(crate) fn lerp(&self, alpha: f32) -> (Vec3, Quat) {
        (
            self.prev.0.lerp(self.body.pos, alpha),
            self.prev.1.slerp(self.body.rot, alpha),
        )
    }

    /// Модель → мир между `prev` и текущим состоянием.
    pub(crate) fn transform(&self, alpha: f32) -> Mat4 {
        let (pos, rot) = self.lerp(alpha);
        Mat4::from_rotation_translation(rot, pos) * Mat4::from_translation(-self.com)
    }

    /// Центр колеса `i` в пространстве модели: ниже крепления на несжатый ход.
    fn wheel_center(&self, i: usize) -> Vec3 {
        self.mounts[i].mount - Vec3::Y * (self.config.travel - self.wheels[i].compression)
    }

    /// Поза колёс: высота по ходу подвески, руль, вращение; колёса Car Kit — корневые узлы.
    pub(crate) fn pose(&self, model: &Model, pose: &mut Pose) {
        pose.clone_from(&model.rest);
        for (i, w) in self.mounts.iter().enumerate() {
            let local = pose.local_mut(w.node);
            local.translation += self.wheel_center(i) - w.mount;
            local.rotation = Quat::from_rotation_y(self.wheels[i].steer)
                * Quat::from_rotation_x(self.spin[i])
                * local.rotation;
        }
        model.recompose(pose);
    }

    /// Центры сфер кузова в мире.
    pub(crate) fn sphere_centers(&self) -> [Vec3; 6] {
        self.spheres.map(|s| self.body.pos + self.body.rot * s)
    }

    /// Скорость вдоль носа машины, м/с.
    pub(crate) fn forward_speed(&self) -> f32 {
        self.body.vel.dot(self.body.rot * Vec3::Z)
    }

    /// Тик 60 Гц подшагами.
    pub(crate) fn tick(&mut self, world: &StaticWorld, controls: Controls) {
        self.prev = (self.body.pos, self.body.rot);
        for _ in 0..SUBSTEPS {
            self.substep(world, controls, SUBSTEP);
        }
    }

    fn substep(&mut self, world: &StaticWorld, controls: Controls, dt: f32) {
        self.wheel_forces(world, controls, dt);
        self.body.integrate_velocity(dt, GRAVITY);
        self.contacts.clear();
        for c in self.sphere_centers() {
            world.sphere_contacts(c, BODY_RADIUS, &mut self.contacts);
        }
        solve_contacts(&mut self.body, &self.contacts, &BODY_CONTACT);
        self.body.integrate_position(dt);
        self.roll(dt);
    }

    /// Колёса поворачиваются на путь центра колеса вдоль его направления за подшаг.
    fn roll(&mut self, dt: f32) {
        for i in 0..4 {
            let center = self.body.pos + self.body.rot * (self.wheel_center(i) - self.com);
            let heading = self.body.rot * Quat::from_rotation_y(self.wheels[i].steer) * Vec3::Z;
            let path = self.body.velocity_at(center).dot(heading) * dt;
            self.spin[i] = (self.spin[i] + path / self.mounts[i].radius).rem_euclid(TAU);
        }
    }

    /// Подвеска и шины: силы копятся в теле до `integrate_velocity`.
    fn wheel_forces(&mut self, world: &StaticWorld, controls: Controls, dt: f32) {
        let speed = self.forward_speed();
        let cfg = &self.config;
        let body = &mut self.body;
        let up = body.rot * Vec3::Y;
        let steer = steer_angle(cfg, controls.steer, speed);
        let reach = |w: &Wheel| w.radius + cfg.travel;
        for (i, (state, mount)) in self.wheels.iter_mut().zip(&self.mounts).enumerate() {
            let origin = body.pos + body.rot * (mount.mount - self.com);
            *state = WheelState {
                steer: if i < 2 { steer } else { 0.0 },
                ..WheelState::default()
            };
            state.ground = world.raycast(origin, -up, reach(mount), |h| {
                h.tag != BARRIER && h.normal.dot(up) >= MIN_GROUND_COS
            });
        }
        let grounded = self.wheels.iter().filter(|w| w.ground.is_some()).count() as f32;
        let drive_count = (0..4).filter(|&i| driven(cfg.drive, i)).count() as f32;
        let drive = drive_force(cfg, controls.throttle, speed) / drive_count;
        let inv_inertia = body.inv_inertia_world();
        for (i, (state, mount)) in self.wheels.iter_mut().zip(&self.mounts).enumerate() {
            let Some(hit) = state.ground else {
                continue;
            };
            let origin = body.pos + body.rot * (mount.mount - self.com);
            let x = reach(mount) - hit.t;
            state.compression = x.min(cfg.travel);
            let bump = (x - cfg.travel).max(0.0) * BUMP_STOP * cfg.spring;
            let closing = -body.velocity_at(origin).dot(up);
            state.load = (cfg.spring * state.compression + bump + cfg.damper * closing).max(0.0);
            let center = origin - up * (hit.t - mount.radius);
            let n = hit.normal;
            body.apply_force_at(n * state.load, center);

            let heading = body.rot * Quat::from_rotation_y(state.steer) * Vec3::Z;
            let (fwd, side) = tire_axes(heading, n);
            let v = body.velocity_at(center);
            let (v_long, v_lat) = (v.dot(fwd), v.dot(side));
            let m_long = effective_mass(body, inv_inertia, center, fwd) / grounded;
            let m_lat = effective_mass(body, inv_inertia, center, side) / grounded;
            let resist = if controls.handbrake && i >= 2 {
                f32::INFINITY
            } else {
                controls.brake * cfg.brake / 4.0 + ROLLING * state.load
            };
            let mut long = (-m_long * v_long / dt).clamp(-resist, resist);
            if driven(cfg.drive, i) {
                long += drive;
            }
            let lat = -m_lat * v_lat / dt;
            let grip = if i < 2 { cfg.grip_front } else { cfg.grip_rear };
            let limit = grip * surface_grip(hit.tag) * state.load;
            let f = Vec2::new(long, lat).clamp_length_max(limit);
            state.traction = f.x;
            body.apply_force_at(fwd * f.x + side * f.y, center);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use engine::{Assets, Triangle, vec3};

    use super::*;
    use crate::ASSETS;
    use crate::layout::{Models, ROAD};
    use crate::pieces::{BARRIER_WALL, CARS, KIT_SCALE};

    /// Два треугольника квада `corners` лицом к `out`.
    fn quad(corners: [Vec3; 4], out: Vec3, tag: u32) -> [Triangle; 2] {
        let [a, b, c, d] = corners;
        let flip = (b - a).cross(c - a).dot(out) < 0.0;
        let tri = |p, q, r| Triangle {
            verts: if flip { [p, r, q] } else { [p, q, r] },
            tag,
        };
        [tri(a, b, c), tri(a, c, d)]
    }

    fn plane(tag: u32) -> Vec<Triangle> {
        let h = 2000.0;
        quad(
            [
                vec3(-h, 0.0, -h),
                vec3(h, 0.0, -h),
                vec3(h, 0.0, h),
                vec3(-h, 0.0, h),
            ],
            Vec3::Y,
            tag,
        )
        .to_vec()
    }

    /// Верх `barrierWall` на трассе, м.
    fn barrier_top() -> f32 {
        let mut assets = Assets::default();
        let models = Models::load(&mut assets);
        let b = assets
            .model(models.get("barrierWall"))
            .bounds()
            .expect("барьер");
        (b.max.y + BARRIER_WALL.offset.y) * KIT_SCALE
    }

    /// Стенка вдоль X от `z` толщиной 0,49 м высотой `top`.
    fn wall(z: f32, top: f32) -> Vec<Triangle> {
        let (x, z1) = (50.0, z + 0.49);
        let c = |x: f32, y: f32, z: f32| vec3(x, y, z);
        [
            quad(
                [c(-x, top, z), c(x, top, z), c(x, top, z1), c(-x, top, z1)],
                Vec3::Y,
                BARRIER,
            ),
            quad(
                [c(-x, 0.0, z), c(x, 0.0, z), c(x, top, z), c(-x, top, z)],
                Vec3::NEG_Z,
                BARRIER,
            ),
            quad(
                [c(-x, 0.0, z1), c(x, 0.0, z1), c(x, top, z1), c(-x, top, z1)],
                Vec3::Z,
                BARRIER,
            ),
        ]
        .concat()
    }

    fn world(tris: Vec<Triangle>) -> StaticWorld {
        StaticWorld::new(500.0, tris)
    }

    fn config(name: &str) -> CarConfig {
        let path = format!("{ASSETS}cars/{name}.toml");
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        CarConfig::parse(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn car_with(config: CarConfig, yaw: f32, y: f32) -> (Car, Assets) {
        let mut assets = Assets::default();
        let handle = assets.load_model(format!("{ASSETS}{}.glb", config.model));
        let car = Car::new(config, assets.model(handle), vec3(0.0, y, 0.0), yaw).expect("машина");
        (car, assets)
    }

    fn car(name: &str) -> Car {
        car_with(config(name), 0.0, 0.0).0
    }

    fn run(car: &mut Car, world: &StaticWorld, controls: Controls, ticks: u32) {
        for _ in 0..ticks {
            car.tick(world, controls);
        }
    }

    fn settled(name: &str, world: &StaticWorld) -> Car {
        let mut car = car(name);
        run(&mut car, world, Controls::default(), 3 * 60);
        car
    }

    #[test]
    fn rests_without_jitter_and_sags_by_weight() {
        let world = world(plane(ROAD));
        for name in CARS {
            let mut car = settled(name, &world);
            let start = car.body.pos;
            for _ in 0..60 {
                car.tick(&world, Controls::default());
                assert!(car.body.vel.length() < 0.01, "{name}: {}", car.body.vel);
                assert!(
                    car.body.ang_vel.length() < 0.01,
                    "{name}: {}",
                    car.body.ang_vel
                );
            }
            assert!(car.body.pos.distance(start) < 1e-3, "{name}");
            let load: f32 = car.wheels.iter().map(|w| w.load).sum();
            let weight = car.config.mass * -GRAVITY.y;
            assert!(
                (load / weight - 1.0).abs() < 0.01,
                "{name}: {load} / {weight}"
            );
            for w in car.wheels {
                let share = w.compression / car.config.travel;
                assert!((0.2..0.8).contains(&share), "{name}: {share}");
            }
        }
    }

    #[test]
    fn throttle_reaches_max_speed_and_no_further() {
        let world = world(plane(ROAD));
        let mut car = settled("race", &world);
        let gas = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        let mut top = 0.0f32;
        for _ in 0..25 * 60 {
            car.tick(&world, gas);
            top = top.max(car.forward_speed());
        }
        let max = car.config.max_speed;
        assert!(car.forward_speed() > max - 0.5, "{}", car.forward_speed());
        assert!(top < max + 0.1, "{top}");
    }

    /// Машина на `world`, катящаяся носом со скоростью `speed`.
    fn rolling(name: &str, world: &StaticWorld, speed: f32) -> Car {
        let mut car = settled(name, world);
        car.body.vel = car.body.rot * Vec3::Z * speed;
        car
    }

    fn braking_distance(tag: u32) -> f32 {
        let world = world(plane(tag));
        let mut car = rolling("race", &world, 20.0);
        let start = car.body.pos;
        let brake = Controls {
            brake: 1.0,
            ..Controls::default()
        };
        for _ in 0..5 * 60 {
            car.tick(&world, brake);
            assert!(car.forward_speed() > -0.05, "{}", car.forward_speed());
        }
        for _ in 0..60 {
            car.tick(&world, brake);
            assert!(car.body.vel.length() < 1e-3, "{}", car.body.vel);
        }
        car.body.pos.distance(start)
    }

    #[test]
    fn brake_stops_without_reversing_and_grass_grips_less() {
        let road = braking_distance(ROAD);
        // 20 м/с при замедлении около μ·g ≈ 11 м/с² — 18 м.
        assert!((12.0..30.0).contains(&road), "{road}");
        let grass = braking_distance(GRASS);
        assert!(grass > 1.5 * road, "{grass} / {road}");
    }

    #[test]
    fn steering_without_throttle_does_not_speed_up() {
        let world = world(plane(ROAD));
        let mut car = rolling("race", &world, 20.0);
        let turn = Controls {
            steer: 1.0,
            ..Controls::default()
        };
        let heading = car.body.rot * Vec3::Z;
        for _ in 0..3 * 60 {
            car.tick(&world, turn);
            assert!(car.body.vel.length() < 20.0 + 1e-3, "{}", car.body.vel);
        }
        let turned = heading.angle_between(car.body.rot * Vec3::Z);
        assert!(turned > 0.5, "{turned}");
    }

    #[test]
    fn slow_sideways_slide_dies_without_wobble() {
        let world = world(plane(ROAD));
        let mut car = settled("race", &world);
        let side = car.body.rot * Vec3::X;
        car.body.vel = side * 0.5;
        // Крен на подвеске проносит центр масс за ноль на доли процента и затухает.
        for tick in 0..60 {
            car.tick(&world, Controls::default());
            let v = car.body.vel.dot(side);
            assert!(v > -0.005, "{tick}: {v}");
            if tick >= 30 {
                assert!(v.abs() < 1e-3, "{tick}: {v}");
            }
        }
        assert!(car.body.vel.dot(side).abs() < 1e-4, "{}", car.body.vel);
        assert!(car.body.ang_vel.length() < 0.01, "{}", car.body.ang_vel);
    }

    #[test]
    fn traction_only_on_driven_wheels() {
        let world = world(plane(ROAD));
        let gas = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        for (drive, want) in [
            (Drive::Fwd, [true, true, false, false]),
            (Drive::Rwd, [false, false, true, true]),
            (Drive::Awd, [true; 4]),
        ] {
            let mut car = settled("race", &world);
            car.config.drive = drive;
            car.tick(&world, gas);
            for (w, driven) in car.wheels.iter().zip(want) {
                if driven {
                    assert!(w.traction > 500.0, "{drive:?}: {w:?}");
                } else {
                    assert!(w.traction.abs() < 50.0, "{drive:?}: {w:?}");
                }
            }
        }
    }

    #[test]
    fn steer_shrinks_with_speed_and_turns_front_wheels() {
        let cfg = config("race");
        let max = cfg.steer_max.to_radians();
        let at_speed = max * cfg.steer_at_speed;
        let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert!(near(steer_angle(&cfg, 1.0, 0.0), max));
        assert!(near(steer_angle(&cfg, -1.0, 0.0), -max));
        assert!(near(steer_angle(&cfg, 1.0, cfg.max_speed), at_speed));
        assert!(near(steer_angle(&cfg, 1.0, 2.0 * cfg.max_speed), at_speed));
        assert!(near(steer_angle(&cfg, 1.0, -cfg.max_speed), at_speed));
        let half = steer_angle(&cfg, 0.5, cfg.max_speed / 2.0);
        assert!(near(half, 0.5 * f32::midpoint(max, at_speed)), "{half}");

        let world = world(plane(ROAD));
        let mut car = settled("race", &world);
        car.tick(
            &world,
            Controls {
                steer: 1.0,
                ..Controls::default()
            },
        );
        let steer: Vec<f32> = car.wheels.iter().map(|w| w.steer).collect();
        assert!(
            (steer[0] - max).abs() < 1e-3 && near(steer[0], steer[1]),
            "{steer:?}"
        );
        assert_eq!(&steer[2..], &[0.0, 0.0]);
    }

    #[test]
    fn airborne_wheels_give_no_force() {
        let world = world(plane(ROAD));
        let (mut car, _) = car_with(config("race"), 0.3, 5.0);
        car.tick(
            &world,
            Controls {
                throttle: 1.0,
                brake: 0.0,
                steer: 1.0,
                handbrake: true,
            },
        );
        for w in car.wheels {
            assert_eq!(w.ground, None);
            assert_eq!((w.load, w.traction, w.compression), (0.0, 0.0, 0.0));
        }
        assert!(
            car.body.vel.abs_diff_eq(GRAVITY / 60.0, 1e-5),
            "{}",
            car.body.vel
        );
        assert_eq!(car.body.ang_vel, Vec3::ZERO);
    }

    #[test]
    fn bump_stop_caps_compression_and_pushes_harder() {
        let world = world(plane(ROAD));
        let mut car = settled("race", &world);
        let travel = car.config.travel;
        car.body.pos.y -= travel;
        car.substep(&world, Controls::default(), SUBSTEP);
        for w in car.wheels {
            assert_eq!(w.compression, travel);
            assert!(w.load > 2.0 * car.config.spring * travel, "{w:?}");
        }
    }

    #[test]
    fn barrier_at_30_degrees_bounces_body_not_wheels() {
        let top = barrier_top();
        let z = 15.0;
        let world = world([plane(ROAD), wall(z, top)].concat());
        let (mut car, _) = car_with(config("race"), 60f32.to_radians(), 0.0);
        run(&mut car, &world, Controls::default(), 2 * 60);
        let forward = car.body.rot * Vec3::Z;
        assert!((forward.z - 0.5).abs() < 1e-3, "{forward}");
        car.body.vel = forward * 30.0;
        let mut hit = false;
        for _ in 0..3 * 60 {
            car.tick(&world, Controls::default());
            for w in car.wheels {
                assert_ne!(w.ground.map(|h| h.tag), Some(BARRIER));
            }
            for c in car.sphere_centers() {
                assert!(c.z + BODY_RADIUS < z + 0.1, "{c}");
            }
            hit |= car.body.vel.z < 0.0;
        }
        assert!(hit, "не отскочила");
        assert!((car.body.rot * Vec3::Y).y > 0.9, "перевернулась");
    }

    #[test]
    fn body_spheres_sit_below_barrier_top_and_cover_wheel_track() {
        let top = barrier_top();
        let world = world(plane(ROAD));
        for name in CARS {
            let (mut car, mut assets) = car_with(config(name), 0.0, 0.0);
            run(&mut car, &world, Controls::default(), 3 * 60);
            for c in car.sphere_centers() {
                assert!(c.y < top, "{name}: {c} над {top}");
                assert!(c.y > BODY_RADIUS, "{name}: {c} в земле");
            }
            let handle = assets.load_model(format!("{ASSETS}{name}.glb"));
            let model = assets.model(handle);
            let local = car.spheres.map(|s| s + car.com);
            let reach = |f: fn(Vec3) -> f32| {
                local
                    .iter()
                    .map(|&s| f(s) + BODY_RADIUS)
                    .fold(f32::MIN, f32::max)
            };
            for w in car.mounts {
                let b = model.nodes[w.node]
                    .bounds
                    .expect("колесо")
                    .transformed(model.rest.globals[w.node]);
                assert!(reach(|s| s.x) >= b.max.x - 1e-4, "{name}: {}", b.max.x);
                assert!(reach(|s| -s.x) >= -b.min.x - 1e-4, "{name}: {}", b.min.x);
                assert!(reach(|s| s.z) >= b.max.z, "{name}: {}", b.max.z);
                assert!(reach(|s| -s.z) >= -b.min.z, "{name}: {}", b.min.z);
            }
        }
    }

    fn traction(car: &Car) -> f32 {
        car.wheels.iter().map(|w| w.traction).sum()
    }

    #[test]
    fn drive_force_is_power_over_speed() {
        let world = world(plane(ROAD));
        let gas = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        // Полный привод: на колесо четверть, сцепление не упирается.
        let mut car = rolling("race", &world, 30.0);
        car.config.drive = Drive::Awd;
        car.substep(&world, gas, SUBSTEP);
        let rolling = ROLLING * car.config.mass * -GRAVITY.y;
        let want = car.config.power / 30.0 - rolling;
        let got = traction(&car);
        assert!((got / want - 1.0).abs() < 0.02, "{got} / {want}");
        let cfg = config("race");
        assert_eq!(
            drive_force(&cfg, 0.5, 0.0),
            0.5 * cfg.power / MIN_DRIVE_SPEED
        );
        assert_eq!(drive_force(&cfg, 1.0, cfg.max_speed), 0.0);
    }

    #[test]
    fn brake_below_grip_and_rolling_resist_by_load() {
        let world = world(plane(ROAD));
        let weight = config("race").mass * -GRAVITY.y;
        for brake in [0.0, 0.5] {
            let mut car = rolling("race", &world, 20.0);
            car.substep(
                &world,
                Controls {
                    brake,
                    ..Controls::default()
                },
                SUBSTEP,
            );
            let want = -(brake * car.config.brake + ROLLING * weight);
            let got = traction(&car);
            assert!((got / want - 1.0).abs() < 0.01, "{brake}: {got} / {want}");
        }
    }

    #[test]
    fn traction_stops_at_axle_grip_times_load() {
        let world = world(plane(ROAD));
        let mut car = settled("race", &world);
        car.config.drive = Drive::Awd;
        car.config.grip_front = 1.6;
        car.config.grip_rear = 0.8;
        car.substep(
            &world,
            Controls {
                throttle: 1.0,
                ..Controls::default()
            },
            SUBSTEP,
        );
        for (i, w) in car.wheels.iter().enumerate() {
            let grip = if i < 2 { 1.6 } else { 0.8 };
            assert!(
                (w.traction / (grip * w.load) - 1.0).abs() < 1e-3,
                "{i}: {w:?}"
            );
        }
    }

    #[test]
    fn suspension_ray_skips_barrier_and_steep_faces() {
        let low = |x: f32, z: f32, y: f32| {
            quad(
                [
                    vec3(x - 0.2, y, z - 0.2),
                    vec3(x + 0.2, y, z - 0.2),
                    vec3(x + 0.2, y, z + 0.2),
                    vec3(x - 0.2, y, z + 0.2),
                ],
                Vec3::Y,
                BARRIER,
            )
        };
        let mut car = car("race");
        let [fl, fr, bl, _] = car.mounts.map(|w| w.mount);
        let normal = vec3(0.8, 0.5, 0.0).normalize();
        let steep = quad(
            [-0.3, 0.3]
                .map(|z| bl + vec3(0.0, -0.1, z))
                .iter()
                .flat_map(|&p| {
                    let along = normal.cross(Vec3::Z) * 0.3;
                    [p - along, p + along]
                })
                .collect::<Vec<_>>()
                .try_into()
                .expect("четыре угла"),
            normal,
            ROAD,
        );
        let tris = [plane(ROAD), low(fl.x, fl.z, 0.1).to_vec(), steep.to_vec()].concat();
        let world = world(tris);
        car.wheel_forces(&world, Controls::default(), SUBSTEP);
        for (w, mount) in car.wheels.iter().zip([fl, fr, bl]) {
            let hit = w.ground.expect("опора");
            assert_eq!(hit.tag, ROAD, "{mount}");
            assert!(hit.point.y.abs() < 1e-4, "{mount}: {}", hit.point);
        }
    }

    #[test]
    fn tick_keeps_prev_from_before_all_substeps_and_place_resets_it() {
        let world = world(plane(ROAD));
        let mut car = rolling("race", &world, 10.0);
        let before = (car.body.pos, car.body.rot);
        car.tick(&world, Controls::default());
        assert_eq!(car.prev, before);
        assert!(car.body.pos.distance(before.0) > 0.15, "{}", car.body.pos);
        let model =
            |pos, rot| Mat4::from_rotation_translation(rot, pos) * Mat4::from_translation(-car.com);
        assert!(
            car.transform(0.0)
                .abs_diff_eq(model(before.0, before.1), 1e-5)
        );
        assert!(
            car.transform(1.0)
                .abs_diff_eq(model(car.body.pos, car.body.rot), 1e-5)
        );
        let mid = car.transform(0.5).transform_point3(car.com);
        assert!(
            mid.abs_diff_eq(before.0.lerp(car.body.pos, 0.5), 1e-5),
            "{mid}"
        );

        car.place(vec3(3.0, 1.0, -2.0), 1.0);
        assert_eq!(car.prev, (car.body.pos, car.body.rot));
        assert_eq!((car.body.vel, car.body.ang_vel), (Vec3::ZERO, Vec3::ZERO));
        assert!(car.transform(0.0).abs_diff_eq(car.transform(1.0), 1e-6));
        let origin = car.transform(1.0).transform_point3(Vec3::ZERO);
        assert!(
            origin.abs_diff_eq(vec3(3.0, 1.0 + car.config.travel, -2.0), 1e-5),
            "{origin}"
        );
        let nose = car.body.rot * Vec3::Z;
        assert!(
            nose.abs_diff_eq(vec3(1f32.sin(), 0.0, 1f32.cos()), 1e-5),
            "{nose}"
        );
    }

    #[test]
    fn wheels_spin_by_path_traveled() {
        let world = world(plane(ROAD));
        let mut car = rolling("race", &world, 5.0);
        let heading = car.body.rot * Vec3::Z;
        let start = car.body.pos;
        // Путь меньше оборота колеса: угол сравним без свёртки.
        run(&mut car, &world, Controls::default(), 15);
        let path = (car.body.pos - start).dot(heading);
        assert!((1.0..1.8).contains(&path), "{path}");
        for (spin, w) in car.spin.iter().zip(car.mounts) {
            assert!((spin - path / w.radius).abs() < 0.01, "{spin} / {path}");
        }
        // В повороте влево наружное заднее колесо проходит больше внутреннего.
        let mut car = rolling("race", &world, 5.0);
        let left = Controls {
            steer: 1.0,
            ..Controls::default()
        };
        run(&mut car, &world, left, 15);
        assert!(car.spin[3] > 1.02 * car.spin[2], "{:?}", car.spin);
        car.spin = [TAU - 0.01; 4];
        car.tick(&world, Controls::default());
        assert!(
            car.spin.iter().all(|s| (0.0..0.5).contains(s)),
            "{:?}",
            car.spin
        );
    }

    #[test]
    fn wheel_pose_steers_spins_and_follows_suspension() {
        let world = world(plane(ROAD));
        for name in CARS {
            let (mut car, mut assets) = car_with(config(name), 0.0, 0.0);
            let handle = assets.load_model(format!("{ASSETS}{name}.glb"));
            let model = assets.model(handle);
            let mut pose = Pose::default();
            run(&mut car, &world, Controls::default(), 2 * 60);
            car.tick(
                &world,
                Controls {
                    steer: 1.0,
                    ..Controls::default()
                },
            );
            car.spin = [0.3, 0.3, 0.7, 0.7];
            car.pose(model, &mut pose);
            for (i, w) in car.mounts.iter().enumerate() {
                let g = pose.globals[w.node];
                let rest = model.rest.globals[w.node];
                let sag = car.config.travel - car.wheels[i].compression;
                let center = g.transform_point3(Vec3::ZERO);
                let want = rest.transform_point3(Vec3::ZERO) - Vec3::Y * sag;
                assert!(
                    center.abs_diff_eq(want, 1e-5),
                    "{name} {i}: {center} / {want}"
                );
                let (steer, spin) = (car.wheels[i].steer, car.spin[i]);
                assert_eq!(steer != 0.0, i < 2, "{name} {i}");
                let axle = g.transform_vector3(Vec3::X);
                assert!(
                    axle.abs_diff_eq(vec3(steer.cos(), 0.0, -steer.sin()), 1e-5),
                    "{name} {i}: {axle}"
                );
                let up = Quat::from_rotation_y(-steer) * g.transform_vector3(Vec3::Y);
                assert!(
                    up.abs_diff_eq(vec3(0.0, spin.cos(), spin.sin()), 1e-5),
                    "{name} {i}: {up}"
                );
            }
        }
    }

    #[test]
    fn airborne_wheels_hang_at_full_travel() {
        let world = world(plane(ROAD));
        let (mut car, mut assets) = car_with(config("race"), 0.0, 5.0);
        car.tick(&world, Controls::default());
        let handle = assets.load_model(format!("{ASSETS}race.glb"));
        let model = assets.model(handle);
        let mut pose = Pose::default();
        car.pose(model, &mut pose);
        for w in car.mounts {
            let drop = model.rest.globals[w.node].w_axis.y - pose.globals[w.node].w_axis.y;
            assert!((drop - car.config.travel).abs() < 1e-5, "{drop}");
        }
    }

    #[test]
    fn handbrake_locks_rear_wheels_only() {
        let world = world(plane(ROAD));
        let mut car = rolling("race", &world, 20.0);
        car.substep(
            &world,
            Controls {
                handbrake: true,
                ..Controls::default()
            },
            SUBSTEP,
        );
        for (i, w) in car.wheels.iter().enumerate() {
            if i < 2 {
                assert!(w.traction.abs() < ROLLING * w.load + 1.0, "{i}: {w:?}");
            } else {
                let limit = car.config.grip_rear * w.load;
                assert!((w.traction / -limit - 1.0).abs() < 0.01, "{i}: {w:?}");
            }
        }
    }

    #[test]
    fn tire_axes_lie_in_ground_plane() {
        let n = vec3(0.0, 0.94, -0.34).normalize();
        let heading = vec3(0.1, 0.0, 1.0).normalize();
        let (fwd, side) = tire_axes(heading, n);
        assert!(fwd.dot(n).abs() < 1e-6 && side.dot(n).abs() < 1e-6);
        assert!((fwd.length() - 1.0).abs() < 1e-6);
        assert!(side.abs_diff_eq(n.cross(fwd), 1e-6));
        assert!(fwd.dot(heading) > 0.9 && fwd.y > 0.3, "{fwd}");
    }
}
