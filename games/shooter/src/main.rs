#[cfg_attr(not(test), expect(dead_code, reason = "бот играет только в тестах"))]
mod bot;
mod enemy;
mod map;
mod obstacle;
mod player;
mod sound;
mod weapon;

use std::f32::consts::FRAC_PI_2;
use std::{fs, iter, mem};

use engine::{
    Aabb3, Assets, Camera2D, Camera3D, Color, Config, Context, DirectionalLight, Fog, Font,
    FontData, Game, Gfx, Handle, IVec2, KeyCode, Mat4, Mesh, MeshDraw, Model, ModelDraw,
    MouseButton, PointLight, Pose, Quat, Scene, Sprite, Vec2, Vec3, Vertex, Viewmodel, vec2, vec3,
};

use crate::enemy::{Clips, Enemy};
use crate::map::{Dir, EnemyKind, Map, Tile};
use crate::obstacle::Obstacles;
use crate::player::{MAX_HP, Player, RADIUS, forward};
use crate::sound::{Event, Sfx, SoundCmd, Sounds};
use crate::weapon::{Arsenal, Impact, Projectile, Shot, Target};

const TICK_HZ: u32 = 60;
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
const LEVEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/levels/level01.txt");
#[cfg(test)]
const LEVEL01: &str = include_str!("../assets/levels/level01.txt");
/// Сторона клетки карты, м.
const CELL: f32 = 2.0;
/// Единица Mini Forest — клетка.
const FOREST_SCALE: f32 = CELL;
/// Рост Blocky Characters — 2,7 единицы, персонаж — 1,8 м.
const CHARACTER_SCALE: f32 = 1.8 / 2.7;
/// Здоровье от аптечки.
const MEDKIT: i32 = 25;
/// Патроны пачки по оружиям `Arsenal::weapons`.
const AMMO_PACK: [u32; 2] = [20, 5];
/// Расстояние от центра игрока до центра клетки предмета, с которого он берётся, клетки.
const PICKUP: f32 = 0.5;
/// Игрок с ключом открывает дверь вплотную: упор в неё — ровно `RADIUS`.
const DOOR_REACH: f32 = RADIUS + 0.05;
/// Ворота Mini Dungeon шириной 0,8 единицы — во всю клетку.
const GATE_SCALE: f32 = CELL / 0.8;
const KEY_SCALE: f32 = 1.5;
const MEDKIT_SCALE: f32 = 1.0;
const AMMO_SCALE: f32 = 3.0;
const BULLET_SCALE: f32 = 8.0;
/// Высота полёта снарядов над землёй, м.
const SHOT_HEIGHT: f32 = 1.2;
/// Земля ниже травы клеток: на одной высоте они мерцают.
const GROUND_DROP: f32 = 0.02;
const GROUND: Color = Color {
    r: 0.12,
    g: 0.25,
    b: 0.06,
    a: 1.0,
};
const AMBIENT: Color = Color {
    r: 0.35,
    g: 0.38,
    b: 0.42,
    a: 1.0,
};
const SUN: DirectionalLight = DirectionalLight {
    dir: Vec3::new(0.4, -1.0, 0.3),
    color: Color {
        r: 1.0,
        g: 0.95,
        b: 0.85,
        a: 1.0,
    },
};
/// Им же очищается 3D-проход.
const FOG: Fog = Fog {
    color: Color {
        r: 0.55,
        g: 0.65,
        b: 0.7,
        a: 1.0,
    },
    start: 10.0,
    end: 40.0,
};
/// Свет над выходом: модели выхода в наборах нет.
const EXIT_LIGHT: Color = Color {
    r: 0.2,
    g: 1.0,
    b: 0.3,
    a: 1.0,
};
const EXIT_RANGE: f32 = 4.0;
/// Сколько держится вспышка выстрела, с: несколько тиков, иначе кадр между ними её пропустит.
const FLASH: f32 = 0.08;
const MUZZLE: Color = Color {
    r: 1.0,
    g: 0.7,
    b: 0.3,
    a: 1.0,
};
const MUZZLE_RANGE: f32 = 6.0;
/// Вспышка перед глазом, м.
const MUZZLE_AHEAD: f32 = 0.8;
/// Угол обзора слоя оружия: уже сцены, чтобы ствол не растягивался к краю.
const GUN_FOV: f32 = 55.0 * std::f32::consts::PI / 180.0;
const GUN_NEAR: f32 = 0.01;
const GUN_SCALE: f32 = 0.5;
/// Бластер в осях камеры: справа внизу перед глазом, м.
const GUN_AT: Vec3 = Vec3::new(0.22, -0.2, -0.45);
/// HUD в единицах окна 1280×720; иной аспект — letterbox.
const VIEW: Vec2 = vec2(1280.0, 720.0);
/// При `VIEW` в окне 1280×720 тексель атласа — пиксель экрана.
const HUD_PX: f32 = 24.0;
const HUD_MARGIN: f32 = 12.0;
const HUD_LAYER: i32 = 1;
/// Длина и толщина штриха прицела.
const CROSS: Vec2 = vec2(16.0, 2.0);
const WIN: &str = "EXIT FOUND";
const DEAD: &str = "YOU DIED";
const RESTART: &str = "PRESS ENTER";
const FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Regular.ttf"
);

/// Клипы, которые враги ищут по имени.
#[cfg(test)]
const CLIPS: [&str; 7] = [
    "idle",
    "walk",
    "sprint",
    "die",
    "holding-right-shoot",
    "attack-melee-right",
    "attack-kick-right",
];

struct Models {
    tree: Handle<Model>,
    tree_high: Handle<Model>,
    rock: Handle<Model>,
    fence: Handle<Model>,
    grass: Handle<Model>,
    brawler: Handle<Model>,
    gunner: Handle<Model>,
    runner: Handle<Model>,
    blaster: Handle<Model>,
    launcher: Handle<Model>,
    bullet: Handle<Model>,
    clip: Handle<Model>,
    crate_: Handle<Model>,
    key: Handle<Model>,
    gate: Handle<Model>,
}

impl Models {
    fn load(assets: &mut Assets) -> Self {
        let mut load = |name: &str| assets.load_model(format!("{ASSETS}{name}.glb"));
        Self {
            tree: load("forest/tree"),
            tree_high: load("forest/tree-high"),
            rock: load("forest/rocks-high"),
            fence: load("forest/fence"),
            grass: load("forest/patch-grass"),
            brawler: load("characters/character-a"),
            gunner: load("characters/character-b"),
            runner: load("characters/character-c"),
            blaster: load("blaster/blaster-a"),
            launcher: load("blaster/blaster-h"),
            bullet: load("blaster/bullet-foam"),
            clip: load("blaster/clip-small"),
            crate_: load("blaster/crate-small"),
            key: load("dungeon/key"),
            gate: load("dungeon/gate"),
        }
    }
}

