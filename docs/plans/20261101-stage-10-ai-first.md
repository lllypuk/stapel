# Этап 10: офскрин-runner и игра из абзаца во внешнем каталоге

## Overview
- Главный барьер для постороннего с агентом: агент не видит кадр и не может сам доказать, что игра играется.
- Движок:
  - runner проигрывает TOML-сценарий по тикам без окна, сохраняет PNG снимков и отчёт;
  - окно и runner идут одним путём кадра и рисуют в одну промежуточную текстуру;
  - вход: `stapel::run_script` из теста, `--script` у любого бинарника игры, `--script --window`.
- Игра: не в `games/`, а во внешнем каталоге с git-зависимостью на закреплённый rev. Пишет одна холодная сессия агента по абзацу идеи и опубликованному README.
- Готово, когда:
  - холодная сессия сама доказала через runner, что игра играется, а независимая проверка подтвердила это по критериям, зафиксированным до её запуска;
  - один сценарий `trains` с кликом после движения камеры, панелью egui и HUD даёт одинаковый отчёт и пиксели промежуточной текстуры в окне и офскрином;
  - тот же сценарий в двух процессах офскрином даёт одинаковый результат;
  - журнал трения заполнен и разобран: каждая запись стала правкой движка, правкой доков, пунктом этапа 11 или «не чиним» с причиной;
  - `make check`, `make cov`, `make mutants-diff` зелёные; офскрин-тесты — под `REQUIRE_GPU=1`.
- Вне этапа: crates.io, генератор, стабильный API, macOS и Windows, шаблон и полный `AGENTS.md` (этап 11), перевод док-комментариев (этап 11).

## Context (from discovery)
- **Кадр.** Порядок зашит в `Running::frame` (`engine/src/app.rs:170`): `begin_pass`, `step_frame`, `sync_grab`, `Game::ui`, `Audio::flush`, `release_focus`, `end_pass`, `tessellate`, `Renderer::render`, запись `shown`.
- **Захват.** `sync_grab` (`app.rs:243`) зовёт окно и по результату пишет `ctx.grabbed`.
- **Рендер.** `Renderer` (`engine/src/renderer.rs:15`) владеет окном и поверхностью и сам создаёт `device` и `queue` под совместимый с поверхностью адаптер. `draw` рисует прямо в кадр поверхности, два view: мир `*Srgb`, egui через `ui_format`. Цель зависит от формата поверхности; второй view поверхности есть только при `SURFACE_VIEW_FORMATS`.
- **Ввод.** `ui::to_input` (`engine/src/ui.rs:19`) принимает `WindowEvent`. Перевод в `Input` — в `App::window_event` (`app.rs:103`). `Input::on_wheel` принимает `MouseScrollDelta` winit.
- **Точки входа.** `run` (`engine/src/lib.rs:111`) возвращает `EventLoopError`, его сигнатуру повторяют `main` восьми игр. Аргументы командной строки движок и игры не читают.
- **Текст.** `Gfx::draw_text` (`engine/src/frame.rs:103`) сразу раскладывает строку в спрайты, строка теряется.
- **Недетерминизм.** `SystemTime` — только сид Asteroids (`games/asteroids/src/main.rs:690`). `Instant` — только тест автопилота. `HashMap` и `HashSet` есть в логике `asteroids` и `racing`, порядок обхода меняется от процесса к процессу.
- **CI.** Lavapipe без дисплея (`.gitlab-ci.yml`, `ci/Dockerfile`): офскрин работает, окно — нет.
- **Зависимости.** У `winit` есть фича `serde`: имена клавиш в сценарии — имена `KeyCode`. `serde` и `toml` уже в workspace, `serde_json` — нет.
- **Часы.** `FixedClock::advance` (`engine/src/clock.rs:26`) зажимает `dt` до `max_frame`: при `MAX_FRAME = 250 мс` и `tick_hz < 4` шаг `clock.step()` не даёт тика.
- **Фокус.** `Context::new` оставляет `focused = false`, окно выставляет его само; `on_mouse_motion` (`engine/src/frame.rs:206`) без фокуса и захвата сдвиг отбрасывает.
- **Покрытие.** `app.rs`, `renderer.rs` исключены из `make cov` и мутантов; всё новое, кроме склейки окна, должно быть вне этих файлов.

