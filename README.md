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
stapel = { git = "https://github.com/lllypuk/stapel", rev = "fda36236f4043be4b82d85313728cc062520fa96" }
```

This rev is the one the sample game was verified on, not a promise of stability.

`src/main.rs`, the whole `clear-color` game with a `report` and a test:

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

    fn report(&self) -> stapel::serde_json::Value {
        stapel::serde_json::json!({ "index": self.index })
    }
}

fn config() -> Config {
    Config {
        title: "clear-color".into(),
        size: (800, 600),
        tick_hz: 60,
        vsync: true,
        pixel_perfect: false,
    }
}

fn main() -> Result<(), stapel::RunError> {
    stapel::run::<ClearColor>(config())
}

#[cfg(test)]
mod tests {
    use super::*;
    use stapel::{RunError, Script, run_script};

    #[test]
    fn space_cycles_color() {
        let script = Script::parse(include_str!("../scripts/cycle.toml")).unwrap();
        match run_script::<ClearColor>(&config(), &script, None) {
            Err(RunError::NoAdapter(why)) if std::env::var_os("REQUIRE_GPU").is_none() => {
                eprintln!("SKIPPED: {why}")
            }
            Err(e) => panic!("{e}"),
            Ok(run) => assert_eq!(run.snapshots[1].report["index"], 1),
        }
    }
}
```

`init` loads assets once through `ctx.assets`; paths are relative to the working directory, so run from the crate root and keep files in the crate's own `assets/` (the engine ships only a font for its own games: bring a `.ttf` for `load_font`). `update` runs every fixed tick and reads the input snapshot in `ctx.input`. `draw` fills `ctx.gfx` immediate-mode every frame; for 3D set `ctx.gfx.scene`. The optional `ui` method draws `egui` panels. Text covers ASCII 32..=126 only: other characters draw as `?`, while `texts` in the report keeps the original string; the engine logs a `warn` once per character and font, so write HUD text in English. Transparency is `Color { a: 0.5, ..color }`; text width is `ctx.assets.font(handle).width(s)` in rasterisation pixels, multiplied by the `scale` of `draw_text`. With the git dependency the engine source is in `~/.cargo/git/checkouts/stapel-*/`; read it there. The games in `games/` are the reference for everything else: each has a README with its data formats.

## Check a game without a window

Every game started through `stapel::run` also takes `--script <file.toml> [--out <dir>] [--window] [--no-png]`: it plays the scenario tick by tick on an offscreen GPU target, saves PNG snapshots and `report.json`, prints the path of the report and exits. `--window` plays the same scenario in a window; pixels and report match the offscreen run. Without `--out` the files go to `target/stapel-run/<scenario name>/`; old `tick-*.png` and `report.json` there are deleted at the start of a run, other files are left alone. `--no-png` writes `report.json` only, with `png` set to `null`: use it while tuning a scenario, then run once without it for the pictures. Other arguments are ignored; `--out`, `--window` and `--no-png` without `--script` are an error, as is `--no-png` with `--window`.

`scripts/cycle.toml` for the game above:

```toml
ticks = 10            # how many times update runs; required
size = [320, 240]     # physical pixels; default is Config::size
snapshots = [0, 5]    # ticks to capture; the last tick is always captured
# seed = 7            # ctx.seed before Game::init; default 0; any u64
# scale = 1.0         # physical pixels per egui point; default 1

[[event]]
tick = 2
press = "Space"       # any KeyCode name; release = "Space" lifts it

[[event]]
tick = 3
release = "Space"
```

Each `[[event]]` has `tick` (`1..=ticks`, non-decreasing down the file) and exactly one action: `press`, `release`, `cursor = [x, y]`, `cursor_left = true`, `button_press` / `button_release = "Left"`, `motion = [dx, dy]`, `wheel = [x, y]`, `focus = false`. Coordinates are physical pixels. Unknown fields, out-of-range ticks, non-finite coordinates and a `press` with a `release` of the same key (or button) in one tick are errors that name the entry (`event[N]`): the press would never reach `update`. Release in a later tick.

Ticks:

- Tick 0 is the initial frame: `update` is not called, `draw` and `ui` run, so the first snapshot shows the start.
- At tick N >= 1 the events of tick N are applied in file order, then one frame runs: one `update`, `draw` with `alpha = 0`, `ui`, render. Snapshot N is taken after that frame.
- Start state: window focused, no cursor until the first `cursor` event, no grab. A grab requested in `update` at tick N is in effect from tick N + 1, so a `motion` in the click's own tick is dropped, as in a window.

`--out` holds `tick-000005.png` per snapshot (what the screen would show, opaque) and `report.json`, abridged here:

```json
{"ticks": 10, "adapter": "AMD Radeon Graphics (RADV RENOIR)", "backend": "Vulkan", "snapshots": [
  {"tick": 0, "png": "tick-000000.png", "sprites": 0, "texts": [], "report": {"index": 0}}
]}
```

`sprites` is the sprite count of the frame; `texts` lists the strings passed to `draw_text`; `report` is `Game::report` at that tick (a JSON object, anything else is an error). The same run is available from code: `stapel::run_script::<G>(&config, &script, Some(out_dir))` returns the same data as `RunReport`; with `None` no PNG is read back, which makes it faster than `--out` for tuning a scenario. It returns `RunError::NoAdapter` when the machine has no Vulkan adapter, so a test can skip on it.

What to rely on and what not to:

- Linux only. Offscreen needs a Vulkan adapter (a software one, such as lavapipe, is enough); `--window` needs a display.
- `--window` on Wayland can stop after tick 0 with no output; the cause is not established. After 10 s without progress the run ends with `Stalled` and the tick; try `env -u WAYLAND_DISPLAY`. The offscreen run is not affected.
- Text typed into egui panels and modifier keys (Ctrl, Shift, Alt) cannot be scripted yet; clicks, drags, keys and the wheel work, including over panels.
- `--window` shows the picture 1:1 from the top left corner: a window of another size than the scenario crops or smears it (one `warn` with both sizes); byte equality with the offscreen run holds for equal sizes.
- `texts` shows what the game sent to `draw_text`, not what is visible: a HUD covered by something else still appears there. Look at the PNG.
- The game must be deterministic: time, `HashMap` iteration order and random numbers not taken from `ctx.seed` make two runs differ without any error.

For a coding agent working on your game, copy `docs/agents/AGENTS.md` from this repository into the project root: it is written to be the only context the agent needs.

## Documentation

- `docs/agents/AGENTS.md` (English): instructions for a coding agent that writes a game on Stapel.
- `docs/friction/README.md` (Russian): format of the friction log kept by such an agent.
- `docs/design.md` (Russian): decisions and their reasons, the list of silent failure modes and what catches each, the stage table.
- `docs/assets.md` (Russian): where to get assets and under which licenses.
- `CLAUDE.md` (Russian): architecture summary and the commands for checks, coverage, and mutation testing. Written for coding agents, useful for people too.

## License

Code: MIT, see `LICENSE`. Assets are credited per game in `games/<name>/assets/CREDITS.md`; they are CC0 unless that file says otherwise. The bundled font `assets/fonts/JetBrainsMono-Regular.ttf` is under the SIL Open Font License, see `assets/fonts/OFL.txt`.
