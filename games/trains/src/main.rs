#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "состав подключается к пути и депо в следующих задачах"
    )
)]
mod consist;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "путь подключается к карте и сцене в следующих задачах"
    )
)]
mod piece;
mod switch;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "граф пути подключается к составу и сцене в следующих задачах"
    )
)]
mod track;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "состав подключается к управлению и сцене в следующих задачах"
    )
)]
mod train;

use std::collections::BTreeMap;
use std::f32::consts::FRAC_PI_4;

use engine::{
    Assets, Camera2D, Camera3D, Color, Config, Context, DirectionalLight, Font, FontData, Game,
    Gfx, Handle, Input, KeyCode, Mat4, Model, ModelDraw, MouseButton, Scene, Vec2, Vec3, vec2,
    vec3,
};

use consist::Car;
use track::{LEVEL01, MARGIN, Track};
use train::Train;

const TICK_HZ: u32 = 60;
const DT: f32 = 1.0 / TICK_HZ as f32;
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
const FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Regular.ttf"
);
/// HUD в единицах окна 1280×720; иной аспект — letterbox.
const VIEW: Vec2 = vec2(1280.0, 720.0);
/// При `VIEW` в окне 1280×720 тексель атласа — пиксель экрана.
const HUD_PX: f32 = 24.0;
const HUD_MARGIN: f32 = 12.0;
const HUD_LAYER: i32 = 1;
/// Атлас шрифта — только ASCII, поэтому HUD по-английски.
const BUSY: &str = "SWITCH BUSY";
const GAS: &[KeyCode] = &[KeyCode::KeyW, KeyCode::ArrowUp];
const BACK: &[KeyCode] = &[KeyCode::KeyS, KeyCode::ArrowDown];
/// Состав в депо на старте.
const START: [&str; 3] = [
    "train/train-locomotive-a",
    "train/train-carriage-box",
    "train/train-carriage-coal",
];
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
    track: Track,
    train: Train,
    font: Handle<Font>,
    /// Тиков до конца надписи об отказе стрелки.
    busy: u32,
}

fn load_models(assets: &mut Assets) -> BTreeMap<&'static str, Handle<Model>> {
    MODELS
        .iter()
        .map(|&name| (name, assets.load_model(format!("{ASSETS}{name}.glb"))))
        .collect()
}

/// Модели с тележками из `MODELS`: локомотивы и вагоны.
fn catalog(
    models: &BTreeMap<&'static str, Handle<Model>>,
    assets: &Assets,
) -> BTreeMap<&'static str, Car> {
    (models.iter())
        .filter_map(|(&n, &h)| Some((n, Car::new(h, assets.model(h))?)))
        .collect()
}

fn pressed(input: &Input, keys: &[KeyCode]) -> f32 {
    f32::from(u8::from(keys.iter().any(|&k| input.is_down(k))))
}

fn text_width(font: &FontData, text: &str) -> f32 {
    text.chars()
        .filter_map(|c| font.glyph(c).or_else(|| font.glyph('?')))
        .map(|g| g.advance)
        .sum()
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
        let models = load_models(&mut ctx.assets);
        let cars = catalog(&models, &ctx.assets);
        let all: Vec<_> = cars.values().copied().collect();
        let track = Track::parse(LEVEL01, consist::max_len(&all)).expect("level01");
        let start = START.iter().map(|n| cars[n]).collect();
        let train = Train::new(&track, track.depot[0], MARGIN, start).expect("депо вмещает состав");
        Self {
            models,
            track,
            train,
            font: ctx.assets.load_font(FONT_PATH, HUD_PX),
            busy: 0,
        }
    }

    fn update(&mut self, ctx: &mut Context) {
        let input = &ctx.input;
        let ahead = (input.just_pressed(KeyCode::KeyE))
            .then(|| switch::ahead(&self.track, &self.train))
            .flatten();
        let clicked = (input.mouse_just_pressed(MouseButton::Left))
            .then(|| switch::pick(&self.track, ctx.cursor_ray()?))
            .flatten()
            .filter(|&c| Some(c) != ahead);
        self.busy = self.busy.saturating_sub(1);
        for s in ahead.into_iter().chain(clicked) {
            if !switch::toggle(&mut self.track, &self.train, s) {
                self.busy = TICK_HZ;
            }
        }
        let pedal = pressed(input, GAS) - pressed(input, BACK);
        self.train.drive(pedal, DT);
        self.train.advance(DT, &self.track);
    }

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
        self.draw_hud(&mut ctx.gfx, &ctx.assets);
    }
}

