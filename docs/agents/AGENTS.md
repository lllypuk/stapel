# AGENTS.md

You build a small game on Stapel, a Rust game engine, from a short idea. You cannot see a window, so you prove the game is playable yourself, with scenarios the engine plays offscreen. A game that compiles is not done.

## Setup

- Linux, Vulkan GPU (a software adapter such as lavapipe is enough). `cargo new <game>`, then in `Cargo.toml`: `stapel = { git = "https://github.com/lllypuk/stapel", rev = "<commit you were given>" }`. The game is its own crate with its own `Cargo.lock`.
- Run everything from the crate root: asset paths and `target/` are relative to the working directory.
- With the git dependency the engine source is in `~/.cargo/git/checkouts/stapel-*/`. Read it only there; do not clone the repository or leave the game's directory.

## Game contract

```rust
use stapel::{Camera2D, Color, Config, Context, Game, Handle, KeyCode, Sprite, Font, vec2};

struct Mine { x: f32, font: Handle<Font> }

impl Game for Mine {
    fn init(ctx: &mut Context) -> Self {            // once; load assets here
        Self { x: 0.0, font: ctx.assets.load_font("assets/font.ttf", 32.0) }
    }
    fn update(&mut self, ctx: &mut Context) {       // fixed tick, 60 per second
        if ctx.input.is_down(KeyCode::KeyD) { self.x += 0.1; }
    }
    fn draw(&mut self, ctx: &mut Context, _alpha: f32) { // every frame; fill ctx.gfx
        ctx.gfx.camera = Camera2D { center: vec2(0.0, 0.0), size: vec2(16.0, 9.0) };
        ctx.gfx.clear_color = Color::from_srgb8(20, 20, 30);
        ctx.gfx.draw_sprite(Sprite { pos: vec2(self.x, 0.0), size: vec2(1.0, 1.0),
            color: Color::from_srgb8(200, 60, 60), ..Sprite::default() });
        ctx.gfx.draw_text(&ctx.assets, self.font, "HELLO", vec2(-7.5, 3.0), 0.03,
            Color::from_srgb8(255, 255, 255), 10);
    }
    fn report(&self) -> stapel::serde_json::Value {  // state for scenarios
        stapel::serde_json::json!({ "x": self.x })
    }
}

fn main() -> Result<(), stapel::RunError> {
    stapel::run::<Mine>(Config { title: "mine".into(), size: (1280, 720), tick_hz: 60,
        vsync: true, pixel_perfect: false })
}
```

- Logic lives in `update`; `draw` only turns state into sprites and text. `ctx.gfx.sprites` is rebuilt every frame.
- World coordinates are in units of your choice; `Camera2D` sets the visible area, Y points up. `Sprite::default()` is a white unit square; `texture: Handle::WHITE` is a solid colour, so a game needs no art to start.
- Input in `update`: `ctx.input.is_down(KeyCode)`, `just_pressed`, `mouse_just_pressed(MouseButton::Left)`, `mouse_down`, `cursor()` (physical pixels), `ctx.cursor_world()` (cursor in world units, `None` before the first shown frame), `wheel`.
- Randomness only from `ctx.seed` (set before `init`); wall-clock time and `HashMap` iteration order make runs differ.
- `fn ui(&mut self, ui: &stapel::egui::Context)` draws egui panels after `draw`. 3D: set `ctx.gfx.scene = Some(Scene { .. })` in `draw`; read the engine source for it.
- `ctx.time.step` is the tick length. Do not use real time in `update`.
- Transparency: `Color { a: 0.5, ..color }`. Text width: `ctx.assets.font(handle).width(s)` is in rasterisation pixels; multiply by the `scale` passed to `draw_text`.

## Assets

- Load in `init` through `ctx.assets`: `load_texture` (PNG), `load_font(path, px)` (TTF), `load_sound` (WAV, OGG), `load_model` (GLB). The same path returns the same handle.
- A missing or broken file does not fail: a texture becomes a checkerboard, a font draws solid boxes, and only a `warn` log tells you. Run with `RUST_LOG=warn` and read stderr.
- Keep files in the crate, e.g. `assets/`. Text needs a TTF: JetBrains Mono (SIL OFL) is at `https://github.com/lllypuk/stapel/raw/main/assets/fonts/JetBrainsMono-Regular.ttf`, licence in `OFL.txt` next to it.
- Text covers ASCII 32..=126 only: any other character draws as `?` (and is measured as `?`), while `texts` in the report keeps the original string. The engine logs a `warn` once per character and font (draw or `width`), so read the log. Cyrillic is unavailable, so write HUD text in English.
- Use only assets you may redistribute. CC0 sources: kenney.nl/assets (sprites, UI, 3D, audio), quaternius.com, kaylousberg.itch.io, opengameart.org (check each file), freesound.org (CC0 or CC-BY per sound), fonts.google.com (OFL). CC-BY needs the author named. Record every file, source and licence in `assets/CREDITS.md`. Prefer solid-colour sprites to unlicensed art.

