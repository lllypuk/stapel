use engine::{IVec2, Mat3, Mat4, Model, Pose, Quat, Vec2, Vec3, vec3};

use crate::map::EnemyKind;
use crate::obstacle::Obstacles;
use crate::player::{Player, walk};
use crate::weapon::{Owner, Projectile, Target};
use crate::{CELL, CHARACTER_SCALE, standing};

/// Радиус круга на плане, клетки.
pub(crate) const RADIUS: f32 = 0.25;
/// Дальность взгляда, клетки.
pub(crate) const SIGHT: f32 = 12.0;
/// Тиков между пересчётами пути к игроку.
const REPATH: u32 = 15;
/// Скорость снаряда стрелка, клеток в секунду.
pub(crate) const BOLT_SPEED: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Sleep,
    Chase,
    Attack,
    Dead,
}

/// Клипы Blocky Characters, которые играют враги.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Anim {
    Idle,
    Walk,
    Sprint,
    Die,
    Shoot,
    Melee,
    Kick,
}

impl Anim {
    const ALL: [Self; 7] = [
        Self::Idle,
        Self::Walk,
        Self::Sprint,
        Self::Die,
        Self::Shoot,
        Self::Melee,
        Self::Kick,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Walk => "walk",
            Self::Sprint => "sprint",
            Self::Die => "die",
            Self::Shoot => "holding-right-shoot",
            Self::Melee => "attack-melee-right",
            Self::Kick => "attack-kick-right",
        }
    }
}

/// Индексы клипов `Anim` и узла `arm-right` в модели персонажа: порядок клипов у файлов свой.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Clips {
    anims: [usize; 7],
    pub(crate) arm: usize,
}

impl Clips {
    /// # Panics
    /// Если в модели нет клипа `Anim` или узла `arm-right`.
    pub(crate) fn new(model: &Model) -> Self {
        fn find<'a>(name: &str, mut names: impl Iterator<Item = &'a String>) -> usize {
            names
                .position(|n| n == name)
                .unwrap_or_else(|| panic!("нет {name}"))
        }
        Self {
            anims: Anim::ALL.map(|a| find(a.name(), model.animations.iter().map(|a| &a.name))),
            arm: find("arm-right", model.nodes.iter().map(|n| &n.name)),
        }
    }

    pub(crate) fn get(self, anim: Anim) -> usize {
        self.anims[anim as usize]
    }
}

/// Свойства типа врага.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stats {
    pub(crate) hp: i32,
    /// Клеток в секунду.
    pub(crate) speed: f32,
    /// Дистанция атаки до центра игрока, клетки.
    pub(crate) reach: f32,
    pub(crate) damage: i32,
    /// Пауза между ударами или выстрелами, с.
    pub(crate) cooldown: f32,
    pub(crate) chase: Anim,
    pub(crate) attack: Anim,
}

pub(crate) fn stats(kind: EnemyKind) -> Stats {
    match kind {
        EnemyKind::Brawler => Stats {
            hp: 150,
            speed: 2.0,
            reach: 0.7,
            damage: 20,
            cooldown: 1.0,
            chase: Anim::Sprint,
            attack: Anim::Melee,
        },
        EnemyKind::Gunner => Stats {
            hp: 60,
            speed: 1.5,
            reach: 5.0,
            damage: 10,
            cooldown: 1.2,
            chase: Anim::Walk,
            attack: Anim::Shoot,
        },
        EnemyKind::Runner => Stats {
            hp: 40,
            speed: 3.5,
            reach: 0.6,
            damage: 8,
            cooldown: 0.6,
            chase: Anim::Sprint,
            attack: Anim::Kick,
        },
    }
}

/// Кисть — низ меша `arm-right`; ствол бластера (−Z) — вдоль руки (−Y), верх — вперёд руки (+Z).
fn grip() -> Mat4 {
    Mat4::from_translation(vec3(-0.2, -1.0, 0.0))
        * Mat4::from_mat3(Mat3::from_cols(Vec3::NEG_X, Vec3::Z, Vec3::Y))
}