## Development Approach
- **testing approach**: TDD, как на этапах 7–9. Тест раньше кода для всего, что не требует окна. GPU-путь — офскрин-тесты с `REQUIRE_GPU=1`.
- Работа — в ветке `stage-10-ai-first`.
- Каждая задача закрывается `cargo mutants -f <файлы задачи>` и `make cov`; следующая — только при зелёном `make check`.
- Зависимость добавляется в задаче, где впервые используется: иначе `cargo machete` упадёт.
- Решение, меняющее `docs/design.md` или «Архитектуру» в `CLAUDE.md`, и новая молчаливая поломка с пометкой «→ что ловит» — в той же задаче.
- Изменения скоупа — сразу в этот файл: ➕ новая задача, ⚠️ блокер.

## Testing Strategy
- **Без GPU:** разбор сценария, расписание событий, сборка `RawInput`, маршрутизация ввода, порядок шага кадра на фейковой игре, разбор аргументов.
- **Офскрин:** `Canvas` и readback, `PresentPass` на текстуре-заменителе поверхности, `run_script` на игрушечной игре с egui, спрайтами и 3D.
- **Сквозные:** `games/trains/tests/replay.rs` запускает бинарник дважды и сравнивает результат.
- **Вне покрытия, проверяется запуском:** настоящий захват курсора, перевод настоящих событий winit, вывод на настоящую поверхность. Совпадение `--window` с офскрином доказывает результат до вывода на поверхность и не доказывает перевод событий winit.

## Solution Overview
- Один путь кадра: `step_visual` выполняет порядок из `app.rs` и принимает `egui::RawInput`, `dt` и адаптер захвата. Обычное окно и `Runner` различаются только источником этих трёх.
- Одна цель: `FrameRenderer` без окна рисует мир, спрайты и egui в переданные view, у обоих режимов это `Canvas`. Окно добавляет `PresentPass` на поверхность, runner — readback в PNG. Ветки `COPY_SRC` у поверхности нет. Цена — один полноэкранный проход, замер — в задачах 1 и 5.
- `--window` — тот же `Runner`, только его `Canvas` показывается в окне. События окна, кроме закрытия и resize поверхности, игнорируются. Поэтому режим не зависит от мыши, живого `dt` и настоящего захвата.
- Правило «функция только под игру» получает исключение: игра-потребитель этапа 10 живёт вне репозитория, сквозная проверка движка внутри него — `trains`.

## Technical Details

### Сценарий
```toml
seed = 7                  # попадает в ctx.seed до Game::init; по умолчанию 0
ticks = 600               # сколько раз вызвать update
size = [800, 600]         # физические пиксели; по умолчанию Config::size
scale = 1.0               # физических пикселей на точку egui
snapshots = [0, 120]      # последний тик снимается всегда

[[event]]
tick = 1
press = "KeyW"            # имя KeyCode

[[event]]
tick = 30
cursor = [400.0, 300.0]   # физические пиксели; cursor_left = true — ушёл с окна

[[event]]
tick = 30
button_press = "Left"     # также release, button_release, motion = [dx, dy], wheel = [x, y], focus = false
```
- `deny_unknown_fields`. В записи ровно одно действие. Ошибка разбора называет номер записи `event[N]`.
- `tick` события — `1..=ticks`, записи идут по неубыванию `tick`. Снимок — `0..=ticks`. Всё остальное — ошибка, а не молчаливо не сработавшее событие.

### Смысл тиков
- Тик 0 — начальный кадр: `dt = 0`, `update` не вызывается, `draw`, `ui` и рендер работают. Без него нет `shown` и геометрии панелей для `wants_pointer`.
- Тик N, N ≥ 1: события тика N применяются по порядку файла, затем кадр с `dt = clock.step()` делает ровно один `update`, `draw` с `alpha = 0`, `ui`, рендер, запись `shown`.
- Снимок N снимается после рендера кадра N.
- Начальное состояние: фокус есть, одинаково в `Context` и `RawInput`; курсора нет до первого `cursor`; захвата нет.
- Захват, запрошенный в `update` тика N, виден с тика N+1: `motion` в том же тике, что и клик захвата, отбрасывается, как в окне.
- Часы `Runner`: `FixedClock::new(tick_hz, Duration::MAX)`, поэтому `clock.step()` даёт ровно один `update` при любой частоте.

