# Этап 0: окно, игровой цикл, ввод

## Overview
- Cargo workspace с `engine` (lib) и игрой `games/clear-color` (bin).
- Движок: окно (`winit`), фиксированный шаг 60 Гц с интерполяцией, снимок ввода, трейт `Game`, очистка экрана цветом через `wgpu`.
- Игра: по `Space` меняет цвет фона, раз в секунду пишет в лог FPS и TPS.
- Готово, когда: TPS = 60 при любом FPS ≥ 4 (ниже срабатывает ограничение `dt` 250 мс, и симуляция намеренно замедляется); одно нажатие даёт ровно одну смену цвета, удержание — тоже одну.
- Платформа этапа — Linux desktop (Wayland и X11).

## Context (from discovery)
- Проект пустой, кроме `docs/design.md`. Git не инициализирован.
- Окружение: rustc 1.99, Vulkan 1.4 (RADV, AMD Renoir).
- Версии (crates.io, 2026-10-14): `winit 0.30.13`, `wgpu 30.0.1`, `pollster 1.0.1`, `log 0.4.34`, `env_logger 0.11.11`.

## Development Approach
- **testing approach**: TDD — тест раньше кода для всей логики без окна (часы, ввод, счётчик частоты, шаг кадра).
- Задачи по порядку, следующая — только при зелёных `cargo test` и `cargo clippy --workspace -- -D warnings`.
- Тесты — модули `#[cfg(test)]` в том же файле (идиома Rust).
- Изменения скоупа — сразу в этот файл: ➕ новая задача, ⚠️ блокер.

## Testing Strategy
- Юнит-тесты: `clock`, `input`, `stats`, `frame`, чистые функции `renderer`, логика игры.
- Окно и GPU юнит-тестами не покрываются — проверяются запуском игры (Task 9).

## Solution Overview
- `winit` владеет циклом событий: движок реализует `ApplicationHandler`, а игра встраивается через трейт `Game`.
- Вся логика кадра (`step_frame`) не знает об окне и GPU, поэтому тестируется с фейковой игрой. `app.rs` — тонкая склейка событий `winit` с ней.
- Рисование immediate-mode с первого дня: игра пишет в `ctx.gfx` (сейчас только `clear_color`), `Renderer` после `draw` отдаёт это на GPU. На этапе 1 в `Gfx` появится список спрайтов, а схема останется прежней.
- `wgpu`, а не `softbuffer`: инициализация GPU понадобится на этапе 1, выбрасывать ничего не придётся.

## Technical Details

```
Cargo.toml                 workspace, resolver = "3", edition 2024
engine/src/lib.rs          pub use; run(); Config
engine/src/clock.rs        FixedClock
engine/src/input.rs        Input
engine/src/stats.rs        RateCounter
engine/src/frame.rs        Game, Context, Time, Gfx, Color, step_frame
engine/src/renderer.rs     Renderer (wgpu)
engine/src/app.rs          App: ApplicationHandler
games/clear-color/src/main.rs
```

- `Config { title, size: (u32, u32), tick_hz: u32, vsync: bool }`.
- `FixedClock { step, max_frame, accumulator: Duration }`, `advance(frame_dt) -> u32` (сколько тиков выполнить), `alpha() -> f32`. Накопитель хранится в `Duration` (целые наносекунды), а не в `f32`: на `f32` за часы работы копится ошибка, и TPS медленно уплывает. `step = Duration::from_secs_f64(1/hz)` округляет до 16 666 667 нс, поэтому тесты выражаются через `step`, а не через «круглые» миллисекунды.
- `Input`: `down`, `pressed`, `released` — множества `KeyCode` (`winit::keyboard::KeyCode`, реэкспорт из движка). `on_key(code, state, repeat)`, `focus_lost()`, `end_tick()`. Флаги «только что» чистятся после каждого тика: со второго тика кадра их уже нет, а при 0 тиков они доживают до следующего кадра.
- Гарантия ввода — «хотя бы одно нажатие за тик»: два нажатия одной клавиши между тиками сливаются в одно. Для 60 Гц этого достаточно; очередь событий — когда понадобится вводу текста.
- `Color` — линейный RGBA. Палитра задаётся в привычном sRGB через `Color::from_srgb8`. Если передать sRGB-числа как линейные, цвета на `*Srgb`-поверхности выйдут не теми, и ошибки при этом не будет.
- `RateCounter`: `record(now)` и `per_second() -> Option<u32>` раз в секунду; время передаётся аргументом, поэтому тест не спит.
- `Context { input, time: Time, gfx: Gfx }`, `Time { step: Duration, ticks: u64, fps: u32, tps: u32 }`.
- `step_frame(game, ctx, clock, frame_dt)`: `advance` → N раз (`update`, `input.end_tick()`) → `draw(alpha)`.

