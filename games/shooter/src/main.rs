use engine::{Aabb3, Assets, Config, Context, Game, Handle, Mat4, Model, Scene, Vec3};

const TICK_HZ: u32 = 60;
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
/// Сторона клетки карты, м.
#[cfg_attr(not(test), expect(dead_code, reason = "карта — в следующих задачах"))]
const CELL: f32 = 2.0;
/// Единица Mini Forest — клетка.
#[cfg_attr(not(test), expect(dead_code, reason = "сцена — в следующих задачах"))]
const FOREST_SCALE: f32 = CELL;
/// Рост Blocky Characters — 2,7 единицы, персонаж — 1,8 м.
#[cfg_attr(not(test), expect(dead_code, reason = "сцена — в следующих задачах"))]
const CHARACTER_SCALE: f32 = 1.8 / 2.7;

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

#[cfg_attr(not(test), expect(dead_code, reason = "сцена — в следующих задачах"))]
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
#[cfg_attr(not(test), expect(dead_code, reason = "сцена — в следующих задачах"))]
fn standing(model: &Model, scale: f32) -> Mat4 {
    let lift = -model.bounds().map_or(0.0, |b: Aabb3| b.min.y) * scale;
    Mat4::from_translation(Vec3::Y * lift) * Mat4::from_scale(Vec3::splat(scale))
}

struct Shooter {
    #[cfg_attr(not(test), expect(dead_code, reason = "сцена — в следующих задачах"))]
    models: Models,
}

impl Game for Shooter {
    fn init(ctx: &mut Context) -> Self {
        Self {
            models: Models::load(&mut ctx.assets),
        }
    }

    fn update(&mut self, _ctx: &mut Context) {}

    fn draw(&mut self, ctx: &mut Context, _alpha: f32) {
        ctx.gfx.scene = Some(Scene::default());
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
    use engine::Texture;

    use super::*;

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
    fn draw_puts_empty_scene() {
        let mut ctx = Context::default();
        let mut g = Shooter::init(&mut ctx);
        g.draw(&mut ctx, 0.5);
        assert_eq!(ctx.gfx.scene, Some(Scene::default()));
    }
}
