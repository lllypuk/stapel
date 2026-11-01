use stapel::{Color, Config, Context, Game, KeyCode};

const PALETTE: [[u8; 3]; 4] = [[30, 30, 46], [180, 60, 60], [60, 160, 90], [70, 110, 200]];

struct ClearColor {
    index: usize,
}

impl Game for ClearColor {
    fn init(_ctx: &mut Context) -> Self {
        Self { index: 0 }
    }

    fn update(&mut self, ctx: &mut Context) {
        if ctx.input.just_pressed(KeyCode::Space) {
            self.index = (self.index + 1) % PALETTE.len();
        }
    }

    fn draw(&mut self, ctx: &mut Context, _alpha: f32) {
        let [r, g, b] = PALETTE[self.index];
        ctx.gfx.clear_color = Color::from_srgb8(r, g, b);
    }
}

fn main() -> Result<(), stapel::RunError> {
    stapel::run::<ClearColor>(Config {
        title: "clear-color".into(),
        size: (800, 600),
        tick_hz: 60,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stapel::{ElementState::Pressed, FixedClock, step_frame};
    use std::time::Duration;

    #[test]
    fn space_switches_color_once_per_press() {
        let mut clock = FixedClock::new(60, Duration::from_millis(250));
        let step = clock.step();
        let mut ctx = Context::new(step);
        let mut game = ClearColor::init(&mut ctx);

        ctx.input.on_key(KeyCode::Space, Pressed, false);
        step_frame(&mut game, &mut ctx, &mut clock, step * 3);
        assert_eq!(game.index, 1);

        ctx.input.focus_lost();
        ctx.input.on_key(KeyCode::Space, Pressed, true);
        step_frame(&mut game, &mut ctx, &mut clock, step * 3);
        step_frame(&mut game, &mut ctx, &mut clock, step * 3);
        assert_eq!(game.index, 1);
        assert_eq!(ctx.gfx.clear_color, Color::from_srgb8(180, 60, 60));
    }
}