/// Масштаб `scale` с подъёмом низа габарита на пол: `rocks-high` отцентрован по высоте.
fn standing(model: &Model, scale: f32) -> Mat4 {
    let lift = -model.bounds().map_or(0.0, |b: Aabb3| b.min.y) * scale;
    Mat4::from_translation(Vec3::Y * lift) * Mat4::from_scale(Vec3::splat(scale))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Door {
    cell: IVec2,
    /// Секунд с открытия ключом.
    opened: Option<f32>,
}

/// Состояние уровня, которое сбрасывает рестарт.
#[derive(Clone, Debug, PartialEq)]
struct Level {
    obstacles: Obstacles,
    player: Player,
    arsenal: Arsenal,
    enemies: Vec<Enemy>,
    projectiles: Vec<Projectile>,
    /// Не взятые.
    keys: Vec<IVec2>,
    /// Ключ в руках; один открывает все двери.
    key: bool,
    doors: Vec<Door>,
    /// Не взятые.
    medkits: Vec<IVec2>,
    /// Не взятые.
    ammo: Vec<IVec2>,
    won: bool,
    /// Секунд до конца вспышки выстрела.
    flash: f32,
    /// Звуки последнего тика.
    sounds: Vec<Event>,
}

impl Level {
    fn new(map: &Map, models: &Models) -> Self {
        let centre = |cell: IVec2| cell.as_vec2() + 0.5;
        Self {
            obstacles: Obstacles::new(map),
            player: Player::new(centre(map.player), map.facing.yaw()),
            arsenal: Arsenal::new(models.blaster, models.launcher),
            enemies: map
                .enemies
                .iter()
                .map(|&(kind, cell)| Enemy::new(kind, centre(cell)))
                .collect(),
            projectiles: Vec::new(),
            keys: map.keys.clone(),
            key: false,
            doors: map
                .doors
                .iter()
                .map(|&cell| Door { cell, opened: None })
                .collect(),
            medkits: map.medkits.clone(),
            ammo: map.ammo.clone(),
            won: false,
            flash: 0.0,
            sounds: Vec::new(),
        }
    }

    fn fire(&mut self) {
        let (targets, ids) = targets(&self.enemies);
        let p = &self.player;
        let Some(shot) = (self.arsenal).fire(p.pos, forward(p.yaw), &self.obstacles, &targets)
        else {
            return;
        };
        self.flash = FLASH;
        let sfx = [Sfx::Blaster, Sfx::Launcher][self.arsenal.current];
        self.sounds.push((sfx, self.player.pos));
        match shot {
            Shot::Hit { target, damage } => self.enemies[ids[target]].hurt(damage),
            Shot::Launch(projectile) => self.projectiles.push(projectile),
            Shot::Miss => {}
        }
    }

    fn pick_up(&mut self) {
        let pos = self.player.pos;
        let near = |cell: &IVec2| pos.distance(cell.as_vec2() + 0.5) < PICKUP;
        let keys = self.keys.len();
        self.keys.retain(|c| !near(c));
        let mut taken = keys - self.keys.len();
        self.key |= taken > 0;
        let hp = &mut self.player.hp;
        if *hp < MAX_HP
            && let Some(i) = self.medkits.iter().position(near)
        {
            self.medkits.remove(i);
            *hp = (*hp + MEDKIT).min(MAX_HP);
            taken += 1;
        }
        let packs = self.ammo.len();
        self.ammo.retain(|c| !near(c));
        taken += packs - self.ammo.len();
        self.sounds
            .extend(iter::repeat_n((Sfx::Pickup, pos), taken));
        for _ in self.ammo.len()..packs {
            for (w, n) in self.arsenal.weapons.iter_mut().zip(AMMO_PACK) {
                w.ammo += n;
            }
        }
    }

    /// Дверь открывает касание с ключом и пропускает только после `open_secs` анимации.
    fn doors(&mut self, dt: f32, open_secs: f32) {
        let pos = self.player.pos;
        for door in &mut self.doors {
            match &mut door.opened {
                Some(t) => {
                    *t += dt;
                    if *t >= open_secs {
                        self.obstacles.open(door.cell);
                    }
                }
                None if self.key && touches_cell(pos, door.cell) => {
                    door.opened = Some(0.0);
                    self.sounds.push((Sfx::Door, door.cell.as_vec2() + 0.5));
                }
                None => {}
            }
        }
    }

    /// Снаряды за тик; попавший пропадает.
    fn fly(&mut self, dt: f32) {
        let player = Target {
            pos: self.player.pos,
            radius: RADIUS,
        };
        let (targets, ids) = targets(&self.enemies);
        let mut projectiles = mem::take(&mut self.projectiles);
        projectiles.retain_mut(|p| match p.step(dt, &self.obstacles, player, &targets) {
            None => true,
            Some(Impact::Wall) => false,
            Some(Impact::Player) => {
                self.player.hurt(p.damage);
                false
            }
            Some(Impact::Enemy(i)) => {
                self.enemies[ids[i]].hurt(p.damage);
                false
            }
        });
        self.projectiles = projectiles;
    }
}

/// Цели живых врагов и их индексы в `enemies`.
fn targets(enemies: &[Enemy]) -> (Vec<Target>, Vec<usize>) {
    enemies
        .iter()
        .enumerate()
        .filter_map(|(i, e)| Some((e.target()?, i)))
        .unzip()
}

/// Центр игрока `pos` у самой клетки `cell`.
fn touches_cell(pos: Vec2, cell: IVec2) -> bool {
    let min = cell.as_vec2();
    pos.distance(pos.clamp(min, min + 1.0)) <= DOOR_REACH
}

/// Точка плана `p` в клетках — в мире на высоте `y`.
fn world(p: Vec2, y: f32) -> Vec3 {
    let p = p * CELL;
    vec3(p.x, y, p.y)
}

/// Модель в центре клетки `cell`, повёрнутая вокруг +Y на `yaw`, с матрицей `local` под поворотом.
fn at(model: Handle<Model>, cell: IVec2, yaw: f32, local: Mat4) -> ModelDraw {
    let pos = world(cell.as_vec2() + 0.5, 0.0);
    ModelDraw {
        model,
        transform: Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), pos) * local,
        pose: None,
        tint: Color::WHITE,
    }
}

/// Поворот по `yaw`, затем наклон по `pitch`; модель смотрит по −Z.
fn view(camera: &Camera3D) -> Quat {
    Quat::from_rotation_y(camera.yaw) * Quat::from_rotation_x(camera.pitch)
}

fn text_width(font: &FontData, text: &str) -> f32 {
    text.chars()
        .filter_map(|c| font.glyph(c).or_else(|| font.glyph('?')))
        .map(|g| g.advance)
        .sum()
}

/// Что лежит в соседней с `cell` клетке по `dir`; `None` — за краем карты.
fn beside(map: &Map, cell: IVec2, dir: Dir) -> Option<Tile> {
    map.tile(cell + dir.offset())
}

/// Модели клеток карты: деревья, камни, забор и трава.
fn statics(map: &Map, m: &Models, assets: &Assets) -> Vec<ModelDraw> {
    let placed = |model, cell, yaw| {
        at(
            model,
            cell,
            yaw,
            standing(assets.model(model), FOREST_SCALE),
        )
    };
    map.cells()
        .map(|cell| match map.tile(cell).expect("клетка карты") {
            Tile::Grass => placed(m.grass, cell, 0.0),
            Tile::Rock => placed(m.rock, cell, 0.0),
            // Высокие — в чаще, у поляны — низкие: с поляны видно дальше.
            Tile::Tree
                if Dir::ALL
                    .iter()
                    .all(|&d| beside(map, cell, d).is_some_and(Tile::solid)) =>
            {
                placed(m.tree_high, cell, 0.0)
            }
            Tile::Tree => placed(m.tree, cell, 0.0),
            // Пролёт забора в модели — вдоль X.
            Tile::Fence => {
                let along = |d| beside(map, cell, d) == Some(Tile::Fence);
                let yaw = if along(Dir::E) || along(Dir::W) {
                    0.0
                } else {
                    FRAC_PI_2
                };
                placed(m.fence, cell, yaw)
            }
        })
        .collect()
}

/// Квадрат 1×1 на XZ от начала, нормалью вверх; UV нулевые — текстура белая.
fn quad() -> (Vec<Vertex>, Vec<u32>) {
    let vertex = |x: f32, z: f32| Vertex {
        pos: [x, 0.0, z],
        normal: [0.0, 1.0, 0.0],
        ..Vertex::default()
    };
    let vertices = vec![
        vertex(0.0, 0.0),
        vertex(0.0, 1.0),
        vertex(1.0, 1.0),
        vertex(1.0, 0.0),
    ];
    (vertices, vec![0, 1, 2, 0, 2, 3])
}

struct Shooter {
    models: Models,
    map: Map,
    font: Handle<Font>,
    ground: Handle<Mesh>,
    /// Клетки карты: за уровень не меняются.
    statics: Vec<ModelDraw>,
    /// Клипы персонажей по `EnemyKind`.
    clips: [Clips; 3],
    /// Анимация `open` двери и её длительность, с.
    open: usize,
    open_secs: f32,
    sounds: Sounds,
    level: Level,
}

impl Shooter {
    fn new(map: Map, assets: &mut Assets) -> Self {
        let models = Models::load(assets);
        let animations = &assets.model(models.gate).animations;
        let open = (animations.iter().position(|a| a.name == "open")).expect("у двери есть open");
        let open_secs = animations[open].duration;
        let clips =
            [models.brawler, models.gunner, models.runner].map(|h| Clips::new(assets.model(h)));
        let (vertices, indices) = quad();
        Self {
            level: Level::new(&map, &models),
            statics: statics(&map, &models, assets),
            font: assets.load_font(FONT_PATH, HUD_PX),
            ground: assets.add_mesh(vertices, indices),
            sounds: Sounds::load(assets),
            clips,
            models,
            map,
            open,
            open_secs,
        }
    }

    fn character(&self, kind: EnemyKind) -> Handle<Model> {
        match kind {
            EnemyKind::Brawler => self.models.brawler,
            EnemyKind::Gunner => self.models.gunner,
            EnemyKind::Runner => self.models.runner,
        }
    }

