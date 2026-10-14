mod app;
pub mod assets;
pub mod camera;
pub mod clock;
pub mod ecs;
pub mod frame;
pub mod input;
pub mod renderer;
pub mod sprite;
pub mod sprite_pipeline;
pub mod stats;
pub mod text;

pub use assets::{Assets, Font, Handle, PendingTexture, Texture, TextureKind};
pub use camera::{Camera2D, Viewport, letterbox};
pub use clock::FixedClock;
pub use ecs::{Entity, SparseSet, World};
pub use frame::{Color, Context, Game, Gfx, Time, step_frame};
pub use glam::{Vec2, vec2};
pub use input::Input;
pub use renderer::{RenderError, Renderer};
pub use sprite::{Batch, Instance, Rect, Sprite, batch};
pub use sprite_pipeline::SpritePipeline;
pub use stats::RateCounter;
pub use text::{FontData, Glyph};
pub use winit::event::ElementState;
pub use winit::keyboard::KeyCode;

use winit::event_loop::{ControlFlow, EventLoop};

pub use winit::error::EventLoopError;

pub struct Config {
    pub title: String,
    pub size: (u32, u32),
    pub tick_hz: u32,
    pub vsync: bool,
}

/// Открывает окно и крутит игру `G` до закрытия.
pub fn run<G: Game>(config: Config) -> Result<(), EventLoopError> {
    env_logger::init();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app::App::<G>::new(config))
}
