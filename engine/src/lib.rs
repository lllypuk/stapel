//! Учебный движок: окно, фиксированный тик, спрайты, текст и меши через wgpu.
#![deny(missing_docs)]

mod app;
/// Текстуры, шрифты и звуки по хэндлам.
pub mod assets;
/// Звуки, их запуск и микшер.
pub mod audio;
/// Ортокамера и вписывание кадра в окно.
pub mod camera;
/// Фиксированный шаг симуляции.
pub mod clock;
/// Сущности, компоненты и отложенные команды.
pub mod ecs;
/// Контекст кадра, трейт `Game` и шаг кадра.
pub mod frame;
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
/// Поверхность окна и рендер кадра.
pub mod renderer;
/// 3D-кадр и его раскрытие в инстансы.
pub mod scene;
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

pub use assets::{Assets, Font, Handle, PendingTexture, Texture, TextureKind};
pub use audio::{Audio, Sound};
pub use camera::{
    Camera2D, Camera3D, MAX_PITCH, Viewport, letterbox, letterbox_pixel, screen_to_world, viewport,
};
pub use clock::FixedClock;
pub use ecs::{Commands, Entity, SparseSet, World};
pub use egui;
pub use frame::{Color, Context, Game, Gfx, Shown, Time, step_frame};
pub use glam::{IVec2, Mat4, Quat, Vec2, Vec3, ivec2, vec2, vec3};
pub use gpu_textures::GpuTextures;
pub use input::Input;
pub use mesh::{Mesh, PendingMesh, Vertex};
pub use mesh_pipeline::MeshPipeline;
pub use model::{Animation, Channel, Model, Node, Primitive, Skin, Values};
pub use physics::{
    Aabb, Actor, Axis, Contact, Hits, Solid, Tilemap, grounded, move_actor, move_solid,
};
pub use renderer::{RenderError, Renderer};
pub use scene::{
    Fog, MAX_LIGHTS, MeshBatch, MeshDraw, MeshInstance, PointLight, Scene, batch_meshes,
};
pub use sprite::{Batch, Instance, Rect, Sprite, batch};
pub use sprite_pipeline::SpritePipeline;
pub use stats::{FrameReport, FrameStats, RateCounter};
pub use text::{FontData, Glyph};
pub use ui::UiFrame;
pub use winit::event::{ElementState, MouseButton};
pub use winit::keyboard::KeyCode;

use winit::event_loop::{ControlFlow, EventLoop};

pub use winit::error::EventLoopError;

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

/// Открывает окно и крутит игру `G` до закрытия.
///
/// # Errors
/// Если не создался или упал цикл событий `winit`.
pub fn run<G: Game>(config: Config) -> Result<(), EventLoopError> {
    env_logger::init();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app::App::<G>::new(config))
}