impl Trains {
    /// Отказ стрелки — по центру сверху.
    fn draw_hud(&self, gfx: &mut Gfx, assets: &Assets) {
        gfx.camera = Camera2D {
            center: Vec2::ZERO,
            size: VIEW,
        };
        if self.busy > 0 {
            let x = -text_width(assets.font(self.font), BUSY) / 2.0;
            let pos = vec2(x, VIEW.y / 2.0 - HUD_MARGIN - HUD_PX);
            gfx.draw_text(assets, self.font, BUSY, pos, 1.0, Color::WHITE, HUD_LAYER);
        }
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
    use engine::{ElementState, IVec2, Shown, ShownScene, Sprite, Texture, UVec2, Viewport, text};

    use super::*;
    use crate::piece::Piece;

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

    const A: Vec2 = vec2(8.0, 0.0);
    const D: Vec2 = vec2(12.0, 16.0);

    fn tick(g: &mut Trains, ctx: &mut Context) {
        g.update(ctx);
        ctx.input.end_tick();
    }

    fn switch_at(t: &Track, pos: Vec2) -> usize {
        (t.switches.iter())
            .position(|s| t.ports[s.port].pos == pos)
            .unwrap_or_else(|| panic!("{pos}"))
    }

    /// Кадр показан ортокамерой на `target`, клик в середине экрана.
    fn click(ctx: &mut Context, target: Vec2) {
        let size = UVec2::new(1280, 720);
        ctx.shown = Some(Shown {
            camera: Camera2D {
                center: Vec2::ZERO,
                size: VIEW,
            },
            viewport: Viewport {
                x: 0.0,
                y: 0.0,
                w: VIEW.x,
                h: VIEW.y,
            },
            scene: Some(ShownScene {
                camera: camera(vec3(target.x, 0.0, target.y)),
                size,
            }),
        });
        ctx.input.on_cursor(Some(size.as_vec2() / 2.0));
        ctx.input
            .on_button(MouseButton::Left, ElementState::Released);
        ctx.input
            .on_button(MouseButton::Left, ElementState::Pressed);
    }

    fn hud(g: &mut Trains, ctx: &mut Context) -> Vec<Sprite> {
        ctx.gfx.sprites.clear();
        g.draw(ctx, 0.0);
        (ctx.gfx.sprites.iter())
            .filter(|s| s.layer == HUD_LAYER)
            .copied()
            .collect()
    }

    #[test]
    fn text_width_sums_advances() {
        let mut assets = Assets::default();
        let font = assets.load_font(FONT_PATH, HUD_PX);
        let font = assets.font(font);
        let one = text_width(font, "0");
        assert_eq!(one, font.glyph('0').expect("0").advance);
        assert!(one > 1.0);
        assert_eq!(text_width(font, "0\u{e9}0"), 3.0 * one);
    }

    #[test]
    fn catalog_is_every_locomotive_and_wagon() {
        let mut ctx = Context::default();
        let g = Trains::init(&mut ctx);
        let cars = catalog(&g.models, &ctx.assets);
        let mut want: Vec<_> = (MODELS.iter().copied())
            .filter(|n| n.starts_with("train/train-") && *n != "train/train-connector")
            .collect();
        want.sort_unstable();
        assert_eq!(cars.keys().copied().collect::<Vec<_>>(), want);
    }

    #[test]
    fn starts_in_depot_with_start_consist() {
        let mut ctx = Context::default();
        let g = Trains::init(&mut ctx);
        assert_eq!(g.train.route[0], g.track.depot[0]);
        assert_eq!(g.train.tail, MARGIN);
        let models: Vec<_> = g.train.cars.iter().map(|c| c.model).collect();
        assert_eq!(models, START.map(|n| g.models[n]));
    }

    #[test]
    fn e_throws_switch_ahead_click_throws_switch_under_cursor() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let (a, d) = (switch_at(&g.track, A), switch_at(&g.track, D));
        ctx.input
            .on_key(KeyCode::KeyE, ElementState::Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(
            (g.track.switches[d].active, g.track.switches[a].active),
            (1, 0)
        );
        tick(&mut g, &mut ctx);
        assert_eq!(
            g.track.switches[d].active, 1,
            "держать E — не переводить снова"
        );
        click(&mut ctx, A);
        tick(&mut g, &mut ctx);
        assert_eq!(
            (g.track.switches[d].active, g.track.switches[a].active),
            (1, 1)
        );
        click(&mut ctx, D);
        ctx.input
            .on_key(KeyCode::KeyE, ElementState::Released, false);
        ctx.input
            .on_key(KeyCode::KeyE, ElementState::Pressed, false);
        tick(&mut g, &mut ctx);
        assert_eq!(
            g.track.switches[d].active, 0,
            "клик и E по одной стрелке — один перевод"
        );
        assert_eq!(g.busy, 0);
        assert_eq!(hud(&mut g, &mut ctx), []);
    }

    #[test]
    fn switch_under_train_refuses_and_hud_shows_busy_for_a_second() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let bottom = (g.track.edges.iter())
            .position(|e| e.rail.piece == Piece::Straight && e.rail.cell == IVec2::new(2, 0))
            .expect("прямая до A");
        g.train = Train::new(&g.track, (bottom, true), 2.0, g.train.cars.clone()).expect("состав");
        let a = switch_at(&g.track, A);
        click(&mut ctx, A);
        tick(&mut g, &mut ctx);
        assert_eq!(g.track.switches[a].active, 0);
        let font = ctx.assets.font(g.font);
        assert!(font.atlas.is_some(), "шрифт не загрузился");
        let pos = vec2(-text_width(font, BUSY) / 2.0, 324.0);
        let want: Vec<_> = text::layout(font, BUSY, pos, 1.0, Color::WHITE, HUD_LAYER).collect();
        assert_eq!(hud(&mut g, &mut ctx), want);
        assert_eq!(
            ctx.gfx.camera,
            Camera2D {
                center: Vec2::ZERO,
                size: VIEW
            }
        );
        for _ in 1..TICK_HZ {
            tick(&mut g, &mut ctx);
        }
        assert_eq!(hud(&mut g, &mut ctx), want);
        tick(&mut g, &mut ctx);
        assert_eq!(hud(&mut g, &mut ctx), []);
    }

    #[test]
    fn keys_drive_train_out_of_depot_and_back() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let start = g.train.clone();
        for key in GAS {
            ctx.input.on_key(*key, ElementState::Pressed, false);
            tick(&mut g, &mut ctx);
            let mut want = start.clone();
            want.drive(1.0, 1.0 / 60.0);
            assert_eq!(g.train.speed, want.speed, "{key:?}");
            assert!(g.train.head > start.head, "{key:?}");
            ctx.input.on_key(*key, ElementState::Released, false);
            g.train = start.clone();
        }
        for key in BACK {
            g.train.speed = 1.0;
            ctx.input.on_key(*key, ElementState::Pressed, false);
            tick(&mut g, &mut ctx);
            assert!(g.train.speed < 0.99, "тормоз, а не накат: {key:?}");
            ctx.input.on_key(*key, ElementState::Released, false);
        }
    }
}