/// Враг — круг на плане в клетках.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Enemy {
    pub(crate) kind: EnemyKind,
    /// Позиция прошлого тика, для интерполяции.
    pub(crate) prev: Vec2,
    pub(crate) pos: Vec2,
    /// Поворот модели вокруг +Y: модель смотрит в +Z.
    pub(crate) yaw: f32,
    pub(crate) hp: i32,
    pub(crate) state: State,
    /// Время клипа, с: с входа в состояние или с последнего удара.
    pub(crate) t: f32,
    cooldown: f32,
    /// Клетки пути к игроку от последней.
    path: Vec<IVec2>,
    repath: u32,
    /// `Obstacles::closed_doors` на момент пересчёта пути.
    doors: usize,
}

impl Enemy {
    pub(crate) fn new(kind: EnemyKind, pos: Vec2) -> Self {
        Self {
            kind,
            prev: pos,
            pos,
            yaw: 0.0,
            hp: stats(kind).hp,
            state: State::Sleep,
            t: 0.0,
            cooldown: 0.0,
            path: Vec::new(),
            repath: 0,
            doors: 0,
        }
    }

    /// Круг для выстрелов; у мёртвого нет.
    pub(crate) fn target(&self) -> Option<Target> {
        (self.state != State::Dead).then_some(Target {
            pos: self.pos,
            radius: RADIUS,
        })
    }

    /// Урон будит; при нуле здоровья — смерть.
    pub(crate) fn hurt(&mut self, damage: i32) {
        if self.state == State::Dead {
            return;
        }
        self.hp = (self.hp - damage).max(0);
        if self.hp == 0 {
            self.enter(State::Dead);
        } else if self.state == State::Sleep {
            self.enter(State::Chase);
        }
    }

    /// Тик: ближний удар бьёт игрока сразу, выстрел стрелка отдаётся снарядом.
    pub(crate) fn update(
        &mut self,
        dt: f32,
        player: &mut Player,
        obstacles: &Obstacles,
    ) -> Option<Projectile> {
        self.prev = self.pos;
        self.t += dt;
        self.cooldown = (self.cooldown - dt).max(0.0);
        if self.state == State::Dead || !player.alive() {
            return None;
        }
        let to = player.pos - self.pos;
        let sees = to.length() <= SIGHT && obstacles.segment(self.pos, player.pos).is_none();
        if self.state == State::Sleep && !sees {
            return None;
        }
        let s = stats(self.kind);
        if !(sees && to.length() <= s.reach) {
            self.enter(State::Chase);
            self.chase(s.speed * dt, player.pos, obstacles);
            return None;
        }
        self.enter(State::Attack);
        self.yaw = facing(to);
        if self.cooldown > 0.0 {
            return None;
        }
        self.cooldown = s.cooldown;
        self.t = 0.0;
        if self.kind == EnemyKind::Gunner {
            return Some(Projectile::new(
                self.pos,
                to,
                BOLT_SPEED,
                s.damage,
                Owner::Enemy,
            ));
        }
        player.hurt(s.damage);
        None
    }

    fn enter(&mut self, state: State) {
        if self.state != state {
            self.state = state;
            self.t = 0.0;
        }
    }

    /// Шаг `step` по центрам клеток пути; в клетке игрока — прямо к нему.
    fn chase(&mut self, step: f32, goal: Vec2, obstacles: &Obstacles) {
        let (cell, goal_cell) = (self.pos.floor().as_ivec2(), goal.floor().as_ivec2());
        let lost = self.path.is_empty() && cell != goal_cell;
        if self.repath == 0 || lost || self.doors != obstacles.closed_doors() {
            self.path = obstacles.path(cell, goal_cell).unwrap_or_default();
            self.path.reverse();
            self.repath = REPATH;
            self.doors = obstacles.closed_doors();
        }
        self.repath -= 1;
        let to = match self.path.last() {
            Some(c) => c.as_vec2() + 0.5,
            None if cell == goal_cell => goal,
            None => return,
        };
        let d = to - self.pos;
        if d.length() <= step {
            self.path.pop();
        }
        if d != Vec2::ZERO {
            self.yaw = facing(d);
        }
        self.pos = walk(self.pos, d.clamp_length_max(step), RADIUS, obstacles);
    }