## Scenarios

A scenario is a TOML file that plays input by tick.

```toml
ticks = 300             # update calls; required
size = [1280, 720]      # physical pixels; default Config::size
snapshots = [0, 150]    # ticks to capture as PNG; the last tick is always captured
# seed = 7  scale = 1.0 (physical pixels per egui point)

[[event]]
tick = 10               # 1..=ticks, non-decreasing down the file
press = "KeyD"          # exactly one action per event
```

Actions: `press` / `release = "<KeyCode name>"`, `cursor = [x, y]`, `cursor_left = true`, `button_press` / `button_release = "Left"`, `motion = [dx, dy]`, `wheel = [x, y]`, `focus = false`. Coordinates are physical pixels. Mistakes (unknown field, tick out of range, two actions) are errors naming `event[N]`.

Ticks:

- Tick 0 is the initial frame: no `update`, but `draw` and `ui` run, so snapshot 0 shows the start.
- At tick N >= 1 the events of that tick are applied in file order, then one `update`, `draw` (`alpha = 0`), `ui` and render run. Snapshot N is taken after that frame.
- Start state: focused, no cursor until a `cursor` event, no grab. A `press` is seen by `just_pressed` in that tick's `update` and stays down until `release`; a click needs a `cursor` event first, then `button_press` and a later `button_release`.
- Aim clicks with coordinates read off an earlier snapshot, not guessed.
- A click that depends on the economy goes by the `report` of the previous run; shifting one click changes everything after it.

## Running and reading

```sh
cargo run -- --script scripts/win.toml --out target/run/win
```

It prints the path of `target/run/win/report.json` and exits; without `--out` files go to `target/stapel-run/<scenario name>/`. `--window` plays the same scenario in a window. Other arguments are ignored.

`report.json`:

```json
{"ticks": 300, "adapter": "...", "backend": "Vulkan", "snapshots": [
  {"tick": 0, "png": "tick-000000.png", "sprites": 12, "texts": ["SCORE 0"], "report": {"phase": "playing"}}
]}
```

- `report` is your `Game::report` at that tick. `sprites` counts sprites in the frame. `texts` lists strings passed to `draw_text`: sent, not visible.
- View every snapshot PNG with your image-reading tool and check what is on screen against the report. A covered, off-screen or invisible element still appears in `texts`.
- The same run from a test: `stapel::run_script::<Mine>(&config, &stapel::Script::parse(text)?, None)` returns the data above as `RunReport` (`snapshots[i].report`, `.texts`, `.sprites`). Match `Err(RunError::NoAdapter(why))` to print `SKIPPED: {why}` and return; a machine without a GPU must not fail the suite, and with `REQUIRE_GPU=1` in the environment the test must panic instead.
- To tune ticks, take frequent snapshots through `run_script(.., None)` in a test or a throwaway program and read `report` and `texts`: no PNG is written or read back from the GPU, so it is faster than `--out`. Use PNG only to look.
- Text typed into egui fields cannot be scripted; clicks and keys can.

## Report fields

Make `Game::report` a JSON object of facts a stranger can check, stable across runs: a `phase` (`"playing"`, `"won"`, `"lost"`), the score or goal counters, positions and health of what matters, remaining time or lives. Derive them only from game state; no times, addresses or unordered collections. An empty report proves nothing.

## Prove it is playable

Deliver, in the repository:

1. A scenario that reaches the goal, driven only by input events. Its report shows the goal state (`phase = "won"` and the counters that earned it) and an earlier snapshot shows it not yet reached.
2. A scenario that reaches the failure state the same way (`phase = "lost"` or equivalent), and an earlier snapshot where it is not yet reached.
3. For both: PNG snapshots at start, mid-way and end that you have viewed, and a note of what each shows. The start snapshot alone is not evidence.
4. A `cargo test` that runs both scenarios through `run_script` and asserts on report fields, so it fails if the goal becomes unreachable.
5. Two runs of a scenario giving equal `report.json` (repeatability; it does not replace 1 and 2).

If a goal or failure cannot be reached by input alone, the game is not playable as built: fix the game, not the scenario.

## FRICTION.md

Keep `FRICTION.md` in the project root from the first hour. Add an entry whenever the engine, its docs or this file cost you time, misled you or lacked something, even if you worked around it. Do not log your own mistakes that the docs covered.

```
## <short title>
- Doing: <what you tried>
- Expected: <what you thought would happen, and why>
- Happened: <what did, with the exact error or output>
- Cost: <attempts or minutes>
- Workaround: <what you did, or none>
- Wanted: <engine change | docs change | other>
```