### Отчёт
`report.json` в `--out`, та же структура — `RunReport`:
```json
{"ticks": 600, "adapter": "...", "backend": "Vulkan", "snapshots": [
  {"tick": 0, "png": "tick-000000.png", "sprites": 12, "texts": ["SCORE 0"], "report": {}}
]}
```
- `png` — имя файла внутри `--out`, поэтому отчёты сравниваются целиком. В `run_script` без `out` — `null`, readback не выполняется.
- `texts` — строки `draw_text` за кадр, отправленные, а не видимые: перекрытый HUD тут не виден.
- `report` — `Game::report`, контракт: JSON-объект; иное — `RunError::Report` с номером тика.
- PNG: байты `*Srgb` как на экране, альфа принудительно 255, поверхность непрозрачна.

### API
```rust
pub trait Game {
    /* init, update, draw, ui */
    fn report(&self) -> serde_json::Value { serde_json::json!({}) }
}

pub fn run_script<G: Game>(config: &Config, script: &Script, out: Option<&Path>)
    -> Result<RunReport, RunError>;
pub fn run<G: Game>(config: Config) -> Result<(), RunError>;
```
- `run_script` берёт `Config`: `tick_hz`, `pixel_perfect`, размер по умолчанию.
- `RunError`: `EventLoop`, `Args`, `Script`, `NoAdapter`, `Size`, `Report`, `Io`, `Png`. `NoAdapter` отдельно: тесты пропускаются по нему вне `REQUIRE_GPU`. `Size`: размер сценария больше лимита текстур; зажимать нельзя, снимки разойдутся со сценарием.
- `run` без `--script` работает как раньше. Аргументы: `--script <path> [--out <dir>] [--window]`, по умолчанию `--out target/stapel-run/<имя сценария>`. Чужие аргументы игнорируются; `--out` и `--window` без `--script` — ошибка.
- `pub use serde_json` из `stapel`: игра собирает отчёт без своей зависимости.

### Владение GPU
- `Gpu { adapter, device, queue }`, новый `engine/src/gpu.rs`. `Canvas` и `FrameRenderer` принимают его ссылкой и не создают устройство сами.
- Офскрин: `Gpu::headless()`, адаптер без поверхности.
- Окно и `--window`: сначала поверхность и совместимый адаптер, из них `Gpu`; его получают и вывод, и `Runner`. Ресурс с другого устройства в выводе не работает.

### Цель рендера
- `Canvas`: `Rgba8UnormSrgb`, `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC`, `view_formats = [Rgba8Unorm]` при `DownlevelFlags::VIEW_FORMATS`. Пайплайны создаются под формат `Canvas`, а не поверхности.
- Формат egui определяется только `VIEW_FORMATS` у `Canvas`, поверхность на него больше не влияет.
- `PresentPass`: полноэкранный треугольник, `textureLoad` без сэмплера и смешивания, `loadOp = Clear`.
- Формат вывода выбирается отдельно от формата egui: источник и цель линейные только при обоих флагах `VIEW_FORMATS` и `SURFACE_VIEW_FORMATS`, иначе оба `*Srgb`. Линейный источник при цели `*Srgb` дал бы лишнее кодирование и светлую картинку. При линейной паре байты идут без преобразования, при `*Srgb` декод и обратное кодирование дают тот же цвет. Порядок каналов `Bgra` учитывается форматом цели.
- Контракт: 1:1 от левого верхнего угла, координаты зажаты в размер `Canvas`. В обычном окне размеры `Canvas` и поверхности всегда равны, в `--window` размер `Canvas` задан сценарием.

### Виртуальный ввод egui
- `RawInput` собирается без `egui_winit`: `screen_rect` в точках `size / scale`, `native_pixels_per_point = scale`, `time = tick * step`, `predicted_dt = step`, `focused`.
- Координаты `Input` — физические, в egui делятся на `scale` ровно один раз.
- Для каждого события тика порядок как в окне: событие в очередь egui, `wants_pointer` по прошлому проходу и текущему курсору, затем `Input`. Курсор перед кликом успевает повлиять на `wants_pointer`, потому что `Input::cursor` меняется первым событием.