    /// Мир кадра между тиками `alpha` длиной `dt`.
    fn scene(&self, camera: Camera3D, alpha: f32, dt: f32, assets: &Assets) -> Scene {
        let m = &self.models;
        let l = &self.level;
        let mut models = self.statics.clone();
        let items = [
            (m.key, &l.keys, KEY_SCALE),
            (m.crate_, &l.medkits, MEDKIT_SCALE),
            (m.clip, &l.ammo, AMMO_SCALE),
        ];
        for (model, cells, scale) in items {
            let local = standing(assets.model(model), scale);
            models.extend(cells.iter().map(|&c| at(model, c, 0.0, local)));
        }
        let gate = assets.model(m.gate);
        models.extend(l.doors.iter().map(|d| {
            let solid = |dir| beside(&self.map, d.cell, dir).is_some_and(Tile::solid);
            // Ворота в модели — вдоль X: между стенами по X стоят как есть.
            let yaw = if solid(Dir::E) || solid(Dir::W) {
                0.0
            } else {
                FRAC_PI_2
            };
            let pose = d.opened.map(|t| {
                let mut pose = Pose::default();
                gate.pose(self.open, t, false, &mut pose);
                pose
            });
            ModelDraw {
                pose,
                ..at(
                    m.gate,
                    d.cell,
                    yaw,
                    Mat4::from_scale(Vec3::splat(GATE_SCALE)),
                )
            }
        }));
        models.extend(self.characters(alpha, dt, assets));
        models.extend(l.projectiles.iter().map(|p| {
            let dir = vec3(p.vel.x, 0.0, p.vel.y).normalize();
            ModelDraw {
                model: m.bullet,
                transform: Mat4::from_scale_rotation_translation(
                    Vec3::splat(BULLET_SCALE),
                    Quat::from_rotation_arc(Vec3::Y, dir),
                    world(p.prev.lerp(p.pos, alpha), SHOT_HEIGHT),
                ),
                pose: None,
                tint: Color::WHITE,
            }
        }));
        let mut lights = vec![PointLight {
            pos: world(self.map.exit.as_vec2() + 0.5, 1.0),
            color: EXIT_LIGHT,
            range: EXIT_RANGE,
        }];
        if l.flash > 0.0 {
            lights.push(PointLight {
                pos: camera.pos + camera.forward() * MUZZLE_AHEAD,
                color: MUZZLE,
                range: MUZZLE_RANGE,
            });
        }
        let size = self.map.size.as_vec2() * CELL;
        Scene {
            camera,
            ambient: AMBIENT,
            fog: FOG,
            sun: Some(SUN),
            lights,
            meshes: vec![MeshDraw {
                mesh: self.ground,
                texture: Handle::WHITE,
                color: GROUND,
                transform: Mat4::from_scale_rotation_translation(
                    vec3(size.x, 1.0, size.y),
                    Quat::IDENTITY,
                    Vec3::NEG_Y * GROUND_DROP,
                ),
            }],
            models,
            viewmodel: self.viewmodel(&camera),
        }
    }

    /// Враги в позе клипа между тиками и бластеры стрелков.
    fn characters(&self, alpha: f32, dt: f32, assets: &Assets) -> Vec<ModelDraw> {
        let l = &self.level;
        let mut out = Vec::new();
        // После победы тики стоят, и клип качался бы между `t` и `t + dt` каждый кадр.
        let ahead = if l.won { 0.0 } else { alpha * dt };
        for e in &l.enemies {
            let handle = self.character(e.kind);
            let model = assets.model(handle);
            let clips = self.clips[e.kind as usize];
            let mut pose = Pose::default();
            e.pose(model, clips, ahead, &mut pose);
            let transform = e.transform(alpha, model);
            if e.kind == EnemyKind::Gunner {
                out.push(ModelDraw {
                    model: self.models.blaster,
                    transform: enemy::blaster(transform, &pose, clips),
                    pose: None,
                    tint: Color::WHITE,
                });
            }
            out.push(ModelDraw {
                model: handle,
                transform,
                pose: Some(pose),
                tint: Color::WHITE,
            });
        }
        out
    }

    /// Оружие в руках живого игрока с отдачей.
    fn viewmodel(&self, camera: &Camera3D) -> Option<Viewmodel> {
        let l = &self.level;
        l.player.alive().then(|| Viewmodel {
            fov_y: GUN_FOV,
            near: GUN_NEAR,
            models: vec![ModelDraw {
                model: l.arsenal.weapon().model,
                transform: Mat4::from_scale_rotation_translation(
                    Vec3::splat(GUN_SCALE),
                    view(camera),
                    camera.pos + view(camera) * (GUN_AT + l.arsenal.kick()),
                ),
                pose: None,
                tint: Color::WHITE,
            }],
        })
    }

    fn status(&self) -> String {
        let l = &self.level;
        let key = if l.key { "\nKEY" } else { "" };
        format!("HP {}\nAMMO {}{key}", l.player.hp, l.arsenal.weapon().ammo)
    }

    /// Счёт в левом верхнем углу, прицел, победа или смерть по центру.
    fn draw_hud(&self, gfx: &mut Gfx, assets: &Assets) {
        let half = VIEW / 2.0;
        for size in [CROSS, vec2(CROSS.y, CROSS.x)] {
            gfx.draw_sprite(Sprite {
                size,
                layer: HUD_LAYER,
                ..Sprite::default()
            });
        }
        let mut text = |s: &str, pos| {
            gfx.draw_text(assets, self.font, s, pos, 1.0, Color::WHITE, HUD_LAYER);
        };
        text(
            &self.status(),
            vec2(-half.x + HUD_MARGIN, half.y - HUD_MARGIN - HUD_PX),
        );
        let l = &self.level;
        let banner = if l.won {
            WIN
        } else if l.player.alive() {
            return;
        } else {
            DEAD
        };
        let font = assets.font(self.font);
        for (s, y) in [(banner, HUD_PX), (RESTART, -HUD_PX)] {
            text(s, vec2(-text_width(font, s) / 2.0, y));
        }
    }

    /// Команды звуков последнего тика от глаза игрока.
    fn sound_commands(&self) -> Vec<SoundCmd> {
        sound::commands(
            &self.level.sounds,
            &self.level.player.camera(1.0, Vec2::ZERO),
        )
    }

    fn update_sound(&self, ctx: &mut Context) {
        for cmd in self.sound_commands() {
            let sound = ctx.assets.sound(self.sounds.get(cmd.sfx));
            ctx.audio.play_stereo(sound, cmd.gains);
        }
    }

    /// Тик до победы; мёртвый игрок стоит, а мир идёт.
    fn play(&mut self, ctx: &Context) {
        let dt = ctx.time.step.as_secs_f32();
        let l = &mut self.level;
        let enemy_hp: Vec<i32> = l.enemies.iter().map(|e| e.hp).collect();
        l.player.update(&ctx.input, dt, &l.obstacles);
        l.arsenal.update(&ctx.input, dt);
        l.flash = (l.flash - dt).max(0.0);
        if l.player.alive() {
            // Клик захвата курсора — не выстрел.
            if ctx.grabbed && ctx.input.mouse_down(MouseButton::Left) {
                l.fire();
            }
            l.pick_up();
        }
        // После аптечки: иначе лечение и урон одного тика гасили бы друг друга.
        let player_hp = l.player.hp;
        l.doors(dt, self.open_secs);
        for e in &mut l.enemies {
            if let Some(bolt) = e.update(dt, &mut l.player, &l.obstacles) {
                l.projectiles.push(bolt);
                l.sounds.push((Sfx::Bolt, e.pos));
            }
        }
        l.fly(dt);
        let enemies = l.enemies.iter().zip(enemy_hp);
        l.sounds.extend(
            enemies
                .filter_map(|(e, hp)| sound::wound(hp, e.hp, e.pos))
                .chain(sound::wound(player_hp, l.player.hp, l.player.pos)),
        );
        l.won = l.player.alive() && l.player.pos.floor().as_ivec2() == self.map.exit;
        if l.won {
            // `play` больше не позовётся, и `draw` качал бы тела между `prev` и `pos` каждый кадр.
            l.player.prev = l.player.pos;
            l.flash = 0.0;
            for e in &mut l.enemies {
                e.prev = e.pos;
            }
            for p in &mut l.projectiles {
                p.prev = p.pos;
            }
        }
    }
}

impl Game for Shooter {
    fn init(ctx: &mut Context) -> Self {
        let text = fs::read_to_string(LEVEL).unwrap_or_else(|e| panic!("{LEVEL}: {e}"));
        let map = Map::parse(&text).unwrap_or_else(|e| panic!("{LEVEL}: {e}"));
        Self::new(map, &mut ctx.assets)
    }

