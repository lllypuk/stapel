# Stapel

A small 2D and 3D game engine in Rust, built as a framework: `winit` owns the event loop, `wgpu` draws, your game implements one trait. The engine grows only when a game in `games/` needs something, so every feature has a working consumer next to it.

Status: preliminary. Linux only (Wayland and X11, Vulkan through `wgpu`); other platforms are untested and not promised. The API is not stable: depend on a pinned commit and read the breaking-change notes when you move it.

## What you get

- Fixed 60 Hz simulation with free-running rendering and interpolation.
- Sprites with a batcher and an orthographic camera; text from TTF fonts.
- 3D: meshes, glTF models with GPU skinning, directional and point lights, fog, a perspective or orthographic camera, a separate view-model layer.
- A small ECS, 2D collisions, and a 3D rigid-body library with a static triangle world.
- Audio mixer with music, spatial effects, and live-tuned looping voices.
- Hot reload of textures, sounds, and game data files; `egui` panels for in-game editors.

## Run the example games

```sh
cargo test --workspace
make pong          # or any directory name from games/, release build
```

| Game | What it exercises | Controls |
|---|---|---|
| `clear-color` | window, fixed step, input | Space cycles the background |
| `pong` | sprites, camera, text | W/S left paddle, O/L right paddle |
| `asteroids` | ECS, 5 000+ sprites | A/D turn, W thrust, Space fire, Enter restart |
| `platformer` | collisions, audio, levels from files, editor | A/D run, Space jump, Enter next level, F1 editor |
| `maze` | first-person 3D, skinning, lights, fog | WASD, mouse look, click to grab, Esc to release, F1 panel |
| `racing` | 3D physics, track files, editor, engine sound | WASD, Space handbrake, R respawn, F1 editor, F2 tuning |
| `shooter` | spatial audio, view-model layer, bots | WASD, mouse, 1/2 weapon, Enter restart |
| `trains` | orthographic 3D, data-driven track with switches | W/S drive, E switch ahead, click switch, Tab depot |

## Use it from your own project

Cargo.toml:

```toml
[dependencies]
stapel = { git = "https://github.com/lllypuk/stapel", rev = "<commit>" }
```

`src/main.rs`, the whole `clear-color` game:

```rust
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

fn main() -> Result<(), stapel::EventLoopError> {
    stapel::run::<ClearColor>(Config {
        title: "clear-color".into(),
        size: (800, 600),
        tick_hz: 60,
        vsync: true,
        pixel_perfect: false,
    })
}
```

`init` loads assets once through `ctx.assets`. `update` runs every fixed tick and reads the input snapshot in `ctx.input`. `draw` fills `ctx.gfx` immediate-mode every frame; for 3D set `ctx.gfx.scene`. The optional `ui` method draws `egui` panels. The games in `games/` are the reference for everything else: each has a README with its data formats.

## Documentation

- `docs/design.md` (Russian): decisions and their reasons, the list of silent failure modes and what catches each, the stage table.
- `docs/assets.md` (Russian): where to get assets and under which licenses.
- `CLAUDE.md` (Russian): architecture summary and the commands for checks, coverage, and mutation testing. Written for coding agents, useful for people too.

## License

Code: MIT, see `LICENSE`. Assets are credited per game in `games/<name>/assets/CREDITS.md`; they are CC0 unless that file says otherwise. The bundled font `assets/fonts/JetBrainsMono-Regular.ttf` is under the SIL Open Font License, see `assets/fonts/OFL.txt`.
