#[cfg_attr(not(test), expect(dead_code, reason = "трассу ставит сцена, Task 11"))]
mod pieces;
#[cfg_attr(not(test), expect(dead_code, reason = "трассу ставит сцена, Task 11"))]
mod track;

use engine::{Config, Context, Game};

struct Racing;

impl Game for Racing {
    fn init(_ctx: &mut Context) -> Self {
        Self
    }

    fn update(&mut self, _ctx: &mut Context) {}

    fn draw(&mut self, _ctx: &mut Context, _alpha: f32) {}
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Racing>(Config {
        title: "racing".into(),
        size: (1280, 720),
        tick_hz: 60,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: false,
    })
}