    pub(crate) fn anim(&self) -> Anim {
        let s = stats(self.kind);
        match self.state {
            State::Sleep => Anim::Idle,
            State::Chase => s.chase,
            State::Attack => s.attack,
            State::Dead => Anim::Die,
        }
    }

    /// Поза клипа состояния через `ahead` с после тика; `die` замирает на последнем ключе.
    pub(crate) fn pose(&self, model: &Model, clips: Clips, ahead: f32, pose: &mut Pose) {
        let anim = self.anim();
        model.pose(clips.get(anim), self.t + ahead, anim != Anim::Die, pose);
    }

    /// Модель в мире между тиками: стоит на полу, смотрит по `yaw`.
    pub(crate) fn transform(&self, alpha: f32, model: &Model) -> Mat4 {
        let p = self.prev.lerp(self.pos, alpha) * CELL;
        Mat4::from_rotation_translation(Quat::from_rotation_y(self.yaw), vec3(p.x, 0.0, p.y))
            * standing(model, CHARACTER_SCALE)
    }
}

/// Бластер в правой руке врага с мировой матрицей `transform` в позе `pose`.
pub(crate) fn blaster(transform: Mat4, pose: &Pose, clips: Clips) -> Mat4 {
    transform * pose.globals[clips.arm] * grip()
}