    fn update(&mut self, ctx: &mut Context) {
        // Клик, пришедший до потери фокуса, захватил бы курсор в фоновом окне.
        if ctx.input.mouse_just_pressed(MouseButton::Left) && ctx.focused {
            ctx.cursor_grab = true;
        }
        if ctx.input.just_pressed(KeyCode::Escape) {
            ctx.cursor_grab = false;
        }
        self.level.sounds.clear();
        let l = &self.level;
        if (l.won || !l.player.alive()) && ctx.input.just_pressed(KeyCode::Enter) {
            self.level = Level::new(&self.map, &self.models);
        } else if !l.won {
            self.play(ctx);
        }
        self.update_sound(ctx);
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        // После выхода `update` не крутит игрока, и непотреблённый сдвиг дёргал бы камеру до тика.
        let delta = if self.level.won {
            Vec2::ZERO
        } else {
            ctx.input.mouse_delta()
        };
        let camera = self.level.player.camera(alpha, delta);
        let scene = self.scene(camera, alpha, ctx.time.step.as_secs_f32(), &ctx.assets);
        ctx.gfx.clear_color = scene.fog.color;
        ctx.gfx.scene = Some(scene);
        ctx.gfx.camera = Camera2D {
            center: Vec2::ZERO,
            size: VIEW,
        };
        self.draw_hud(&mut ctx.gfx, &ctx.assets);
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Shooter>(Config {
        title: "shooter".into(),
        size: (1280, 720),
        tick_hz: TICK_HZ,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: false,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use engine::{
        ElementState::{Pressed, Released},
        FixedClock, Texture, ivec2, text,
    };

    use super::*;
    use crate::bot::{Bot, press};
    use crate::enemy::{State, stats};
    use crate::player::{EYE, SENSITIVITY, SPEED};

    fn all(m: &Models) -> [Handle<Model>; 15] {
        [
            m.tree,
            m.tree_high,
            m.rock,
            m.fence,
            m.grass,
            m.brawler,
            m.gunner,
            m.runner,
            m.blaster,
            m.launcher,
            m.bullet,
            m.clip,
            m.crate_,
            m.key,
            m.gate,
        ]
    }

    fn textures(model: &Model) -> Vec<Handle<Texture>> {
        model
            .nodes
            .iter()
            .flat_map(|n| n.primitives.iter().map(|p| p.texture))
            .collect()
    }

    #[test]
    fn every_model_loads_and_characters_have_clips() {
        let mut ctx = Context::default();
        let g = Shooter::init(&mut ctx);
        let m = &g.models;
        let handles = all(m);
        for (i, h) in handles.iter().enumerate() {
            assert!(!ctx.assets.model(*h).nodes.is_empty(), "модель {i}");
            assert!(!handles[..i].contains(h), "модель {i} — повтор");
        }
        for h in [m.brawler, m.gunner, m.runner] {
            let model = ctx.assets.model(h);
            for clip in CLIPS {
                assert!(model.animations.iter().any(|a| a.name == clip), "{clip}");
            }
            assert!(model.nodes.iter().any(|n| n.name == "arm-right"));
        }
    }

    #[test]
    fn sets_keep_their_own_textures() {
        let mut ctx = Context::default();
        let m = Models::load(&mut ctx.assets);
        let tex = |h| textures(ctx.assets.model(h));
        let sets = [m.tree, m.blaster, m.brawler, m.gunner, m.key];
        let firsts: Vec<_> = sets.iter().map(|&h| tex(h)[0]).collect();
        for (i, t) in firsts.iter().enumerate() {
            assert_ne!(*t, Handle::WHITE, "набор {i}");
            assert!(!firsts[..i].contains(t), "набор {i} делит текстуру");
        }
        for (a, b) in [(m.tree, m.rock), (m.blaster, m.crate_), (m.key, m.gate)] {
            assert_eq!(tex(a)[0], tex(b)[0]);
        }
    }

    #[test]
    fn obstacles_fit_cell_and_character_stands_on_floor() {
        let mut ctx = Context::default();
        let m = Models::load(&mut ctx.assets);
        let placed = |h, scale| {
            let model = ctx.assets.model(h);
            model.bounds().unwrap().transformed(standing(model, scale))
        };
        for h in [m.tree, m.tree_high, m.rock] {
            let b = placed(h, FOREST_SCALE);
            assert!(b.min.y.abs() < 1e-5, "{b:?}");
            assert!(b.max.y >= CELL * 0.8, "{b:?}");
            for v in [b.min, b.max] {
                assert!(v.x.abs() <= CELL / 2.0 && v.z.abs() <= CELL / 2.0, "{b:?}");
            }
        }
        for h in [m.brawler, m.gunner, m.runner] {
            let b = placed(h, CHARACTER_SCALE);
            assert!(b.min.y.abs() < 1e-5, "{b:?}");
            assert!((1.7..1.9).contains(&b.max.y), "{b:?}");
            assert!(b.max.x - b.min.x < CELL, "{b:?}");
        }
    }

    #[test]
    fn init_reads_level_from_disk() {
        let mut ctx = Context::default();
        let g = Shooter::init(&mut ctx);
        assert_eq!(g.map, Map::parse(LEVEL01).unwrap());
    }

    fn shooter(text: &str) -> (Shooter, Context) {
        let mut ctx = Context::new(FixedClock::new(TICK_HZ, Duration::from_millis(250)).step());
        let g = Shooter::new(Map::parse(text).unwrap(), &mut ctx.assets);
        (g, ctx)
    }

    fn camera(ctx: &Context) -> Camera3D {
        ctx.gfx.scene.as_ref().unwrap().camera
    }

    const ROOM: &str = "#####\n#P.X#\n#####\n";

    #[test]
    fn player_starts_at_cell_centre_facing_open_neighbour() {
        let (g, _) = shooter(ROOM);
        assert_eq!(g.level.player.pos, vec2(1.5, 1.5));
        assert_eq!(g.level.player.yaw, Dir::E.yaw());
    }

    #[test]
    fn update_walks_player_by_tick_step() {
        let (mut g, mut ctx) = shooter(ROOM);
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        g.update(&mut ctx);
        assert_eq!(g.level.player.pos, vec2(1.5 + SPEED / TICK_HZ as f32, 1.5));
    }

    #[test]
    fn closed_door_holds_player() {
        let (mut g, mut ctx) = shooter("######\n#PD.X#\n#k...#\n######\n");
        g.level.player.yaw = Dir::E.yaw();
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
        for _ in 0..120 {
            g.update(&mut ctx);
        }
        assert!(
            (g.level.player.pos.x - (2.0 - RADIUS)).abs() < 1e-5,
            "{:?}",
            g.level.player.pos
        );
    }

    #[test]
    fn update_consumes_mouse_and_draw_adds_rest() {
        let (mut g, mut ctx) = shooter(ROOM);
        let yaw = g.level.player.yaw;
        ctx.input.on_mouse_motion(vec2(100.0, -40.0));
        g.update(&mut ctx);
        ctx.input.end_tick();
        assert_eq!(g.level.player.yaw, yaw - 100.0 * SENSITIVITY);
        g.draw(&mut ctx, 1.0);
        assert_eq!(camera(&ctx).yaw, g.level.player.yaw);
        assert_eq!(camera(&ctx).pitch, 40.0 * SENSITIVITY);

        let before = g.level.player;
        ctx.input.on_mouse_motion(vec2(50.0, 0.0));
        g.draw(&mut ctx, 1.0);
        assert_eq!(camera(&ctx).yaw, before.yaw - 50.0 * SENSITIVITY);
        assert_eq!(g.level.player, before);
        assert_eq!(camera(&ctx).pos, vec3(1.5 * CELL, EYE, 1.5 * CELL));
    }

    #[test]
    fn click_grabs_cursor_only_when_focused() {
        let (mut g, mut ctx) = shooter(ROOM);
        ctx.input.on_button(MouseButton::Left, Pressed);
        g.update(&mut ctx);
        assert!(!ctx.cursor_grab, "окно без фокуса");
        ctx.input.on_button(MouseButton::Left, Released);
        ctx.input.end_tick();

        ctx.focused = true;
        ctx.input.on_button(MouseButton::Right, Pressed);
        g.update(&mut ctx);
        assert!(!ctx.cursor_grab, "не левая кнопка");
        ctx.input.end_tick();

        ctx.input.on_button(MouseButton::Left, Pressed);
        g.update(&mut ctx);
        assert!(ctx.cursor_grab);
        ctx.input.end_tick();
        g.update(&mut ctx);
        assert!(ctx.cursor_grab, "без Esc захват держится");
        ctx.input.on_key(KeyCode::Escape, Pressed, false);
        g.update(&mut ctx);
        assert!(!ctx.cursor_grab);
    }

    fn tick(g: &mut Shooter, ctx: &mut Context) {
        g.update(ctx);
        ctx.input.end_tick();
    }

    fn until(g: &mut Shooter, ctx: &mut Context, done: impl Fn(&Shooter) -> bool) {
        for _ in 0..TICK_HZ * 10 {
            if done(g) {
                return;
            }
            tick(g, ctx);
        }
        panic!("за 10 с не дождались");
    }

    fn walk_east(g: &mut Shooter, ctx: &mut Context) {
        g.level.player.yaw = Dir::E.yaw();
        ctx.input.on_key(KeyCode::KeyW, Pressed, false);
    }

    #[test]
    fn key_is_taken_once_by_touch() {
        let (mut g, mut ctx) = shooter("#######\n#P.k.X#\n#######\n");
        g.level.player.pos = vec2(3.5 - PICKUP, 1.5);
        tick(&mut g, &mut ctx);
        assert!(!g.level.key, "ровно на PICKUP не берётся");
        g.level.player.pos = vec2(1.5, 1.5);
        walk_east(&mut g, &mut ctx);
        until(&mut g, &mut ctx, |g| g.level.key);
        assert_eq!(g.level.keys, []);
        let d = g.level.player.pos.distance(vec2(3.5, 1.5));
        assert!(d < PICKUP && d >= PICKUP - SPEED / TICK_HZ as f32, "{d}");
        until(&mut g, &mut ctx, |g| g.level.won);
        assert!(g.level.key);
    }

    #[test]
    fn key_opens_door_and_door_holds_until_animation_ends() {
        let (mut g, mut ctx) = shooter("#######\n#kPD.X#\n#######\n");
        let door = ivec2(3, 1);
        walk_east(&mut g, &mut ctx);
        for _ in 0..TICK_HZ {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level.doors[0].opened, None, "без ключа");
        assert!((g.level.player.pos.x - (3.0 - RADIUS)).abs() < 1e-5);

        g.level.player.yaw = Dir::W.yaw();
        until(&mut g, &mut ctx, |g| g.level.key);
        assert_eq!(g.level.doors[0].opened, None, "ключ издалека не открывает");
        g.level.player.yaw = Dir::E.yaw();
        until(&mut g, &mut ctx, |g| g.level.doors[0].opened.is_some());
        assert!(g.level.player.pos.x <= 3.0 - RADIUS);
        let dt = 1.0 / TICK_HZ as f32;
        while g.level.obstacles.solid(door) {
            let t = g.level.doors[0].opened.unwrap();
            assert!(t < g.open_secs, "{t}");
            assert!(g.level.player.pos.x <= 3.0 - RADIUS + 1e-5);
            tick(&mut g, &mut ctx);
        }
        let t = g.level.doors[0].opened.unwrap();
        assert!(t >= g.open_secs && t - dt < g.open_secs, "{t}");
        until(&mut g, &mut ctx, |g| g.level.won);
    }

    #[test]
    fn medkit_heals_up_to_max_and_waits_at_full_health() {
        let (mut g, mut ctx) = shooter("#########\n#P+.+.+X#\n#########\n");
        walk_east(&mut g, &mut ctx);
        until(&mut g, &mut ctx, |g| g.level.player.pos.x > 3.0);
        assert_eq!(g.level.medkits.len(), 3, "при полном здоровье не берётся");
        g.level.player.hp = MAX_HP - 10;
        until(&mut g, &mut ctx, |g| g.level.medkits.len() == 2);
        assert_eq!(g.level.player.hp, MAX_HP);
        assert_eq!(g.level.medkits, [ivec2(2, 1), ivec2(6, 1)]);
        g.level.player.hp = 50;
        until(&mut g, &mut ctx, |g| g.level.medkits.len() == 1);
        assert_eq!(g.level.player.hp, 50 + MEDKIT);
    }

    #[test]
    fn dead_player_takes_nothing() {
        let (mut g, mut ctx) = shooter("#######\n#P+akX#\n#######\n");
        for cell in [2.5, 3.5, 4.5] {
            g.level.player.pos = vec2(cell, 1.5);
            g.level.player.hp = 0;
            tick(&mut g, &mut ctx);
        }
        assert_eq!(
            g.level,
            Level {
                player: g.level.player,
                ..Level::new(&g.map, &g.models)
            }
        );
    }

    #[test]
    fn ammo_pack_adds_to_both_weapons() {
        let (mut g, mut ctx) = shooter("######\n#Pa.X#\n######\n");
        let before = g.level.arsenal.weapons.map(|w| w.ammo);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level.ammo, [ivec2(2, 1)], "издалека не берётся");
        walk_east(&mut g, &mut ctx);
        until(&mut g, &mut ctx, |g| g.level.ammo.is_empty());
        let after = g.level.arsenal.weapons.map(|w| w.ammo);
        assert_eq!(after, [before[0] + AMMO_PACK[0], before[1] + AMMO_PACK[1]]);
    }

    #[test]
    fn exit_wins_and_enter_restarts_level() {
        let (mut g, mut ctx) = shooter("######\n#P..X#\n#..f.#\n######\n");
        walk_east(&mut g, &mut ctx);
        ctx.input.on_key(KeyCode::Enter, Pressed, false);
        tick(&mut g, &mut ctx);
        assert_ne!(
            g.level.player.pos,
            vec2(1.5, 1.5),
            "Enter до победы не рестартит"
        );
        ctx.input.on_key(KeyCode::Enter, Released, false);

        until(&mut g, &mut ctx, |g| g.level.won);
        assert_eq!(g.level.player.pos.floor().as_ivec2(), g.map.exit);
        assert_eq!(g.level.player.prev, g.level.player.pos);
        assert!(g.level.enemies.iter().all(|e| e.prev == e.pos));
        let won = g.level.clone();
        for _ in 0..10 {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level, won, "после победы мир стоит");

        ctx.input.on_key(KeyCode::Enter, Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level, Level::new(&g.map, &g.models));
    }

    #[test]
    fn dead_player_on_exit_does_not_win() {
        let (mut g, mut ctx) = shooter(ROOM);
        g.level.player.pos = vec2(3.5, 1.5);
        g.level.player.hp = 0;
        tick(&mut g, &mut ctx);
        assert!(!g.level.won);
    }

    #[test]
    fn death_waits_for_enter_and_restarts_full_level() {
        let (mut g, mut ctx) = shooter("########\n#Pm+a.X#\n########\n");
        ctx.grabbed = true;
        ctx.input.on_button(MouseButton::Left, Pressed);
        for _ in 0..TICK_HZ / 2 {
            tick(&mut g, &mut ctx);
        }
        ctx.input.on_button(MouseButton::Left, Released);
        assert!(g.level.enemies[0].hp < stats(EnemyKind::Brawler).hp);
        ctx.input.on_key(KeyCode::Enter, Pressed, false);
        tick(&mut g, &mut ctx);
        ctx.input.on_key(KeyCode::Enter, Released, false);
        assert!(g.level.player.hp < MAX_HP, "живого Enter не рестартит");

        until(&mut g, &mut ctx, |g| !g.level.player.alive());
        let ammo = g.level.arsenal.weapons[0].ammo;
        ctx.input.on_button(MouseButton::Left, Pressed);
        for _ in 0..TICK_HZ {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(g.level.arsenal.weapons[0].ammo, ammo, "мёртвый не стреляет");
        assert!(!g.level.player.alive() && !g.level.won);

        ctx.input.on_key(KeyCode::Enter, Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level, Level::new(&g.map, &g.models));
    }

    #[test]
    fn hitscan_needs_grab_and_skips_dead_enemy() {
        let (mut g, mut ctx) = shooter("########\n#P.m.m.#\n#X.....#\n########\n");
        g.level.enemies[0].hurt(1000);
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        assert_eq!(
            g.level.arsenal.weapons[0].ammo, 40,
            "клик захвата — не выстрел"
        );
        ctx.grabbed = true;
        tick(&mut g, &mut ctx);
        let w = g.level.arsenal.weapons[0];
        assert_eq!(w.ammo, 39);
        assert_eq!(
            g.level.enemies[1].hp,
            stats(EnemyKind::Brawler).hp - w.damage
        );
        assert_eq!(g.level.enemies[1].state, State::Chase);
    }

    #[test]
    fn launcher_projectile_flies_past_dead_enemy_into_live_one() {
        let (mut g, mut ctx) = shooter("########\n#P.m.m.#\n#X.....#\n########\n");
        g.level.enemies[0].hurt(1000);
        ctx.grabbed = true;
        ctx.input.on_key(KeyCode::Digit2, Pressed, false);
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        ctx.input.on_button(MouseButton::Left, Released);
        assert_eq!(g.level.projectiles.len(), 1);
        until(&mut g, &mut ctx, |g| g.level.projectiles.is_empty());
        let w = g.level.arsenal.weapons[1];
        assert_eq!(
            g.level.enemies[1].hp,
            stats(EnemyKind::Brawler).hp - w.damage
        );
    }

    #[test]
    fn gunner_bolt_hurts_player_and_wall_stops_bolt() {
        let (mut g, mut ctx) = shooter("#######\n#P...s#\n#X....#\n#######\n");
        tick(&mut g, &mut ctx);
        assert_eq!(g.level.projectiles.len(), 1);
        until(&mut g, &mut ctx, |g| g.level.player.hp < MAX_HP);
        assert_eq!(g.level.player.hp, MAX_HP - stats(EnemyKind::Gunner).damage);
        assert_eq!(g.level.projectiles, []);
    }

    fn sfx(g: &Shooter) -> Vec<Sfx> {
        g.sound_commands().iter().map(|c| c.sfx).collect()
    }

    #[test]
    fn one_sound_per_shot_of_current_weapon() {
        let (mut g, mut ctx) = shooter("########\n#P....X#\n########\n");
        ctx.grabbed = true;
        ctx.input.on_button(MouseButton::Left, Pressed);
        let mut shots = 0;
        for _ in 0..TICK_HZ {
            tick(&mut g, &mut ctx);
            shots += sfx(&g).iter().filter(|&&s| s == Sfx::Blaster).count();
            assert!(sfx(&g).iter().all(|&s| s == Sfx::Blaster));
        }
        assert_eq!(shots, (40 - g.level.arsenal.weapons[0].ammo) as usize);
        assert!(shots > 1);
        ctx.input.on_key(KeyCode::Digit2, Pressed, false);
        until(&mut g, &mut ctx, |g| !g.level.sounds.is_empty());
        assert_eq!(sfx(&g), [Sfx::Launcher]);
    }

    #[test]
    fn gunner_on_the_right_sounds_louder_on_the_right() {
        let (mut g, mut ctx) = shooter("#####\n#P..#\n#...#\n#..s#\n#X..#\n#####\n");
        g.level.player.yaw = Dir::E.yaw();
        until(&mut g, &mut ctx, |g| !g.level.sounds.is_empty());
        let cmds = g.sound_commands();
        assert_eq!(cmds.iter().map(|c| c.sfx).collect::<Vec<_>>(), [Sfx::Bolt]);
        assert!(cmds[0].gains.right > cmds[0].gains.left, "{cmds:?}");
    }

    #[test]
    fn enemy_hit_then_death_sound_at_enemy() {
        let (mut g, mut ctx) = shooter("#########\n#P.....m#\n#X......#\n#########\n");
        g.level.enemies[0].hp = 30;
        ctx.grabbed = true;
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        let at = |g: &Shooter| g.level.enemies[0].pos;
        assert_eq!(
            g.level.sounds,
            [(Sfx::Blaster, vec2(1.5, 1.5)), (Sfx::Hit, at(&g))]
        );
        tick(&mut g, &mut ctx);
        until(&mut g, &mut ctx, |g| !g.level.sounds.is_empty());
        assert_eq!(
            g.level.sounds,
            [(Sfx::Blaster, vec2(1.5, 1.5)), (Sfx::Death, at(&g))]
        );
    }

    #[test]
    fn player_hit_then_death_sound_at_player() {
        let (mut g, mut ctx) = shooter("#####\n#Pm.#\n#X..#\n#####\n");
        until(&mut g, &mut ctx, |g| !g.level.sounds.is_empty());
        assert_eq!(g.level.sounds, [(Sfx::Hit, g.level.player.pos)]);
        g.level.player.hp = 1;
        tick(&mut g, &mut ctx);
        until(&mut g, &mut ctx, |g| !g.level.sounds.is_empty());
        assert_eq!(g.level.sounds, [(Sfx::Death, g.level.player.pos)]);
    }

    #[test]
    fn pickups_and_door_sound_once_each() {
        let (mut g, mut ctx) = shooter("########\n#Pka+D.#\n#X.....#\n########\n");
        g.level.player.hp = MAX_HP - 1;
        walk_east(&mut g, &mut ctx);
        let mut heard = Vec::new();
        for _ in 0..TICK_HZ * 10 {
            if g.level.doors[0].opened.is_some() {
                break;
            }
            tick(&mut g, &mut ctx);
            heard.extend(g.sound_commands().iter().map(|c| c.sfx));
            if let Some(&(Sfx::Pickup, at)) = g.level.sounds.first() {
                assert_eq!(at, g.level.player.pos);
            }
        }
        assert_eq!(heard, [Sfx::Pickup, Sfx::Pickup, Sfx::Pickup, Sfx::Door]);
        assert_eq!(g.level.sounds, [(Sfx::Door, vec2(5.5, 1.5))]);
    }

    #[test]
    fn projectile_vanishes_at_wall() {
        let (mut g, mut ctx) = shooter(ROOM);
        g.level.player.yaw = Dir::N.yaw();
        ctx.grabbed = true;
        ctx.input.on_key(KeyCode::Digit2, Pressed, false);
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        ctx.input.on_button(MouseButton::Left, Released);
        assert_eq!(g.level.projectiles.len(), 1);
        until(&mut g, &mut ctx, |g| g.level.projectiles.is_empty());
    }

    fn scene(ctx: &Context) -> &Scene {
        ctx.gfx.scene.as_ref().unwrap()
    }

    fn draws(scene: &Scene, model: Handle<Model>) -> Vec<&ModelDraw> {
        scene.models.iter().filter(|d| d.model == model).collect()
    }

    /// Центры клеток, на которых стоят модели `model`, на плане.
    fn placed(scene: &Scene, model: Handle<Model>) -> Vec<Vec2> {
        draws(scene, model).iter().map(|d| plan(d)).collect()
    }

    /// Начало модели на плане, клетки.
    fn plan(d: &ModelDraw) -> Vec2 {
        let p = d.transform.w_axis;
        vec2(p.x, p.z) / CELL
    }

    fn centres(cells: impl IntoIterator<Item = IVec2>) -> Vec<Vec2> {
        cells.into_iter().map(|c| c.as_vec2() + 0.5).collect()
    }

    fn floor(d: &ModelDraw, assets: &Assets) -> f32 {
        let model = assets.model(d.model);
        model.bounds().unwrap().transformed(d.transform).min.y
    }

    #[test]
    fn every_cell_has_its_model_standing_on_ground() {
        let (mut g, mut ctx) = shooter(LEVEL01);
        g.draw(&mut ctx, 0.0);
        let s = scene(&ctx);
        let m = &g.models;
        let map = &g.map;
        let cells = |tile| map.cells().filter(move |&c| map.tile(c) == Some(tile));
        assert_eq!(placed(s, m.grass), centres(cells(Tile::Grass)));
        assert_eq!(placed(s, m.rock), centres(cells(Tile::Rock)));
        assert_eq!(placed(s, m.fence), centres(cells(Tile::Fence)));
        let thick = |c: &IVec2| {
            (Dir::ALL.iter()).all(|d| g.map.tile(c + d.offset()).is_some_and(Tile::solid))
        };
        let (high, low): (Vec<_>, Vec<_>) = cells(Tile::Tree).partition(thick);
        assert!(!high.is_empty() && !low.is_empty());
        assert_eq!(placed(s, m.tree_high), centres(high));
        assert_eq!(placed(s, m.tree), centres(low));
        assert_eq!(g.statics.len(), g.map.cells().count());
        for d in &g.statics {
            assert!(floor(d, &ctx.assets).abs() < 1e-5, "{:?}", d.transform);
        }
        let along = |cell| {
            let d = g.statics.iter().find(|d| plan(d) == cell);
            d.unwrap().transform.x_axis.truncate().normalize()
        };
        assert!(
            along(vec2(5.5, 0.5)).abs_diff_eq(Vec3::X, 1e-6),
            "верхний край"
        );
        assert!(along(vec2(0.5, 0.5)).abs_diff_eq(Vec3::X, 1e-6), "угол");
        assert!(
            along(vec2(0.5, 5.5)).abs_diff_eq(Vec3::NEG_Z, 1e-6),
            "левый край"
        );
    }

    #[test]
    fn ground_covers_map_just_below_grass() {
        let (mut g, mut ctx) = shooter(LEVEL01);
        g.draw(&mut ctx, 0.0);
        let s = scene(&ctx);
        assert_eq!(s.meshes.len(), 1);
        let ground = s.meshes[0];
        assert_eq!((ground.mesh, ground.texture), (g.ground, Handle::WHITE));
        assert_eq!(ground.color, GROUND);
        let geometry = ctx.assets.mesh_geometry(g.ground);
        let corners: Vec<_> = (geometry.positions.iter())
            .map(|&p| ground.transform.transform_point3(p))
            .collect();
        let w = g.map.size.as_vec2() * CELL;
        let y = -GROUND_DROP;
        let want = [
            vec3(0.0, y, 0.0),
            vec3(0.0, y, w.y),
            vec3(w.x, y, w.y),
            vec3(w.x, y, 0.0),
        ];
        assert_eq!(corners, want);
        const { assert!(GROUND_DROP > 0.0) };
    }

    #[test]
    fn quad_faces_up() {
        let (vertices, indices) = quad();
        for t in indices.chunks(3) {
            let p = |i: u32| Vec3::from(vertices[i as usize].pos);
            let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            assert!(n.normalize().abs_diff_eq(Vec3::Y, 1e-6), "{t:?}");
        }
        assert!(vertices.iter().all(|v| Vec3::from(v.normal) == Vec3::Y));
    }

    #[test]
    fn items_and_door_stand_on_their_cells() {
        let (mut g, ctx) = shooter("######\n#PD.X#\n#k+a.#\n######\n");
        let s = g.scene(Camera3D::default(), 0.0, 0.0, &ctx.assets);
        let m = &g.models;
        assert_eq!(placed(&s, m.key), [vec2(1.5, 2.5)]);
        assert_eq!(placed(&s, m.crate_), [vec2(2.5, 2.5)]);
        assert_eq!(placed(&s, m.clip), [vec2(3.5, 2.5)]);
        assert_eq!(placed(&s, m.gate), [vec2(2.5, 1.5)]);
        for model in [m.key, m.crate_, m.clip, m.gate] {
            assert!(floor(draws(&s, model)[0], &ctx.assets).abs() < 1e-5);
        }
        let gate = draws(&s, m.gate)[0];
        assert_eq!(gate.pose, None);
        let b = (ctx.assets.model(m.gate).bounds().unwrap()).transformed(gate.transform);
        assert!(
            (b.max.z - b.min.z - CELL).abs() < 1e-5,
            "коридор вдоль X: {b:?}"
        );

        g.level.doors[0].opened = Some(0.5);
        (g.level.keys, g.level.medkits, g.level.ammo) = (vec![], vec![], vec![]);
        let s = g.scene(Camera3D::default(), 0.0, 0.0, &ctx.assets);
        for model in [m.key, m.crate_, m.clip] {
            assert_eq!(draws(&s, model).len(), 0, "взятые не рисуются");
        }
        let gate = ctx.assets.model(m.gate);
        let mut want = Pose::default();
        gate.pose(g.open, 0.5, false, &mut want);
        assert_ne!(want.globals, gate.rest.globals);
        assert_eq!(draws(&s, m.gate)[0].pose.as_ref(), Some(&want));
    }

    #[test]
    fn door_beside_solid_cell_spans_along_x() {
        for level in [LEVEL01, "#####\n#PD##\n#k.X#\n#####\n"] {
            let (g, ctx) = shooter(level);
            let s = g.scene(Camera3D::default(), 0.0, 0.0, &ctx.assets);
            let gate = draws(&s, g.models.gate)[0];
            let b = (ctx.assets.model(g.models.gate).bounds().unwrap()).transformed(gate.transform);
            assert!((b.max.x - b.min.x - CELL).abs() < 1e-5, "{b:?}");
        }
    }

    #[test]
    fn text_width_sums_advances() {
        let mut assets = Assets::default();
        let font = assets.load_font(FONT_PATH, HUD_PX);
        let font = assets.font(font);
        let one = text_width(font, "0");
        assert_eq!(one, font.glyph('0').unwrap().advance);
        assert!(one > 0.0);
        assert_eq!(text_width(font, "0\u{e9}0"), 3.0 * one);
    }

    #[test]
    fn sun_shines_down_and_gun_sits_low_right_in_narrower_view() {
        const {
            assert!(SUN.dir.y < 0.0);
            assert!(GUN_AT.x > 0.0 && GUN_AT.y < 0.0 && GUN_AT.z < 0.0);
        };
        assert!(
            (0.5..Camera3D::default().fov_y).contains(&GUN_FOV),
            "{GUN_FOV}"
        );
    }

    #[test]
    fn enemies_drawn_between_ticks_in_clip_pose() {
        let (mut g, mut ctx) = shooter("########\n#P....s#\n#X..f..#\n########\n");
        for _ in 0..10 {
            tick(&mut g, &mut ctx);
        }
        let dt = 1.0 / TICK_HZ as f32;
        let check = |g: &Shooter, ctx: &Context, ahead: f32| {
            let s = scene(ctx);
            let mut blasters = 0;
            for e in &g.level.enemies {
                let handle = g.character(e.kind);
                let model = ctx.assets.model(handle);
                let clips = Clips::new(model);
                let mut want = Pose::default();
                e.pose(model, clips, ahead, &mut want);
                let d = draws(s, handle)[0];
                assert_eq!(d.transform, e.transform(0.25, model));
                assert_eq!(d.pose.as_ref(), Some(&want), "{:?}", e.kind);
                if e.kind == EnemyKind::Gunner {
                    let b = draws(s, g.models.blaster)[0];
                    assert_eq!(b.transform, enemy::blaster(d.transform, &want, clips));
                    blasters += 1;
                }
            }
            assert_eq!(draws(s, g.models.blaster).len(), blasters);
        };
        let runner = g.level.enemies[1].clone();
        assert_eq!(runner.state, State::Chase);
        assert_ne!(runner.prev, runner.pos);
        g.draw(&mut ctx, 0.25);
        check(&g, &ctx, 0.25 * dt);
        let model = ctx.assets.model(g.models.runner);
        let mut at_tick = Pose::default();
        runner.pose(
            model,
            g.clips[EnemyKind::Runner as usize],
            0.0,
            &mut at_tick,
        );
        assert_ne!(draws(scene(&ctx), g.models.runner)[0].pose, Some(at_tick));

        g.level.won = true;
        g.draw(&mut ctx, 0.25);
        check(&g, &ctx, 0.0);
    }

    #[test]
    fn projectile_drawn_between_ticks_along_flight() {
        let (mut g, mut ctx) = shooter("########\n#P.....#\n#X.....#\n########\n");
        ctx.grabbed = true;
        ctx.input.on_key(KeyCode::Digit2, Pressed, false);
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        tick(&mut g, &mut ctx);
        let p = g.level.projectiles[0];
        assert_ne!(p.prev, p.pos);
        g.draw(&mut ctx, 0.5);
        let d = draws(scene(&ctx), g.models.bullet)[0];
        let at = world(p.prev.lerp(p.pos, 0.5), SHOT_HEIGHT);
        assert!(d.transform.w_axis.truncate().abs_diff_eq(at, 1e-5));
        let nose = d.transform.transform_vector3(Vec3::Y);
        assert!(nose.abs_diff_eq(Vec3::X * BULLET_SCALE, 1e-5), "{nose:?}");
    }

    #[test]
    fn viewmodel_holds_current_weapon_at_eye_with_recoil() {
        let (mut g, mut ctx) = shooter(ROOM);
        g.level.player.pitch = 0.3;
        let gun = |ctx: &Context| scene(ctx).viewmodel.clone().unwrap();
        g.draw(&mut ctx, 1.0);
        let cam = camera(&ctx);
        let vm = gun(&ctx);
        assert_eq!((vm.fov_y, vm.near), (GUN_FOV, GUN_NEAR));
        assert!(GUN_FOV < cam.fov_y);
        assert_eq!(vm.models.len(), 1);
        let d = &vm.models[0];
        assert_eq!(d.model, g.models.blaster);
        let barrel = d.transform.transform_vector3(Vec3::NEG_Z);
        assert!(
            barrel.abs_diff_eq(cam.forward() * GUN_SCALE, 1e-6),
            "{barrel:?}"
        );
        let rest = d.transform.w_axis.truncate();
        let local = |p: Vec3| view(&cam).inverse() * (p - cam.pos);
        assert!(local(rest).abs_diff_eq(GUN_AT, 1e-5), "{:?}", local(rest));

        ctx.grabbed = true;
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        ctx.input.on_button(MouseButton::Left, Released);
        g.draw(&mut ctx, 1.0);
        let kicked = local(gun(&ctx).models[0].transform.w_axis.truncate());
        let kick = g.level.arsenal.kick();
        assert!(kick.z > 0.0);
        assert!(kicked.abs_diff_eq(GUN_AT + kick, 1e-5), "{kicked:?}");

        ctx.input.on_key(KeyCode::Digit2, Pressed, false);
        tick(&mut g, &mut ctx);
        g.draw(&mut ctx, 1.0);
        assert_eq!(gun(&ctx).models[0].model, g.models.launcher);

        g.level.player.hp = 0;
        g.draw(&mut ctx, 1.0);
        assert_eq!(scene(&ctx).viewmodel, None, "у мёртвого оружия нет");
    }

    fn muzzle(ctx: &Context) -> Option<PointLight> {
        scene(ctx)
            .lights
            .iter()
            .copied()
            .find(|l| l.color == MUZZLE)
    }

    #[test]
    fn flash_lasts_by_timer_across_ticks() {
        let (mut g, mut ctx) = shooter(ROOM);
        g.draw(&mut ctx, 0.0);
        let exit = PointLight {
            pos: vec3(3.5 * CELL, 1.0, 1.5 * CELL),
            color: EXIT_LIGHT,
            range: EXIT_RANGE,
        };
        assert_eq!(scene(&ctx).lights, [exit]);
        ctx.grabbed = true;
        ctx.input.on_button(MouseButton::Left, Pressed);
        tick(&mut g, &mut ctx);
        ctx.input.on_button(MouseButton::Left, Released);
        let (ticks, dt) = (5, 1.0 / TICK_HZ as f32);
        assert!(
            4.0 * dt < FLASH && FLASH <= 5.0 * dt,
            "кадр на два тика не теряет вспышку"
        );
        for i in 1..ticks {
            tick(&mut g, &mut ctx);
            g.draw(&mut ctx, 0.0);
            let cam = camera(&ctx);
            let want = PointLight {
                pos: cam.pos + cam.forward() * MUZZLE_AHEAD,
                color: MUZZLE,
                range: MUZZLE_RANGE,
            };
            assert_eq!(muzzle(&ctx), Some(want), "тик {i}");
        }
        tick(&mut g, &mut ctx);
        g.draw(&mut ctx, 0.0);
        assert_eq!(muzzle(&ctx), None);
        assert_eq!(scene(&ctx).lights, [exit]);
    }

    #[test]
    fn win_puts_out_flash() {
        let (mut g, mut ctx) = shooter(ROOM);
        ctx.grabbed = true;
        ctx.input.on_button(MouseButton::Left, Pressed);
        g.level.player.pos = vec2(3.5, 1.5);
        tick(&mut g, &mut ctx);
        assert!(g.level.won);
        assert_eq!(g.level.flash, 0.0);
    }

    fn hud(gfx: &Gfx) -> Vec<Sprite> {
        (gfx.sprites.iter())
            .filter(|s| s.layer == HUD_LAYER)
            .copied()
            .collect()
    }

    #[test]
    fn hud_shows_health_ammo_key_crosshair_and_banners() {
        let (mut g, mut ctx) = shooter(ROOM);
        g.level.player.hp = 70;
        g.level.arsenal.current = 1;
        let frame = |g: &mut Shooter, ctx: &mut Context| {
            ctx.gfx.sprites.clear();
            g.draw(ctx, 0.0);
            hud(&ctx.gfx)
        };
        let playing = frame(&mut g, &mut ctx);
        assert_eq!(
            ctx.gfx.camera,
            Camera2D {
                center: Vec2::ZERO,
                size: VIEW
            }
        );
        g.level.key = true;
        let with_key = frame(&mut g, &mut ctx);
        g.level.player.hp = 0;
        let dead = frame(&mut g, &mut ctx);
        g.level.player.hp = 70;
        g.level.won = true;
        let won = frame(&mut g, &mut ctx);

        let font = ctx.assets.font(g.font);
        let text = |s, pos| text::layout(font, s, pos, 1.0, Color::WHITE, HUD_LAYER);
        let cross = |size| Sprite {
            size,
            layer: HUD_LAYER,
            ..Sprite::default()
        };
        let corner = vec2(-628.0, 324.0);
        let screen = |status| {
            [cross(vec2(16.0, 2.0)), cross(vec2(2.0, 16.0))]
                .into_iter()
                .chain(text(status, corner))
                .collect::<Vec<_>>()
        };
        assert_eq!(playing, screen("HP 70\nAMMO 10"));
        assert_eq!(with_key, screen("HP 70\nAMMO 10\nKEY"));
        let centered = |s, y| text(s, vec2(-text_width(font, s) / 2.0, y));
        let banner = |status, s| {
            (screen(status).into_iter())
                .chain(centered(s, HUD_PX))
                .chain(centered(RESTART, -HUD_PX))
                .collect::<Vec<_>>()
        };
        assert_eq!(dead, banner("HP 0\nAMMO 10\nKEY", DEAD));
        assert_eq!(won, banner("HP 70\nAMMO 10\nKEY", WIN));
    }

    #[test]
    fn scene_is_lit_by_sun_and_clears_with_fog() {
        let (mut g, mut ctx) = shooter(ROOM);
        g.draw(&mut ctx, 0.0);
        let s = scene(&ctx);
        assert_eq!((s.ambient, s.fog, s.sun), (AMBIENT, FOG, Some(SUN)));
        assert_eq!(ctx.gfx.clear_color, FOG.color);
    }

    /// Тики бота до победы или смерти, не дольше `secs`; выстрелы каждого оружия.
    fn run_bot(g: &mut Shooter, ctx: &mut Context, secs: u32) -> [u32; 2] {
        let mut bot = Bot::default();
        let mut shots = [0; 2];
        ctx.grabbed = true;
        for _ in 0..TICK_HZ * secs {
            if g.level.won || !g.level.player.alive() {
                break;
            }
            press(
                bot.act(&g.level, g.map.exit),
                g.level.player.yaw,
                &mut ctx.input,
            );
            tick(g, ctx);
            for &(sfx, _) in &g.level.sounds {
                match sfx {
                    Sfx::Blaster => shots[0] += 1,
                    Sfx::Launcher => shots[1] += 1,
                    _ => {}
                }
            }
        }
        shots
    }

    #[test]
    fn bot_clears_level01_with_both_weapons() {
        let (mut g, mut ctx) = shooter(LEVEL01);
        // Без бластера с начала и с тремя снарядами бот обязан менять оружие дважды.
        g.level.arsenal.weapons[0].ammo = 0;
        g.level.arsenal.weapons[1].ammo = 3;
        let start = g.level.arsenal.weapons.map(|w| w.ammo);
        let shots = run_bot(&mut g, &mut ctx, 90);
        let l = &g.level;
        assert!(l.won, "hp {} в {:?}", l.player.hp, l.player.pos);
        assert!(l.enemies.iter().all(|e| e.state == State::Dead));
        assert!(l.key && l.doors.iter().all(|d| d.opened.is_some()));
        let packs = u32::try_from(g.map.ammo.len() - l.ammo.len()).unwrap();
        assert!(packs > 0);
        for (w, weapon) in l.arsenal.weapons.iter().enumerate() {
            assert!(shots[w] > 0, "оружие {w} не стреляло");
            let left = start[w] + packs * AMMO_PACK[w] - shots[w];
            assert_eq!(weapon.ammo, left, "оружие {w}: выстрел — патрон");
        }
    }

    #[test]
    fn idle_player_near_enemies_dies_and_enter_restarts() {
        let (mut g, mut ctx) = shooter("##########\n#P..f.s.X#\n##########\n");
        for _ in 0..TICK_HZ * 20 {
            tick(&mut g, &mut ctx);
        }
        assert!(!g.level.player.alive());
        assert_eq!(g.level.player.pos, vec2(1.5, 1.5));
        ctx.input.on_key(KeyCode::Enter, Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(g.level, Level::new(&g.map, &g.models));
    }

    #[test]
    fn closed_door_stops_sight_path_shot_and_projectile() {
        let (mut g, mut ctx) = shooter("##########\n#P...Dm.X#\n#k...#####\n##########\n");
        let door = 5.0;
        g.level.player.yaw = Dir::E.yaw();
        ctx.grabbed = true;
        ctx.input.on_button(MouseButton::Left, Pressed);
        for key in [KeyCode::Digit1, KeyCode::Digit2] {
            ctx.input.on_key(key, Pressed, false);
            for _ in 0..TICK_HZ {
                tick(&mut g, &mut ctx);
                assert!(g.level.projectiles.iter().all(|p| p.pos.x <= door));
            }
            ctx.input.on_key(key, Released, false);
        }
        let start = Arsenal::new(g.models.blaster, g.models.launcher);
        for (w, s) in g.level.arsenal.weapons.iter().zip(&start.weapons) {
            assert!(w.ammo < s.ammo, "{:?} не стрелял", w.kind);
        }
        let e = &g.level.enemies[0];
        assert_eq!(
            (e.state, e.hp),
            (State::Sleep, stats(EnemyKind::Brawler).hp)
        );

        g.level.enemies[0].hurt(1);
        for _ in 0..TICK_HZ * 5 {
            tick(&mut g, &mut ctx);
            assert!(g.level.enemies[0].pos.x > door + 1.0);
        }
        assert_eq!(g.level.enemies[0].state, State::Chase);
        assert_eq!(g.level.player.hp, MAX_HP);
    }
}