### Адаптер захвата
- `CursorGrab::sync(&mut self, ctx)` — вызывается на месте `sync_grab`.
- `WindowGrab` — нынешняя логика, `ctx.grabbed` по результату окна.
- `VirtualGrab` — `ctx.grabbed = ctx.cursor_grab && ctx.focused`.

## What Goes Where
- **Implementation Steps**: код, тесты и документы этого репозитория, а также запуск холодной сессии и разбор её журнала.
- **Post-Completion**: действия вне репозитория.

## Implementation Steps

### Task 1: Замер до правки рендера

- [x] на текущем `main`: `NO_VSYNC=1 RUST_LOG=info cargo run --release -p racing`, 30 с в гараже; `fps` и `worst` по логу записать сюда
- [x] то же для `shooter` на старте уровня

Замер (NO_VSYNC=1, release, без ввода; первые 5 с отброшены):

| игра | median fps | max worst | sprites | adapter |
|---|---|---|---|---|
| racing (гараж) | 1589 (34 с) | 6.2 ms | 0..0 | AMD Radeon Graphics (RADV RENOIR), Vulkan |
| shooter (старт) | 1679 (29 с) | 16.7 ms | 13..13 | AMD Radeon Graphics (RADV RENOIR), Vulkan |

### Task 2: Событие ввода движка

**Files:**
- Modify: `engine/src/ui.rs`, `engine/src/input.rs`, `engine/src/frame.rs`, `engine/src/app.rs`, `engine/src/lib.rs`

- [ ] тест: `InputEvent` проходит те же случаи, что `to_input` сейчас: нажатие и колесо над панелью не доходят, отпускание и остальное доходят
- [ ] `InputEvent { Key, Button, Cursor, Motion, Wheel, Focus }` на типах `KeyCode`, `MouseButton`, `ElementState`, колесо — строки
- [ ] `ui::to_input` принимает `&InputEvent`
- [ ] `Context::apply(event)` вместо ветвей в `window_event`
- [ ] `app.rs` только переводит `WindowEvent`; фильтр `is_synthetic`, снятие захвата при потере фокуса, resize и `Occluded` остаются в склейке
- [ ] `Input::on_wheel` принимает строки, перевод `MouseScrollDelta` — в склейке; тест перевода пикселей в строки сохранить

### Task 3: Общий шаг кадра

**Files:**
- Modify: `engine/src/frame.rs`, `engine/src/app.rs`, `engine/src/lib.rs`

- [ ] тест на фейковой игре и `egui::Context`: порядок `update`, `draw`, `CursorGrab::sync`, `ui`, снятие фокуса egui
- [ ] тест: `VirtualGrab` — `grabbed` только при `cursor_grab && focused`
- [ ] `step_visual(game, ctx, clock, egui, raw, dt, grab) -> VisualFrame { ticks, ui: UiFrame, platform_output }`
- [ ] `CursorGrab`, `VirtualGrab` в `frame.rs`; `WindowGrab` в `app.rs` с прежней логикой и прежним `grab_warned`
- [ ] `Running::frame` зовёт `step_visual`; проверка запуском `make shooter`: захват, Esc, потеря фокуса
- [ ] `docs/design.md`, раздел «Цикл», и «Архитектура» в `CLAUDE.md`

### Task 4: Canvas и FrameRenderer без окна

**Files:**
- Create: `engine/src/gpu.rs`, `engine/src/canvas.rs`, `engine/src/frame_renderer.rs`
- Modify: `engine/src/renderer.rs`, `engine/src/lib.rs`

- [ ] офскрин-тест: заливка известным цветом, readback отдаёт ожидаемые sRGB-байты при ширине, требующей выравнивания строки
- [ ] офскрин-тест: при полупрозрачном спрайте альфа PNG равна 255
- [ ] `Gpu`, `Gpu::headless`; `Renderer::new` строит `Gpu` из поверхности и совместимого адаптера
- [ ] `Canvas { new, resize, view, ui_view, read_rgba }`; `read_rgba` — `copy_texture_to_buffer`, `poll` до готовности, снятие паддинга
- [ ] `FrameRenderer::new(&gpu, format, ui_format, size, pixel_perfect)`: пайплайны, `depth`, egui; рисует в переданные `view` и `ui_view`
- [ ] в него переносятся заливка ресурсов и `draw`; порядок заливка до проверки поверхности сохраняется с прежним комментарием
- [ ] офскрин-тест: `FrameRenderer` с 3D, `viewmodel`, спрайтами в letterbox и панелью egui рисует все четыре слоя
- [ ] `Renderer` держит `FrameRenderer` и передаёт ему оба view кадра поверхности: окно работает, `Canvas` пока только в тестах

