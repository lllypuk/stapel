//! Учебный движок: окно, фиксированный тик, спрайты, текст и меши через wgpu.
#![deny(missing_docs)]

mod app;
/// Текстуры, шрифты и звуки по хэндлам.
pub mod assets;
/// Звуки, их запуск и микшер.
pub mod audio;
/// Ортокамера и вписывание кадра в окно.
pub mod camera;
/// Цель рендера без окна.
pub mod canvas;
mod cli;
/// Фиксированный шаг симуляции.
pub mod clock;
/// Сущности, компоненты и отложенные команды.
pub mod ecs;
/// Контекст кадра, трейт `Game` и шаг кадра.
pub mod frame;
/// Кадр в переданные виды.
pub mod frame_renderer;
/// Адаптер, устройство и очередь.
pub mod gpu;
#[cfg(test)]
mod gpu_test;
/// Текстуры на GPU, общие для конвейеров.
pub mod gpu_textures;
/// Клавиатура и мышь по тикам.
pub mod input;
/// Вершины и меши.
pub mod mesh;
/// GPU-путь мешей.
pub mod mesh_pipeline;
/// Модели glTF: узлы, скины, анимации.
pub mod model;
/// Целочисленные коллизии и движение тел: AABB, тайловая карта, Actor.
pub mod physics;
/// Твёрдые тела в 3D.
pub mod physics3d;
/// Вывод `Canvas` на поверхность.
pub mod present;
/// Поверхность окна и рендер кадра.
pub mod renderer;
/// Прогон сценария без окна.
pub mod runner;
/// 3D-кадр и его раскрытие в инстансы.
pub mod scene;
/// Сценарий runner: TOML со входом по тикам.
pub mod script;
/// Спрайты и их батчинг в инстансы.
pub mod sprite;
/// GPU-путь спрайтов.
pub mod sprite_pipeline;
/// Счётчики FPS, TPS и времени кадра.
pub mod stats;
mod surface;
/// Растеризация шрифтов и раскладка строк в спрайты.
pub mod text;
/// egui поверх кадра: маршрутизация ввода и вывод кадра.
pub mod ui;
/// Перечитывание текстового файла по содержимому.
pub mod watch;

pub use assets::{Assets, Font, Handle, MeshGeometry, PendingTexture, Texture, TextureKind};
pub use audio::{Audio, LoopId, Sound, Stereo};
pub use camera::{
    Camera2D, Camera3D, MAX_PITCH, Ray, Viewport, letterbox, letterbox_pixel, screen_to_world,
    viewport,
};
pub use canvas::{CANVAS_FORMAT, Canvas};
pub use clock::FixedClock;
pub use ecs::{Commands, Entity, SparseSet, World};
pub use egui;
pub use frame::{
    Color, Context, CursorGrab, Game, Gfx, Shown, ShownScene, Time, VirtualGrab, VisualFrame,
    step_frame, step_visual,
};
pub use frame_renderer::FrameRenderer;
pub use glam::{IVec2, Mat3, Mat4, Quat, UVec2, Vec2, Vec3, ivec2, vec2, vec3};
pub use gpu::Gpu;
pub use gpu_textures::GpuTextures;
pub use input::{Input, InputEvent, wheel_lines};
pub use mesh::{Mesh, PendingMesh, Vertex};
pub use mesh_pipeline::MeshPipeline;
pub use model::{
    Aabb3, Animation, Channel, Material, Model, Node, Pose, Primitive, Skin, Trs, Values,
};
pub use physics::{
    Aabb, Actor, Axis, Contact, Hits, Solid, Tilemap, grounded, move_actor, move_solid,
};
pub use physics3d::{
    ContactParams, Hit, RigidBody, StaticWorld, Triangle, box_inertia, solve_contacts,
};
pub use present::{PresentPass, linear_present};
pub use renderer::{RenderError, Renderer};
pub use runner::{RunError, RunReport, Runner, Snapshot, run_script};
pub use scene::{
    DirectionalLight, ExpandedModels, Fog, MAX_JOINTS, MAX_LIGHTS, MeshBatch, MeshDraw,
    MeshInstance, ModelDraw, PointLight, Scene, SkinnedDraw, Viewmodel, batch_meshes,
};
pub use script::{Script, ScriptError, ScriptEvent};
pub use serde_json;
pub use sprite::{Batch, Instance, Rect, Sprite, batch};
pub use sprite_pipeline::SpritePipeline;
pub use stats::{FrameReport, FrameStats, RateCounter};
pub use text::{FontData, Glyph};
pub use ui::UiFrame;
pub use watch::{WATCH_TICKS, Watcher};
pub use winit::event::{ElementState, MouseButton};
pub use winit::keyboard::KeyCode;

use winit::event_loop::{ControlFlow, EventLoop};

/// Параметры окна и цикла для `run`.
pub struct Config {
    /// Заголовок окна.
    pub title: String,
    /// Начальный размер окна в логических пикселях.
    pub size: (u32, u32),
    /// Тиков `update` в секунду.
    pub tick_hz: u32,
    /// `false` — показ кадров без ожидания vsync.
    pub vsync: bool,
    /// Целый масштаб мира и целые отступы (`letterbox_pixel`) — для пиксель-арта.
    pub pixel_perfect: bool,
}

/// Открывает окно и крутит игру `G` до закрытия; с `--script <путь> [--out <каталог>] [--window] [--no-png]` прогоняет сценарий без окна или, с `--window`, в окне и печатает путь `report.json`.
///
/// # Errors
/// Неверные аргументы, ошибка цикла событий или прогона сценария.
pub fn run<G: Game>(config: Config) -> Result<(), RunError> {
    env_logger::init();
    let args = cli::parse_args(std::env::args_os().skip(1))?;
    if let Some(script) = &args.script {
        let report = cli::run_script_file::<G>(
            &config,
            script,
            args.out.as_deref(),
            args.window,
            args.no_png,
        )?;
        #[expect(clippy::print_stdout, reason = "путь отчёта — результат команды")]
        {
            println!("{}", report.display());
        }
        return Ok(());
    }
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app::App::<G>::new(config))?;
    Ok(())
}
