use engine::{Handle, Input, KeyCode, Model, Vec2, Vec3};

use crate::obstacle::Obstacles;

/// Дальность мгновенного выстрела, клетки.
pub(crate) const RANGE: f32 = 64.0;
/// Время возврата модели в покой после выстрела, с.
pub(crate) const RECOIL: f32 = 0.15;
/// Смещение модели в слое сразу после выстрела, м в осях камеры: вверх и назад.
const KICK: Vec3 = Vec3::new(0.0, 0.02, 0.08);

/// Круг на плане, в который попадают выстрел и снаряд.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Target {
    pub(crate) pos: Vec2,
    pub(crate) radius: f32,
}

impl Target {
    /// Расстояние вдоль единичного `dir` от `from` до входа в круг; изнутри — ноль.
    fn entry(self, from: Vec2, dir: Vec2) -> Option<f32> {
        let m = from - self.pos;
        let c = m.length_squared() - self.radius * self.radius;
        if c <= 0.0 {
            return Some(0.0);
        }
        let b = m.dot(dir);
        let disc = b * b - c;
        (b < 0.0 && disc >= 0.0).then(|| -b - disc.sqrt())
    }
}

/// Ближайшая цель вдоль единичного `dir` ближе `max`: индекс и расстояние.
fn nearest(from: Vec2, dir: Vec2, max: f32, targets: &[Target]) -> Option<(usize, f32)> {
    targets
        .iter()
        .enumerate()
        .filter_map(|(i, t)| Some((i, t.entry(from, dir)?)))
        .filter(|&(_, d)| d < max)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Hitscan,
    /// Скорость снаряда, клеток в секунду.
    Projectile {
        speed: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Weapon {
    pub(crate) model: Handle<Model>,
    pub(crate) damage: i32,
    /// Пауза между выстрелами, с.
    pub(crate) cooldown: f32,
    pub(crate) ammo: u32,
    pub(crate) kind: Kind,
}

/// Чей снаряд: он бьёт только другую сторону.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Player,
    Enemy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Impact {
    Wall,
    Player,
    Enemy(usize),
}

/// Снаряд на плане в клетках.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Projectile {
    /// Точка вылета.
    pub(crate) origin: Vec2,
    /// Позиция прошлого тика, для интерполяции.
    pub(crate) prev: Vec2,
    pub(crate) pos: Vec2,
    pub(crate) vel: Vec2,
    pub(crate) damage: i32,
    pub(crate) owner: Owner,
}

impl Projectile {
    pub(crate) fn new(pos: Vec2, dir: Vec2, speed: f32, damage: i32, owner: Owner) -> Self {
        Self {
            origin: pos,
            prev: pos,
            pos,
            vel: dir.normalize_or_zero() * speed,
            damage,
            owner,
        }
    }

    /// Тик: весь отрезок хода против препятствий и целей другой стороны; удар оставляет снаряд в точке удара.
    pub(crate) fn step(
        &mut self,
        dt: f32,
        obstacles: &Obstacles,
        player: Target,
        enemies: &[Target],
    ) -> Option<Impact> {
        self.prev = self.pos;
        let dir = self.vel.normalize_or_zero();
        let len = self.vel.length() * dt;
        let wall = obstacles.ray(self.pos, dir, len);
        let reach = wall.unwrap_or(len);
        let hit = match self.owner {
            Owner::Player => {
                nearest(self.pos, dir, reach, enemies).map(|(i, d)| (Impact::Enemy(i), d))
            }
            Owner::Enemy => {
                nearest(self.pos, dir, reach, &[player]).map(|(_, d)| (Impact::Player, d))
            }
        };
        let (impact, d) = match (hit, wall) {
            (Some(hit), _) => hit,
            (None, Some(w)) => (Impact::Wall, w),
            (None, None) => {
                self.pos += dir * len;
                return None;
            }
        };
        self.pos += dir * d;
        Some(impact)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Shot {
    /// `dist` — длина луча до входа в цель, клетки.
    Hit {
        target: usize,
        damage: i32,
        dist: f32,
    },
    /// `dist` — длина луча до стены или `RANGE`, клетки.
    Miss {
        dist: f32,
    },
    Launch(Projectile),
}

/// Оружие игрока: мгновенный бластер и снаряд, общая пауза между выстрелами и отдача.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Arsenal {
    pub(crate) weapons: [Weapon; 2],
    pub(crate) current: usize,
    cooldown: f32,
    recoil: f32,
}

impl Arsenal {
    pub(crate) fn new(blaster: Handle<Model>, launcher: Handle<Model>) -> Self {
        Self {
            weapons: [
                Weapon {
                    model: blaster,
                    damage: 25,
                    cooldown: 0.25,
                    ammo: 40,
                    kind: Kind::Hitscan,
                },
                Weapon {
                    model: launcher,
                    damage: 60,
                    cooldown: 0.75,
                    ammo: 10,
                    kind: Kind::Projectile { speed: 12.0 },
                },
            ],
            current: 0,
            cooldown: 0.0,
            recoil: 0.0,
        }
    }

    pub(crate) fn weapon(&self) -> &Weapon {
        &self.weapons[self.current]
    }

    /// Тик: 1 и 2 выбирают оружие, пауза и отдача идут.
    pub(crate) fn update(&mut self, input: &Input, dt: f32) {
        for (i, key) in [KeyCode::Digit1, KeyCode::Digit2].into_iter().enumerate() {
            if input.just_pressed(key) {
                self.current = i;
            }
        }
        self.cooldown = (self.cooldown - dt).max(0.0);
        self.recoil = (self.recoil - dt).max(0.0);
    }

    /// Выстрел из `from` вдоль `dir` по `targets`, если пауза прошла и есть патроны.
    pub(crate) fn fire(
        &mut self,
        from: Vec2,
        dir: Vec2,
        obstacles: &Obstacles,
        targets: &[Target],
    ) -> Option<Shot> {
        let w = &mut self.weapons[self.current];
        if self.cooldown > 0.0 || w.ammo == 0 {
            return None;
        }
        w.ammo -= 1;
        self.cooldown = w.cooldown;
        self.recoil = RECOIL;
        let dir = dir.normalize_or_zero();
        Some(match w.kind {
            Kind::Hitscan => {
                let wall = obstacles.ray(from, dir, RANGE).unwrap_or(RANGE);
                nearest(from, dir, wall, targets).map_or(
                    Shot::Miss { dist: wall },
                    |(target, dist)| Shot::Hit {
                        target,
                        damage: w.damage,
                        dist,
                    },
                )
            }
            Kind::Projectile { speed } => {
                Shot::Launch(Projectile::new(from, dir, speed, w.damage, Owner::Player))
            }
        })
    }

    /// Смещение модели в слое от отдачи, м в осях камеры; ноль — модель в покое.
    pub(crate) fn kick(&self) -> Vec3 {
        KICK * (self.recoil / RECOIL)
    }
}

#[cfg(test)]
mod tests {
    use engine::{Assets, ElementState::Pressed, ivec2, vec2};

    use super::*;
    use crate::map::Map;

    const DT: f32 = 1.0 / 60.0;

    fn obstacles(text: &str) -> Obstacles {
        Obstacles::new(&Map::parse(text).unwrap())
    }

    fn arsenal() -> Arsenal {
        let mut assets = Assets::default();
        let mut load = |name: &str| assets.load_model(format!("{}{name}.glb", crate::ASSETS));
        Arsenal::new(load("blaster/blaster-a"), load("blaster/blaster-h"))
    }

    fn target(x: f32, y: f32) -> Target {
        Target {
            pos: vec2(x, y),
            radius: 0.3,
        }
    }

    /// Попадание бластера в `target` с длиной луча `dist`.
    #[track_caller]
    fn assert_hit(shot: Option<Shot>, target: usize, dist: f32) {
        let Some(Shot::Hit {
            target: t,
            damage,
            dist: d,
        }) = shot
        else {
            panic!("{shot:?}");
        };
        assert_eq!((t, damage), (target, 25));
        assert!((d - dist).abs() < 1e-5 * dist.max(1.0), "{d} != {dist}");
    }

    fn near(a: Vec2, b: Vec2) -> bool {
        a.distance(b) < 1e-5
    }

    const HALL: &str = "\
##########
#P.......#
#........#
#X.......#
##########
";

    #[test]
    fn hitscan_hits_nearest_target_on_line() {
        let o = obstacles(HALL);
        let mut a = arsenal();
        let targets = [
            target(6.5, 1.5),
            target(3.5, 2.5),
            target(4.5, 1.6),
            target(2.5, 1.5),
        ];
        let shot = a.fire(vec2(1.5, 1.5), vec2(2.0, 0.0), &o, &targets[..3]);
        assert_hit(shot, 2, 3.0 - 0.08_f32.sqrt());
        assert_eq!(a.weapon().ammo, 39);

        let mut a = arsenal();
        let shot = a.fire(vec2(1.5, 1.5), Vec2::X, &o, &targets);
        assert_hit(shot, 3, 0.7);
        let mut a = arsenal();
        let shot = a.fire(vec2(1.5, 1.5), -Vec2::X, &o, &targets);
        assert_eq!(shot, Some(Shot::Miss { dist: 0.5 }), "цели позади");
        let mut a = arsenal();
        let shot = a.fire(vec2(1.5, 1.5), Vec2::Y, &o, &targets);
        assert_eq!(shot, Some(Shot::Miss { dist: 2.5 }), "луч мимо всех кругов");
    }

    #[test]
    fn hitscan_from_inside_target_hits_it() {
        let mut a = arsenal();
        let shot = a.fire(
            vec2(1.5, 1.5),
            Vec2::X,
            &obstacles(HALL),
            &[target(1.6, 1.5)],
        );
        assert_hit(shot, 0, 0.0);
    }

    #[test]
    fn hitscan_misses_behind_wall_and_closed_door() {
        let o = obstacles("#######\n#P.T.X#\n#.....#\n#######\n");
        let mut a = arsenal();
        assert_eq!(
            a.fire(vec2(1.5, 1.5), Vec2::X, &o, &[target(4.5, 1.5)]),
            Some(Shot::Miss { dist: 1.5 })
        );

        let mut o = obstacles("#######\n#P.D.X#\n#k....#\n#######\n");
        let mut a = arsenal();
        assert_eq!(
            a.fire(vec2(1.5, 1.5), Vec2::X, &o, &[target(4.5, 1.5)]),
            Some(Shot::Miss { dist: 1.5 })
        );
        o.open(ivec2(3, 1));
        let mut a = arsenal();
        assert_hit(
            a.fire(vec2(1.5, 1.5), Vec2::X, &o, &[target(4.5, 1.5)]),
            0,
            2.7,
        );
    }

    #[test]
    fn hitscan_reaches_range_in_open_field() {
        let row = format!("P{}X\n", ".".repeat(80));
        let o = obstacles(&row);
        let mut a = arsenal();
        let far = |x| [target(x, 0.5)];
        assert_hit(
            a.fire(vec2(0.5, 0.5), Vec2::X, &o, &far(RANGE)),
            0,
            RANGE - 0.8,
        );
        let mut a = arsenal();
        assert_eq!(
            a.fire(vec2(0.5, 0.5), Vec2::X, &o, &far(RANGE + 1.0)),
            Some(Shot::Miss { dist: RANGE })
        );
    }

    #[test]
    fn fire_waits_cooldown() {
        let o = obstacles(HALL);
        let mut a = arsenal();
        let input = Input::default();
        let fire = |a: &mut Arsenal| a.fire(vec2(1.5, 1.5), Vec2::X, &o, &[]);
        let half = a.weapon().cooldown / 2.0;
        assert_eq!(fire(&mut a), Some(Shot::Miss { dist: 7.5 }));
        assert_eq!(fire(&mut a), None, "в том же тике");
        a.update(&input, half);
        assert_eq!(fire(&mut a), None);
        a.update(&input, half);
        assert_eq!(fire(&mut a), Some(Shot::Miss { dist: 7.5 }));
        assert_eq!(a.weapon().ammo, 38);
    }

    #[test]
    fn ammo_runs_out() {
        let o = obstacles(HALL);
        let mut a = arsenal();
        a.weapons[0].ammo = 2;
        let input = Input::default();
        let mut shots = 0;
        for _ in 0..120 {
            a.update(&input, DT);
            shots += usize::from(a.fire(vec2(1.5, 1.5), Vec2::X, &o, &[]).is_some());
        }
        assert_eq!(shots, 2);
        assert_eq!(a.weapon().ammo, 0);
    }

    #[test]
    fn keys_switch_weapon_and_launcher_fires_projectile() {
        let o = obstacles(HALL);
        let mut a = arsenal();
        let mut input = Input::default();
        assert_eq!(a.current, 0);
        input.on_key(KeyCode::Digit2, Pressed, false);
        a.update(&input, DT);
        input.end_tick();
        assert_eq!(a.current, 1);
        assert_eq!(a.weapon().model, a.weapons[1].model);
        let shot = a.fire(vec2(1.5, 1.5), vec2(0.0, 2.0), &o, &[target(1.5, 2.5)]);
        assert_eq!(
            shot,
            Some(Shot::Launch(Projectile {
                origin: vec2(1.5, 1.5),
                prev: vec2(1.5, 1.5),
                pos: vec2(1.5, 1.5),
                vel: vec2(0.0, 12.0),
                damage: 60,
                owner: Owner::Player,
            }))
        );
        assert_eq!((a.weapons[0].ammo, a.weapons[1].ammo), (40, 9));

        a.update(&input, DT);
        assert_eq!(a.current, 1, "удержание не переключает");
        input.on_key(KeyCode::Digit1, Pressed, false);
        a.update(&input, DT);
        assert_eq!(a.current, 0);
    }

    #[test]
    fn recoil_kicks_model_and_returns_it_to_rest() {
        let o = obstacles(HALL);
        let mut a = arsenal();
        let input = Input::default();
        assert_eq!(a.kick(), Vec3::ZERO);
        a.fire(vec2(1.5, 1.5), Vec2::X, &o, &[]);
        assert_eq!(a.kick(), KICK);
        assert!(a.kick().y > 0.0 && a.kick().z > 0.0, "вверх и назад");
        let (mut last, mut ticks) = (a.kick(), 0);
        while a.kick() != Vec3::ZERO {
            a.update(&input, DT);
            assert!(a.kick().z < last.z);
            last = a.kick();
            ticks += 1;
            assert!(ticks <= 10, "покой через {RECOIL} с");
        }
        assert!(ticks >= 9, "{ticks}");
    }

    fn step(
        p: &mut Projectile,
        o: &Obstacles,
        player: Target,
        enemies: &[Target],
    ) -> Option<Impact> {
        p.step(DT, o, player, enemies)
    }

    #[test]
    fn projectile_flies_by_speed() {
        let o = obstacles(HALL);
        let mut p = Projectile::new(vec2(1.5, 1.5), vec2(3.0, 0.0), 6.0, 10, Owner::Player);
        assert_eq!(step(&mut p, &o, target(1.5, 3.5), &[]), None);
        assert_eq!(p.prev, vec2(1.5, 1.5));
        assert!(near(p.pos, vec2(1.6, 1.5)), "{}", p.pos);
        let first = p.pos;
        step(&mut p, &o, target(1.5, 3.5), &[]);
        assert_eq!(p.prev, first);
        assert!(near(p.pos, vec2(1.7, 1.5)), "{}", p.pos);
    }

    #[test]
    fn projectile_hits_nearest_enemy_at_its_edge() {
        let o = obstacles(HALL);
        let enemies = [target(3.5, 1.5), target(2.5, 1.5), target(1.5, 2.5)];
        let mut p = Projectile::new(vec2(1.5, 1.5), Vec2::X, 180.0, 10, Owner::Player);
        assert_eq!(
            step(&mut p, &o, target(1.5, 1.5), &enemies),
            Some(Impact::Enemy(1))
        );
        assert!(near(p.pos, vec2(2.2, 1.5)), "{}", p.pos);
    }

    #[test]
    fn projectile_dies_on_wall_and_does_not_pass_thin_wall() {
        let o = obstacles("#########\n#P..T..X#\n#.......#\n#########\n");
        let mut p = Projectile::new(vec2(1.5, 1.5), Vec2::X, 90.0, 10, Owner::Player);
        let behind = [target(5.5, 1.5)];
        let player = target(1.5, 2.5);
        assert_eq!(step(&mut p, &o, player, &behind), None);
        assert!(near(p.pos, vec2(3.0, 1.5)), "{}", p.pos);
        assert_eq!(step(&mut p, &o, player, &behind), Some(Impact::Wall));
        assert!(near(p.pos, vec2(4.0, 1.5)), "{}", p.pos);

        let mut fast = Projectile::new(vec2(1.5, 1.5), Vec2::X, 300.0, 10, Owner::Player);
        assert_eq!(
            step(&mut fast, &o, player, &behind),
            Some(Impact::Wall),
            "шаг 5 клеток"
        );
        assert!(near(fast.pos, vec2(4.0, 1.5)), "{}", fast.pos);
    }

    #[test]
    fn enemy_projectile_passes_enemies_and_hits_player() {
        let o = obstacles(HALL);
        let enemies = [target(2.5, 1.5)];
        let player = target(4.5, 1.5);
        let mut p = Projectile::new(vec2(1.5, 1.5), Vec2::X, 300.0, 10, Owner::Enemy);
        assert_eq!(step(&mut p, &o, player, &enemies), Some(Impact::Player));
        assert!(near(p.pos, vec2(4.2, 1.5)), "{}", p.pos);

        let mut own = Projectile::new(vec2(1.5, 1.5), Vec2::X, 300.0, 10, Owner::Player);
        assert_eq!(
            step(&mut own, &o, target(2.5, 1.5), &[player]),
            Some(Impact::Enemy(0)),
            "игрок своим снарядом не задет"
        );
    }
}