### Task 5: Проход вывода на поверхность

**Files:**
- Create: `engine/src/present.rs`, `engine/src/present.wgsl`
- Modify: `engine/src/renderer.rs`, `docs/design.md`

- [ ] офскрин-тест на текстуре-заменителе `Bgra8UnormSrgb` с view `Bgra8Unorm`: байты совпадают с `Canvas`, каналы не перепутаны, полутона не сдвинуты
- [ ] юнит-тест выбора формата вывода: четыре сочетания флагов, линейная пара только при обоих
- [ ] офскрин-тест смешанного случая: у `Canvas` есть линейный view, цель только `*Srgb`; полутона не сдвинуты
- [ ] офскрин-тест цели только `*Srgb` при `Canvas` без второго view
- [ ] тест resize: после смены размера `Canvas` и цели вывод снова 1:1; цель больше `Canvas` не читает за границей
- [ ] `PresentPass::draw(encoder, source, target)`
- [ ] `Renderer` создаёт `Canvas` и строит `FrameRenderer` под его формат
- [ ] `Renderer::draw`: `FrameRenderer` в `Canvas`, затем `PresentPass` в том же encoder; `pre_present_notify`, `present` и запись `Shown` — как раньше
- [ ] `resize` пересоздаёт `Canvas` вместе с `depth`; поверхность больше не держит цвет между кадрами
- [ ] запуск всех игр из `games/`: гамма панелей egui и ресайз как до правки
- [ ] замер из задачи 1 повторить, числа — сюда и в `docs/design.md`, раздел «Рендер»; при падении FPS больше 5% на тяжёлой сцене — ⚠️ и решение до следующей задачи

### Task 6: Разбор сценария

**Files:**
- Create: `engine/src/script.rs`
- Modify: `engine/Cargo.toml`, `Cargo.toml`, `engine/src/lib.rs`

- [ ] тест: разбор полного примера, значения по умолчанию
- [ ] тест ошибок с номером записи: неизвестное поле, два действия в записи, ни одного действия, неизвестный `KeyCode`, `tick = 0`, `tick > ticks`, убывающие тики, снимок вне диапазона, `ticks = 0`, нулевой размер, `scale <= 0`
- [ ] тест: последний тик всегда в снимках, снимки упорядочены без повторов
- [ ] `Script::parse(&str) -> Result<Script, ScriptError>`, `Script::events_at(tick)`
- [ ] фича `serde` у `winit`, `serde` и `toml` в `engine`

### Task 7: Seed, Game::report и строки кадра

**Files:**
- Modify: `engine/src/frame.rs`, `engine/src/lib.rs`, `engine/Cargo.toml`, `Cargo.toml`, `games/asteroids/src/main.rs`

- [ ] тест: `Gfx::draw_text` пишет строку при `record_text` и не пишет без него; `step_frame` очищает строки
- [ ] тест: `Game::report` по умолчанию — пустой объект
- [ ] `Context::seed: u64`; вне сценария app ставит его от `SystemTime` до `G::init`
- [ ] Asteroids берёт `ctx.seed`; импорт времени из игры уходит
- [ ] `Gfx::record_text`, `Gfx::texts`; в обычном кадре выключено, кадр не аллоцирует лишнего
- [ ] проверить обход `HashMap` и `HashSet` в логике `asteroids` и `racing`; там, где порядок влияет на исход, — `BTreeMap` или сортировка

### Task 8: Runner и run_script

**Files:**
- Create: `engine/src/runner.rs`
- Modify: `engine/src/lib.rs`

