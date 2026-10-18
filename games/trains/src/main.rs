#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "путь подключается к карте и сцене в следующих задачах"
    )
)]
mod piece;

use std::collections::BTreeMap;
use std::f32::consts::FRAC_PI_4;

use engine::{
    Assets, Camera3D, Color, Config, Context, DirectionalLight, Game, Handle, Mat4, Model,
    ModelDraw, Scene, Vec3, vec3,
};

const TICK_HZ: u32 = 60;
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
/// Подкаталог набора и имя GLB без расширения.
const MODELS: [&str; 34] = [
    "train/railroad-straight",
    "train/railroad-corner-large",
    "train/railroad-corner-small",
    "train/railroad-curve",
    "train/train-connector",
    "train/train-locomotive-a",
    "train/train-diesel-a",
    "train/train-diesel-box-a",
    "train/train-electric-double-a",
    "train/train-electric-bullet-a",
    "train/train-carriage-box",
    "train/train-carriage-coal",
    "train/train-carriage-container-blue",
    "train/train-carriage-container-red",
    "train/train-carriage-flatbed-wood",
    "train/train-carriage-lumber",
    "train/train-carriage-tank",
    "train/train-carriage-wood",
    "road/road-straight",
    "road/road-corner",
    "road/road-crossing",
    "road/road-end",
    "road/grass",
    "road/pond",
    "road/pool",
    "road/plaza",
    "city/building-a",
    "city/building-b",
    "city/building-c",
    "city/building-d",
    "city/building-e",
    "city/building-f",
    "city/building-skyscraper-a",
    "city/building-skyscraper-b",
];
/// Изометрия: взгляд по диагонали клетки.
const YAW: f32 = FRAC_PI_4;
/// Наклон изометрии, `−atan(1/√2)`.
const PITCH: f32 = -35.264_f32.to_radians();
/// Видимая высота кадра ортокамеры, м.
const VIEW_HEIGHT: f32 = 16.0;
/// Отвод камеры от цели вдоль взгляда: ортографии он не виден, но сцена должна быть перед `near`.
const CAMERA_BACK: f32 = 50.0;
/// Шаг витрины моделей, м.
const SHOWCASE_STEP: f32 = 4.0;
const SHOWCASE_COLUMNS: usize = 6;
const AMBIENT: Color = Color {
    r: 0.35,
    g: 0.35,
    b: 0.4,
    a: 1.0,
};
const SKY: Color = Color {
    r: 0.45,
    g: 0.62,
    b: 0.8,
    a: 1.0,
};

struct Trains {
    models: BTreeMap<&'static str, Handle<Model>>,
}

fn load_models(assets: &mut Assets) -> BTreeMap<&'static str, Handle<Model>> {
    MODELS
        .iter()
        .map(|&name| (name, assets.load_model(format!("{ASSETS}{name}.glb"))))
        .collect()
}

/// Изометрическая ортокамера, смотрящая на `target`.
fn camera(target: Vec3) -> Camera3D {
    let mut camera = Camera3D {
        yaw: YAW,
        pitch: PITCH,
        ortho: Some(VIEW_HEIGHT),
        ..Camera3D::default()
    };
    camera.pos = target - camera.forward() * CAMERA_BACK;
    camera
}

impl Game for Trains {
    fn init(ctx: &mut Context) -> Self {
        Self {
            models: load_models(&mut ctx.assets),
        }
    }

    fn update(&mut self, _ctx: &mut Context) {}

    fn draw(&mut self, ctx: &mut Context, _alpha: f32) {
        let models = MODELS
            .iter()
            .enumerate()
            .map(|(i, name)| ModelDraw {
                model: self.models[name],
                transform: Mat4::from_translation(
                    vec3(
                        (i % SHOWCASE_COLUMNS) as f32,
                        0.0,
                        (i / SHOWCASE_COLUMNS) as f32,
                    ) * SHOWCASE_STEP,
                ),
                pose: None,
                tint: Color::WHITE,
            })
            .collect();
        let rows = MODELS.len().div_ceil(SHOWCASE_COLUMNS);
        let center =
            vec3((SHOWCASE_COLUMNS - 1) as f32, 0.0, (rows - 1) as f32) * SHOWCASE_STEP / 2.0;
        ctx.gfx.clear_color = SKY;
        ctx.gfx.scene = Some(Scene {
            camera: camera(center),
            ambient: AMBIENT,
            sun: Some(DirectionalLight {
                dir: vec3(-0.4, -1.0, -0.6),
                color: Color::WHITE,
            }),
            models,
            ..Scene::default()
        });
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Trains>(Config {
        title: "trains".into(),
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

    fn texture(
        assets: &Assets,
        name: &str,
        models: &BTreeMap<&str, Handle<Model>>,
    ) -> Handle<Texture> {
        assets.model(models[name]).nodes[0].primitives[0].texture
    }

    #[test]
    fn every_model_loads_with_volume() {
        let mut ctx = Context::default();
        let g = Trains::init(&mut ctx);
        assert_eq!(g.models.len(), MODELS.len());
        let mut seen = Vec::new();
        for (name, &h) in &g.models {
            let b = ctx
                .assets
                .model(h)
                .bounds()
                .unwrap_or_else(|| panic!("{name}"));
            assert!(b.max.cmpgt(b.min).all(), "{name}: {b:?}");
            assert!(!seen.contains(&h), "{name} — повтор");
            seen.push(h);
        }
    }

    #[test]
    fn sets_keep_their_own_textures() {
        let mut ctx = Context::default();
        let m = load_models(&mut ctx.assets);
        let tex = |name| texture(&ctx.assets, name, &m);
        let train = tex("train/train-diesel-a");
        let city = tex("city/building-a");
        assert_ne!(train, Handle::WHITE);
        assert_ne!(city, Handle::WHITE);
        assert_ne!(train, city);
        assert_eq!(tex("train/railroad-straight"), train);
        assert_eq!(tex("city/building-skyscraper-a"), city);
        assert_eq!(tex("road/road-straight"), Handle::WHITE);
    }

    #[test]
    fn draw_puts_every_model_under_isometric_ortho_camera() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        g.draw(&mut ctx, 0.0);
        let scene = ctx.gfx.scene.take().unwrap();
        assert_eq!(scene.camera.ortho, Some(VIEW_HEIGHT));
        let f = scene.camera.forward();
        assert!((f.x - f.z).abs() < 1e-5 && f.x < 0.0, "{f}");
        assert!((f.y + 1.0 / 3f32.sqrt()).abs() < 1e-4, "{f}");
        let models: Vec<_> = scene.models.iter().map(|d| d.model).collect();
        assert_eq!(models.len(), MODELS.len());
        assert!(MODELS.iter().all(|n| models.contains(&g.models[n])));
    }
}