/// `yaw` модели, смотрящей вдоль `d` на плане.
fn facing(d: Vec2) -> f32 {
    d.x.atan2(d.y)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use engine::{Assets, ElementState::Pressed, Handle, Input, KeyCode, ivec2, vec2};

    use super::*;
    use crate::map::{Dir, Map};
    use crate::player::{self, MAX_HP};
    use crate::weapon::Impact;

    const DT: f32 = 1.0 / 60.0;

    struct Level {
        enemies: Vec<Enemy>,
        player: Player,
        obstacles: Obstacles,
    }

    impl Level {
        fn new(text: &str) -> Self {
            let map = Map::parse(text).unwrap();
            Self {
                enemies: (map.enemies.iter())
                    .map(|&(kind, cell)| Enemy::new(kind, cell.as_vec2() + 0.5))
                    .collect(),
                player: Player::new(map.player.as_vec2() + 0.5, map.facing.yaw()),
                obstacles: Obstacles::new(&map),
            }
        }

        /// Тик всех врагов; выпущенные снаряды.
        fn tick(&mut self) -> Vec<Projectile> {
            (self.enemies.iter_mut())
                .filter_map(|e| e.update(DT, &mut self.player, &self.obstacles))
                .collect()
        }

        fn enemy(&self) -> &Enemy {
            &self.enemies[0]
        }

        fn distance(&self) -> f32 {
            self.enemy().pos.distance(self.player.pos)
        }
    }

    /// Расстояние от центра до ближайшей твёрдой клетки вокруг.
    fn clearance(p: Vec2, o: &Obstacles) -> f32 {
        let c = p.floor().as_ivec2();
        (-1..=1)
            .flat_map(|y| (-1..=1).map(move |x| c + ivec2(x, y)))
            .filter(|&n| o.solid(n))
            .map(|n| p.distance(p.clamp(n.as_vec2(), n.as_vec2() + 1.0)))
            .fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn sleeps_until_it_sees_player() {
        let mut l = Level::new("#########\n#P..T..m#\n#.......#\n#X......#\n#########\n");
        for _ in 0..60 {
            l.tick();
        }
        assert_eq!(l.enemy().state, State::Sleep, "дерево закрывает");
        assert_eq!(l.enemy().pos, vec2(7.5, 1.5));
        l.player.pos = vec2(1.5, 3.5);
        l.tick();
        assert_eq!(l.enemy().state, State::Chase);
        assert_ne!(l.enemy().pos, vec2(7.5, 1.5), "в том же тике идёт");
    }

    #[test]
    fn sight_is_limited() {
        let row = |gap: usize| format!("P{}mX\n", ".".repeat(gap));
        assert_eq!(SIGHT, 12.0);
        let mut far = Level::new(&row(12));
        far.tick();
        assert_eq!(far.enemy().state, State::Sleep);
        let mut near = Level::new(&row(11));
        near.tick();
        assert_eq!(near.enemy().state, State::Chase);
    }

    #[test]
    fn hurt_wakes_and_kills() {
        let mut l = Level::new("#########\n#P..T..m#\n#X......#\n#########\n");
        let e = &mut l.enemies[0];
        assert_eq!(
            e.target(),
            Some(Target {
                pos: vec2(7.5, 1.5),
                radius: RADIUS
            })
        );
        e.hurt(10);
        assert_eq!((e.state, e.hp), (State::Chase, 140));
        e.t = 1.0;
        e.hurt(10);
        assert_eq!(e.t, 1.0, "раненый не перезапускает клип");
        e.hurt(500);
        assert_eq!((e.state, e.hp, e.t), (State::Dead, 0, 0.0));
        assert_eq!(e.target(), None);
        e.t = 1.0;
        e.hurt(10);
        assert_eq!((e.state, e.hp, e.t), (State::Dead, 0, 1.0));
    }

    #[test]
    fn dead_enemy_stays_and_does_not_strike() {
        let mut l = Level::new("#####\n#Pm.#\n#X..#\n#####\n");
        l.enemies[0].hurt(1000);
        l.enemies[0].pos = vec2(2.2, 1.5);
        for _ in 0..120 {
            assert_eq!(l.tick(), []);
        }
        assert_eq!(l.enemy().pos, vec2(2.2, 1.5));
        assert_eq!(l.enemy().prev, l.enemy().pos);
        assert_eq!(l.player.hp, MAX_HP);
        assert!((l.enemy().t - 2.0).abs() < 1e-3, "клип смерти идёт");
    }

    const DOOR: &str = "\
#######
#Pk.Dm#
#X#####
#######
";

    #[test]
    fn closed_door_blocks_sight_and_holds_walk() {
        let mut l = Level::new(DOOR);
        for _ in 0..60 {
            l.tick();
        }
        assert_eq!(l.enemy().state, State::Sleep);
        l.enemies[0].hurt(1);
        for _ in 0..120 {
            l.tick();
            assert_eq!(l.enemy().pos, vec2(5.5, 1.5), "без пути стоит");
        }
        assert_eq!(l.player.hp, MAX_HP);

        l.obstacles.open(ivec2(4, 1));
        l.tick();
        assert!(l.enemy().pos.x < 5.5, "путь пересчитан при открытии");
        for _ in 0..180 {
            l.tick();
        }
        assert_eq!(l.enemy().state, State::Attack);
        assert!(l.player.hp < MAX_HP);
    }

    #[test]
    fn sleeper_behind_door_wakes_after_open() {
        let mut l = Level::new(DOOR);
        l.tick();
        assert_eq!(l.enemy().state, State::Sleep);
        l.obstacles.open(ivec2(4, 1));
        l.tick();
        assert_eq!(l.enemy().state, State::Chase);
    }

    #[test]
    fn chases_along_path_around_wall() {
        let mut l = Level::new("#######\n#P...X#\n#TTTT.#\n#m....#\n#######\n");
        l.enemies[0].hurt(1);
        let mut east = 0.0f32;
        let mut ticks = 0;
        l.tick();
        assert!(
            (l.enemy().yaw - FRAC_PI_2).abs() < 1e-5,
            "смотрит по ходу на восток"
        );
        while l.enemy().state != State::Attack {
            l.tick();
            let p = l.enemy().pos;
            assert!(clearance(p, &l.obstacles) >= RADIUS - 1e-4, "{p}");
            east = east.max(p.x);
            ticks += 1;
            assert!(ticks < 400, "{p}");
        }
        assert!(east > 5.0, "обход через (5, 2)");
        assert!(ticks > 240, "путь ~9,5 клетки: {ticks}");
        assert!(l.distance() <= stats(EnemyKind::Brawler).reach);
    }

    const ROOM: &str = "#######\n#P....#\n#.....#\n#....m#\n#X....#\n#######\n";

    #[test]
    fn path_recomputed_once_per_repath_ticks() {
        let mut l = Level::new(ROOM);
        l.enemies[0].hurt(1);
        l.tick();
        assert_eq!(l.enemy().path.first(), Some(&ivec2(1, 1)));
        l.player.pos = vec2(1.5, 4.5);
        for _ in 1..REPATH {
            l.tick();
            assert_eq!(l.enemy().path.first(), Some(&ivec2(1, 1)), "старый путь");
        }
        l.tick();
        assert_eq!(l.enemy().path.first(), Some(&ivec2(1, 4)));
    }

    #[test]
    fn ended_path_recomputed_at_once() {
        let mut l = Level::new(ROOM);
        let doors = l.obstacles.closed_doors();
        let e = &mut l.enemies[0];
        (e.state, e.repath, e.doors) = (State::Chase, REPATH, doors);
        l.tick();
        assert_ne!(l.enemy().pos, vec2(5.5, 3.5));
    }

    #[test]
    fn in_player_cell_goes_straight_to_player() {
        let mut l = Level::new(ROOM);
        l.player.pos = vec2(1.9, 1.9);
        l.enemies[0].pos = vec2(1.1, 1.1);
        l.enemies[0].hurt(1);
        let before = l.distance();
        l.tick();
        assert_eq!(l.enemy().state, State::Chase);
        assert!(l.distance() < before);
    }

    #[test]
    fn opened_door_recomputes_path_at_once() {
        let mut l = Level::new("#######\n#P.D.m#\n#.###.#\n#....k#\n#X#####\n#######\n");
        l.enemies[0].hurt(1);
        l.tick();
        assert!(l.enemy().pos.y > 1.5, "в обход через юг");
        l.obstacles.open(ivec2(3, 1));
        l.tick();
        assert!(l.enemy().pos.x < 5.5, "через дверь: {}", l.enemy().pos);
    }

    #[test]
    fn path_follows_player_to_new_cell() {
        let mut l = Level::new("#######\n#P...X#\n#TTTT.#\n#m....#\n#######\n");
        l.enemies[0].hurt(1);
        for _ in 0..90 {
            l.tick();
        }
        l.player.pos = vec2(1.5, 3.5);
        for _ in 0..150 {
            l.tick();
        }
        assert_eq!(l.enemy().state, State::Attack, "{}", l.enemy().pos);
    }

    #[test]
    fn attack_starts_at_type_reach() {
        for (ch, kind) in [
            ('m', EnemyKind::Brawler),
            ('s', EnemyKind::Gunner),
            ('f', EnemyKind::Runner),
        ] {
            let s = stats(kind);
            let mut l = Level::new(&format!("P.........{ch}X\n"));
            let mut ticks = 0;
            while l.enemy().state != State::Attack {
                l.tick();
                ticks += 1;
                assert!(ticks < 600, "{kind:?}");
            }
            let d = l.distance();
            assert!(
                d <= s.reach && d > s.reach - s.speed * DT - 1e-4,
                "{kind:?}: {d}"
            );
            let travelled = (10.0 - s.reach) / s.speed / DT;
            assert!((ticks as f32 - travelled).abs() <= 3.0, "{kind:?}: {ticks}");
            assert!(l.enemy().yaw < 0.0, "смотрит на запад");
        }
    }

    #[test]
    fn brawler_strikes_adjacent_player_by_cooldown() {
        let mut l = Level::new("#####\n#Pm.#\n#X..#\n#####\n");
        l.enemies[0].pos = vec2(2.1, 1.5);
        let s = stats(EnemyKind::Brawler);
        let hits = |l: &Level| (MAX_HP - l.player.hp) / s.damage;
        l.tick();
        assert_eq!(hits(&l), 1, "сразу при входе в атаку");
        assert_eq!(l.enemy().state, State::Attack);
        assert_eq!(s.cooldown, 1.0, "пауза — 60 тиков");
        for _ in 0..58 {
            l.tick();
        }
        assert_eq!(hits(&l), 1);
        for _ in 0..4 {
            l.tick();
        }
        assert_eq!(hits(&l), 2);
        assert!(l.enemy().t < 4.0 * DT, "удар перезапускает клип");

        l.player.hp = 0;
        let pos = l.enemy().pos;
        for _ in 0..120 {
            l.tick();
        }
        assert_eq!((l.player.hp, l.enemy().pos), (0, pos), "мёртвого не бьёт");
    }

    #[test]
    fn brawler_does_not_strike_through_corner() {
        let mut l = Level::new("####\n#P##\n##m#\n#X.#\n####\n");
        l.player.pos = vec2(1.8, 1.8);
        l.enemies[0].pos = vec2(2.25, 2.25);
        l.enemies[0].hurt(1);
        for _ in 0..30 {
            l.tick();
        }
        assert_eq!(l.player.hp, MAX_HP);
    }

    #[test]
    fn gunner_keeps_distance_and_shoots_player() {
        let mut l = Level::new(&format!("P{}sX\n", ".".repeat(9)));
        let s = stats(EnemyKind::Gunner);
        let mut shots = Vec::new();
        for _ in 0..400 {
            shots.extend(l.tick());
        }
        let d = l.distance();
        assert!(d <= s.reach && d > s.reach - 0.1, "{d}");
        assert_eq!(l.player.hp, MAX_HP, "урон несёт снаряд");
        assert!((2..=4).contains(&shots.len()), "{}", shots.len());
        let mut bolt = shots[0];
        assert_eq!((bolt.owner, bolt.damage), (Owner::Enemy, s.damage));
        assert!(
            bolt.vel.abs_diff_eq(vec2(-BOLT_SPEED, 0.0), 1e-4),
            "{}",
            bolt.vel
        );
        let player = Target {
            pos: l.player.pos,
            radius: player::RADIUS,
        };
        let impact = (0..60).find_map(|_| bolt.step(DT, &l.obstacles, player, &[]));
        assert_eq!(impact, Some(Impact::Player));
    }

    #[test]
    fn runner_catches_fleeing_player_and_brawler_does_not() {
        for (ch, caught) in [('f', true), ('m', false)] {
            let mut l = Level::new(&format!("{ch}.P{}X\n", ".".repeat(30)));
            l.player.yaw = Dir::E.yaw();
            let mut input = Input::default();
            input.on_key(KeyCode::KeyW, Pressed, false);
            for _ in 0..240 {
                l.player.update(&input, DT, &l.obstacles);
                l.tick();
            }
            assert_eq!(l.player.hp < MAX_HP, caught, "{ch}");
        }
        assert!(stats(EnemyKind::Runner).speed > player::SPEED);
    }

    fn character(assets: &mut Assets, name: &str) -> Handle<Model> {
        assets.load_model(format!("{}characters/{name}.glb", crate::ASSETS))
    }

    #[test]
    fn clips_found_by_name_in_every_character() {
        let mut assets = Assets::default();
        for name in ["character-a", "character-b", "character-c"] {
            let h = character(&mut assets, name);
            let model = assets.model(h);
            let clips = Clips::new(model);
            for anim in Anim::ALL {
                assert_eq!(model.animations[clips.get(anim)].name, anim.name());
            }
            assert_eq!(model.nodes[clips.arm].name, "arm-right");
        }
    }

    #[test]
    #[should_panic(expected = "нет idle")]
    fn missing_clip_panics() {
        Clips::new(&Model::default());
    }

    #[test]
    fn state_selects_clip_of_kind() {
        let mut e = Enemy::new(EnemyKind::Brawler, Vec2::ZERO);
        for (kind, chase, attack) in [
            (EnemyKind::Brawler, Anim::Sprint, Anim::Melee),
            (EnemyKind::Gunner, Anim::Walk, Anim::Shoot),
            (EnemyKind::Runner, Anim::Sprint, Anim::Kick),
        ] {
            e.kind = kind;
            for (state, anim) in [
                (State::Sleep, Anim::Idle),
                (State::Chase, chase),
                (State::Attack, attack),
                (State::Dead, Anim::Die),
            ] {
                e.state = state;
                assert_eq!(e.anim(), anim, "{kind:?} {state:?}");
            }
        }
    }

    #[test]
    fn die_stops_on_last_key_and_others_loop() {
        let mut assets = Assets::default();
        let h = character(&mut assets, "character-a");
        let model = assets.model(h);
        let clips = Clips::new(model);
        let posed = |e: &Enemy, ahead| {
            let mut pose = Pose::default();
            e.pose(model, clips, ahead, &mut pose);
            pose
        };
        let at = |anim, t, looped| {
            let mut pose = Pose::default();
            model.pose(clips.get(anim), t, looped, &mut pose);
            pose
        };
        let mut e = Enemy::new(EnemyKind::Brawler, Vec2::ZERO);
        e.hurt(1000);
        let die = model.animations[clips.get(Anim::Die)].duration;
        e.t = die + 0.3;
        assert_eq!(posed(&e, 0.2), at(Anim::Die, die, false));
        assert_ne!(posed(&e, 0.2), at(Anim::Die, die / 2.0, false));

        e.state = State::Chase;
        let sprint = model.animations[clips.get(Anim::Sprint)].duration;
        e.t = sprint + 0.125;
        assert_eq!(posed(&e, 0.125), at(Anim::Sprint, 0.25, false));
    }

    #[test]
    fn transform_stands_enemy_between_ticks_facing_yaw() {
        let mut assets = Assets::default();
        let h = character(&mut assets, "character-b");
        let model = assets.model(h);
        let mut e = Enemy::new(EnemyKind::Gunner, vec2(1.5, 2.5));
        e.pos = vec2(2.5, 2.5);
        e.yaw = facing(vec2(1.0, 0.0));
        let m = e.transform(0.25, model);
        let feet = model.bounds().unwrap().transformed(m);
        assert!(feet.min.y.abs() < 1e-5, "{feet:?}");
        let centre = m.transform_point3(Vec3::ZERO);
        assert!((centre.x - 1.75 * CELL).abs() < 1e-5 && (centre.z - 2.5 * CELL).abs() < 1e-5);
        let forward = m.transform_vector3(Vec3::Z).normalize();
        assert!(forward.abs_diff_eq(Vec3::X, 1e-5), "{forward}");
    }

    #[test]
    fn gunner_blaster_follows_right_arm() {
        let mut assets = Assets::default();
        let h = character(&mut assets, "character-b");
        let model = assets.model(h);
        let clips = Clips::new(model);
        let e = Enemy::new(EnemyKind::Gunner, vec2(1.5, 1.5));
        let m = e.transform(1.0, model);
        let hand = |pose: &Pose| blaster(m, pose, clips).transform_point3(Vec3::ZERO);

        let rest = hand(&model.rest);
        let body = m.transform_point3(Vec3::ZERO);
        assert!(
            rest.x < body.x - 0.1,
            "правая рука при взгляде в +Z — в −X: {rest}"
        );
        assert!(
            (0.3..0.8).contains(&rest.y),
            "кисть на уровне пояса: {rest}"
        );
        let tip = m * model.rest.globals[clips.arm];
        assert!(rest.abs_diff_eq(tip.transform_point3(vec3(-0.2, -1.0, 0.0)), 1e-5));

        let mut aim = Pose::default();
        model.pose(clips.get(Anim::Shoot), 0.2, true, &mut aim);
        assert!(
            hand(&aim).distance(rest) > 0.05,
            "рука поднята: {}",
            hand(&aim)
        );
        let barrel = blaster(m, &aim, clips).transform_vector3(Vec3::NEG_Z);
        let arm = (m * aim.globals[clips.arm]).transform_vector3(Vec3::NEG_Y);
        assert!(barrel.normalize().abs_diff_eq(arm.normalize(), 1e-4));
    }
}