- [ ] тест без GPU: сборка `RawInput`: `screen_rect` в точках, время от тика, координаты указателя делятся на `scale` один раз
- [ ] тест без GPU: перевод каждого `InputEvent` в `egui::Event`
- [ ] офскрин-тест на игрушечной игре: тик 0 без `update`, ровно `ticks` вызовов `update`, снимок N видит состояние после N-го `update`
- [ ] офскрин-тест: клик по кнопке панели egui не доходит до `Input`, отпускание доходит, клик мимо панели доходит
- [ ] офскрин-тест: `cursor_world` и `cursor_ray` в тике N идут по кадру N−1
- [ ] офскрин-тест: `report` не объект — `RunError::Report` с тиком; нет адаптера — `RunError::NoAdapter`
- [ ] офскрин-тест: `tick_hz = 1` даёт ровно `ticks` вызовов `update`
- [ ] офскрин-тест: размер сценария больше лимита текстур — `RunError::Size`, без паники
- [ ] офскрин-тест начального состояния: `motion` работает без события `focus` после захвата, `focused` равен в `Context` и `RawInput`; `motion` в тике клика захвата отброшен
- [ ] `Runner<G>::new(&gpu, ...)`, `step`, `snapshot`, `finish`; часы и начальное состояние — как в разделе «Смысл тиков»; один шаг — один виртуальный кадр через `step_visual`, `VirtualGrab`, `FrameRenderer`, запись `shown`
- [ ] `RunReport`, `Snapshot`, `RunError`, `run_script`; `reload_changed` не вызывается, `ctx.audio` немой
- [ ] `docs/design.md`: новый раздел о runner и смысле тиков; молчаливые поломки «→ что ловит»: событие вне диапазона, `shown` не обновлён, альфа в PNG, отправленная строка не равна видимой, недетерминизм игры

### Task 9: Командная строка и артефакты

**Files:**
- Create: `engine/src/cli.rs`
- Modify: `engine/src/lib.rs`, `games/*/src/main.rs`, `README.md`

- [ ] тест разбора аргументов: нет `--script`, полный набор, `--script` без пути, `--out` и `--window` без `--script`, чужой аргумент игнорируется
- [ ] офскрин-тест: в каталоге `--out` появляются `report.json` и PNG всех снимков, повторный запуск перезаписывает
- [ ] `run` возвращает `RunError`, при `--script` идёт в `run_script`
- [ ] `main` восьми игр и пример в README: `Result<(), stapel::RunError>`
- [ ] после успешного прогона — одна строка с путём `report.json` в stdout; ошибка — ненулевой код
- [ ] проверка запуском: `cargo run -p clear-color -- --script <сценарий>`

### Task 10: Режим --window

**Files:**
- Modify: `engine/src/app.rs`, `engine/src/renderer.rs`, `engine/src/lib.rs`

- [ ] `ScriptApp<G>`: на каждом `RedrawRequested` один `Runner::step`, затем вывод `Canvas` на поверхность
- [ ] `Gpu` создаётся `Renderer` под поверхность и передаётся в `Runner`
- [ ] `Renderer` принимает внешний `Canvas`, размер которого задан сценарием; поверхность своего размера
- [ ] события окна, кроме `CloseRequested` и resize поверхности, не доходят до игры
- [ ] после последнего тика `report.json` и PNG пишутся так же, окно закрывается, код возврата 0; закрытие раньше — ошибка
- [ ] тест покрывает общую часть в `Runner`; склейка остаётся в `app.rs` вне покрытия
- [ ] проверка запуском на `clear-color`

### Task 11: Сквозная проверка на trains

**Files:**
- Create: `games/trains/tests/replay.rs`, `games/trains/tests/scripts/switch.toml`
- Modify: `games/trains/src/main.rs`

- [ ] `Trains::report`: положение головы состава, скорость, положения стрелок, открыто ли депо
- [ ] сценарий: клик по кнопке «Ехать» панели депо, газ, клик по стрелке после движения камеры, клик по стрелке под составом для `SWITCH BUSY`
- [ ] тест в `main.rs` через `run_script`: стрелка перевелась, надпись есть в `texts` нужного снимка
- [ ] `replay.rs`, бинарник через `CARGO_BIN_EXE_trains`: два процесса офскрином дают равные `report.json` и равные пиксели; без адаптера — `SKIPPED`, при `REQUIRE_GPU=1` — падение
- [ ] `replay.rs`: `--window` против офскрина: те же равенства; без `DISPLAY` и `WAYLAND_DISPLAY` — `SKIPPED`, при `REQUIRE_DISPLAY=1` — падение; перед сравнением пикселей проверить равенство `adapter` и `backend`
- [ ] сравнение точное; расхождение — ⚠️ с измерением и причиной, допуск только после неё
- [ ] тест отравлением: `HashMap` в логике или недостающий сид ломают первый тест; проверить разовой правкой, не коммитить