## Implementation Steps

### Task 1: Workspace и скелет

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `engine/Cargo.toml`, `engine/src/lib.rs`, `games/clear-color/Cargo.toml`, `games/clear-color/src/main.rs`

- [x] `git init`, `.gitignore` с `/target`
- [x] workspace `members = ["engine", "games/*"]`, зависимости версий из Context в `[workspace.dependencies]`
- [x] `engine` — пустая библиотека, `clear-color` зависит от неё по `path`
- [x] `cargo build` и `cargo test` проходят

### Task 2: Фиксированный шаг — `FixedClock`

Читать: Glenn Fiedler, «Fix Your Timestep!».

**Files:**
- Create: `engine/src/clock.rs`

- [x] тесты: `dt` = `step` → 1 тик; `a`, затем `step - a` → 0, затем 1 тик; `dt` = 1 с → тиков `max_frame / step`, остаток сохраняется; `alpha` в `[0, 1)`; `tick_hz = 30` → шаг ≈ 1/30 с
- [x] реализация `FixedClock::new(tick_hz, max_frame)`, `advance`, `alpha`
- [x] `cargo test` — зелёный

### Task 3: Снимок ввода — `Input`

**Files:**
- Create: `engine/src/input.rs`

- [ ] тесты: нажатие → `just_pressed` до `end_tick`, после — только `is_down`; событие с `repeat = true` игнорируется (иначе удержание клавиши «нажимает» её много раз); нажатие и отпускание до тика → в тике `just_pressed` и `just_released`, `is_down == false` (короткий тап не теряется); отпускание → `just_released` один тик; `focus_lost` → `down` пуст (иначе клавиша, отпущенная в другом окне, «залипает»)
- [ ] реализация `on_key`, `focus_lost`, `is_down`, `just_pressed`, `just_released`, `end_tick`
- [ ] `cargo test` — зелёный

### Task 4: Счётчик частоты — `RateCounter`

**Files:**
- Create: `engine/src/stats.rs`

- [ ] тесты: 60 событий за 1 с → `Some(60)`; до истечения секунды → `None`; после отчёта счёт начинается заново
- [ ] реализация
- [ ] `cargo test` — зелёный

### Task 5: `Game`, `Context` и `step_frame`

**Files:**
- Create: `engine/src/frame.rs`
- Modify: `engine/src/lib.rs`

- [ ] тесты с фейковой игрой, которая считает вызовы: кадр на 2 шага → 2 `update`, 1 `draw`; нажатие перед кадром на 2 тика → `just_pressed` видит только первый `update`; кадр на 0 тиков → `just_pressed` доживает до следующего кадра; `time.ticks` растёт на число тиков
- [ ] тест `Color::from_srgb8`: 0 → 0.0, 255 → 1.0, 128 → ≈0.216
- [ ] `Color` (линейный RGBA, `f64`, как в `wgpu::Color`), `Gfx { clear_color }`, `Time`, `Context`, трейт `Game`
- [ ] `step_frame`
- [ ] `cargo test` — зелёный

### Task 6: `Renderer` — очистка экрана через wgpu

Читать: «Learn Wgpu», главы Dependencies and the window и The Surface.

