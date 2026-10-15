//! Учебный 2D-движок: окно, фиксированный тик, спрайты и текст через wgpu.
#![deny(missing_docs)]

mod app;
/// Текстуры и шрифты по хэндлам.
pub mod assets;
/// Ортокамера и вписывание кадра в окно.
pub mod camera;
/// Фиксированный шаг симуляции.
pub mod clock;
/// Сущности, компоненты и отложенные команды.
pub mod ecs;
/// Контекст кадра, трейт `Game` и шаг кадра.
pub mod frame;
/// Клавиатура по тикам.
pub mod input;
/// Целочисленные коллизии: AABB и тайловая карта.
pub mod physics;
/// Поверхность окна и рендер кадра.
pub mod renderer;
/// Спрайты и их батчинг в инстансы.
pub mod sprite;
/// GPU-путь спрайтов.
pub mod sprite_pipeline;
/// Счётчики FPS, TPS и времени кадра.
pub mod stats;
mod surface;
/// Растеризация шрифтов и раскладка строк в спрайты.
pub mod text;

pub use assets::{Assets, Font, Handle, PendingTexture, Texture, TextureKind};
pub use camera::{Camera2D, Viewport, letterbox};
pub use clock::FixedClock;
pub use ecs::{Commands, Entity, SparseSet, World};
pub use frame::{Color, Context, Game, Gfx, Time, step_frame};
pub use glam::{IVec2, Vec2, ivec2, vec2};
pub use input::Input;
pub use physics::{Aabb, Tilemap};
pub use renderer::{RenderError, Renderer};
pub use sprite::{Batch, Instance, Rect, Sprite, batch};
pub use sprite_pipeline::SpritePipeline;
pub use stats::{FrameReport, FrameStats, RateCounter};
pub use text::{FontData, Glyph};
pub use winit::event::ElementState;
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