### Task 12: Публичный путь для внешнего проекта

**Files:**
- Create: `docs/agents/AGENTS.md`, `docs/friction/README.md`
- Modify: `README.md`, `docs/design.md`, `CLAUDE.md`

- [ ] раздел README: как написать сценарий, как запустить `--script`, что лежит в `--out`, как читать `report.json`, смысл тиков
- [ ] `docs/agents/AGENTS.md`, по-английски, копируется в проект игры: контракт `Game`, ассеты и лицензии, цикл проверки через сценарий, PNG и `cargo test`, правила журнала `FRICTION.md`
- [ ] `docs/friction/README.md`: формат записи и четыре исхода разбора
- [ ] `docs/design.md`: строка этапа 10, исключение из правила «функция только под игру»
- [ ] `CLAUDE.md`: команды `--script`, `REQUIRE_DISPLAY`, слой runner в «Архитектуре»
- [ ] дымовая проверка: `cargo new` в пустом каталоге вне репозитория, зависимость на запушенный rev ветки, игра из README, прогон сценария; пути автора нигде не нужны

### Task 13: Проверка перед холодной сессией
- [ ] `make check`, `REQUIRE_GPU=1 REQUIRE_DISPLAY=1 cargo test -p trains --test replay`
- [ ] ветка запушена, её rev закреплён для холодной сессии

### Task 14: Холодная сессия
- [ ] ⚠️ абзац идеи игры даёт пользователь
- [ ] до запуска зафиксировать наблюдаемые критерии идеи: что должно быть в отчёте и на снимках, включая достижимость цели и проигрыша. Критерии хранятся у нас, сессии не передаются: она доказывает играбельность из одного абзаца
- [ ] запуск новой сессии агента в пустом каталоге вне репозитория: только абзац, адрес репозитория и `AGENTS.md`, без контекста этого плана
- [ ] сессия завершена сама, без подсказок; результат: игра, сценарий, снимки, отчёт, `FRICTION.md`
- [ ] независимая проверка: сценарий прогнан заново из её каталога, PNG просмотрены, отчёт и снимки сверены с критериями. Пустой отчёт и снимок старта критерии не проходят; равенство прогонов доказывает повторяемость, а не выполнение цели
- [ ] журнал скопирован в `docs/friction/<дата>-<игра>.md`, туда же критерии и результат сверки
- [ ] каждая запись разобрана в один из четырёх исходов; правки движка и доков — ➕ задачами сюда, выполняются до задачи 15

### Task 15: Итоговая приёмка
Идёт после разбора журнала и всех его правок: проверки до холодной сессии их не покрывают.
- [ ] все пункты «Готово, когда»
- [ ] `make check`, `make cov`, `make mutants-diff`
- [ ] `REQUIRE_GPU=1 WGPU_BACKEND=vulkan cargo test -p stapel`, `REQUIRE_GPU=1 REQUIRE_DISPLAY=1 cargo test -p trains --test replay`
- [ ] игра холодной сессии пересобрана на итоговом rev, её сценарий зелёный, критерии выполняются
- [ ] ревью ветки через `revmux`

### Task 16: [Final] Документация
- [ ] `README.md` и `CLAUDE.md` соответствуют итогу разбора журнала
- [ ] ссылка на коммит для примера зависимости в README
- [ ] план перенести в `docs/plans/completed/`

## Post-Completion
*Действия вне репозитория, без чекбоксов.*

**Проверка запуском:** настоящий захват курсора в `maze` и `shooter`, ресайз окна на Wayland и X11.

**CI:** окно в CI нет. Для `--window` против офскрина в CI нужен `xvfb` в `ci/Dockerfile`, подъём тега `CI_IMAGE` и ручная сборка образа. До тех пор этот тест — локальный.