**Files:**
- Create: `engine/src/renderer.rs`

- [ ] тесты чистых функций: размер с нулевой шириной **или** высотой → поверхность недоступна, кадр не берётся (wgpu паникует на `configure` с нулём); результат `get_current_texture()` (`CurrentSurfaceTexture` в wgpu 30) → действие: `Success` → рисовать; `Suboptimal` → рисовать, переконфигурировать после `present` (не раньше: пока жива текстура, `configure` нельзя); `Timeout`/`Occluded` → пропустить кадр; `Outdated` → `configure`; `Lost` → пересоздать `Surface` и `configure`; `Validation` → ошибка в лог и выход
- [ ] `Renderer::new(Arc<Window>, vsync)`: instance → surface → adapter → device/queue (`pollster::block_on`), формат поверхности — `*Srgb`, `PresentMode::AutoVsync` / `AutoNoVsync`; выбранный present mode в лог (`AutoNoVsync` может откатиться на `Fifo`)
- [ ] `resize(size)`, `render(&Gfx) -> bool` (кадр показан или нет): текстура поверхности → render pass с `LoadOp::Clear(clear_color)` → `submit` → `window.pre_present_notify()` → `present`
- [ ] `cargo test` — зелёный

### Task 7: `App` и `engine::run`

Читать: документацию `winit::application::ApplicationHandler`.

**Files:**
- Create: `engine/src/app.rs`
- Modify: `engine/src/lib.rs`

- [ ] `run::<G>(Config)`: `env_logger`, `EventLoop`, `ControlFlow::Poll`, `run_app`
- [ ] `resumed`: окно и `Renderer` создаются один раз (повторный вызов ничего не делает), затем `G::init`
- [ ] `window_event`: `KeyboardInput` → `Input::on_key`, события с `is_synthetic` отбрасываются (X11 присылает фальшивое нажатие при возврате фокуса); `Focused(false)` → `Input::focus_lost`; `Resized` → `Renderer::resize`; `CloseRequested` → `exit`; `RedrawRequested` → `step_frame`, `render`, счётчики → `ctx.time` (FPS считает только показанные кадры)
- [ ] `about_to_wait` → `request_redraw`, только если поверхность доступна (иначе свёрнутое окно крутит пустые кадры на 100% CPU)
- [ ] тестов нет: логика покрыта задачами 2–5, склейка проверяется в Task 9; `cargo test` и `cargo clippy` — зелёные

### Task 8: Игра `clear-color`

**Files:**
- Modify: `games/clear-color/src/main.rs`

- [ ] тест через `step_frame`: `Space` нажат перед кадром на 3 тика → цвет сменился ровно один раз; удержание на следующих кадрах цвет не меняет
- [ ] палитра из нескольких цветов, `Space` → следующий
- [ ] раз в секунду `log::info!` с FPS и TPS
- [ ] `vsync` из переменной окружения: `NO_VSYNC=1` выключает
- [ ] `cargo test` — зелёный

### Task 9: Verify acceptance criteria
- [ ] `cargo test --workspace` и `cargo clippy --workspace -- -D warnings`
- [ ] `RUST_LOG=info cargo run -p clear-color`: TPS 60, FPS = частота монитора
- [ ] `NO_VSYNC=1 RUST_LOG=info cargo run -p clear-color`: TPS 60; FPS выше частоты монитора, если в логе present mode не `Fifo`
- [ ] короткое нажатие и долгое удержание `Space` — по одной смене цвета
- [ ] зажать `Space`, переключиться в другое окно, отпустить, вернуться — цвет не меняется
- [ ] ресайз и сворачивание не роняют игру; свёрнутое окно не грузит CPU

### Task 10: [Final] Update documentation
- [ ] `README.md`: что это, как собрать и запустить, ссылка на `docs/design.md`
- [ ] перенести план в `docs/plans/completed/`

## Post-Completion
- Проверить на Wayland и X11 (`WAYLAND_DISPLAY= cargo run -p clear-color` принудительно включает X11), если доступны оба.
