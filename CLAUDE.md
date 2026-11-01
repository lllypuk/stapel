# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Учебный движок на Rust (`engine/`, крейт `stapel`) и игры-проверки на нём (`games/*`). Платформа — Linux (Wayland, X11), GPU через Vulkan. Решения, их причины и список молчаливых поломок — `docs/design.md`; планы этапов — `docs/plans/` (выполненные — в `completed/`); источники ассетов и их лицензии — `docs/assets.md`. Основной remote — GitHub (`origin`, `github.com/lllypuk/stapel`); GitLab (`gitlab`) — зеркало с CI, `git push origin` пушит в оба.

## Команды

```sh
cargo test --workspace
cargo test -p stapel input::tests::focus_lost_clears_held_keys   # один тест
cargo clippy --workspace --all-targets -- -D warnings
RUST_LOG=info cargo run -p clear-color                            # движок пишет FPS/TPS/worst/sprites раз в секунду
RUST_LOG=info cargo run -p pong
RUST_LOG=info cargo run -p platformer                             # F1 — редактор уровней из games/platformer/assets/levels/
make racing                                                       # гонки: старт в гараже, F1 — редактор трасс, F2 — панель характеристик; TRACK=track02 — другая трасса из games/racing/assets/tracks/, правка трассы видна на лету
make maze                                                         # 3D-лабиринт: клик — захват мыши, Esc отпускает, F1 — панель
make shooter                                                      # шутер: клик — захват мыши, Esc отпускает, 1/2 — оружие, Enter — заново после смерти или выхода
make trains                                                       # поезда: старт в депо, Tab — депо, W/S — вперёд/назад, E — стрелка впереди, ЛКМ — стрелка под курсором
make asteroids                                                    # любая игра из games/, release; make test, make lint
NO_VSYNC=1 RUST_LOG=info cargo run -p clear-color
make check                                                        # fmt, typos, clippy, doc, machete, deny, тесты; deny качает advisory-db — нужна сеть
make cov                                                          # покрытие строк с порогом
make mutants                                                      # мутанты вполсилы под nice, ~1 ч; исключения — .cargo/mutants.toml
make mutants-diff                                                 # мутанты только по строкам ветки против main (BASE=...); CI гоняет его на push ветки с правкой *.rs, отчёт — артефакт mutants.out/
REQUIRE_GPU=1 WGPU_BACKEND=vulkan cargo test -p stapel   # офскрин-тесты 2D и 3D без адаптера падают, а не пропускаются; без WGPU_BACKEND найдётся адаптер GL
cargo run -p trains -- --script games/trains/tests/scripts/switch.toml --out target/run   # прогон сценария без окна: PNG снимков и report.json; --window — тот же сценарий в окне; у любой игры
REQUIRE_GPU=1 REQUIRE_DISPLAY=1 WGPU_BACKEND=vulkan cargo test -p trains --test replay -- --include-ignored   # два процесса офскрином и окно против офскрина; окно под #[ignore] (make test и make cov его включают); без адаптера или дисплея — SKIPPED, с переменными — падение, в том числе Stalled; в CI дисплея нет, окно и app.rs проверяются локально
```

## Архитектура

- Движок — фреймворк: `winit` владеет циклом, `stapel::run::<G>(Config)` запускает `app::App`, игра реализует трейт `Game { init, update, draw(alpha), ui, report }`; `run` возвращает `Result<(), RunError>`.
- Кадр: `RedrawRequested` → раз в 0,5 с `Assets::reload_changed` → `step_visual`: egui `begin_pass` → `step_frame` (N фиксированных тиков `update` по `FixedClock`, после каждого — `Input::end_tick`, затем один `draw(alpha)`) → `CursorGrab::sync` по `ctx.cursor_grab` (`WindowGrab` в окне, `VirtualGrab` без него) → `Game::ui(&egui::Context)` → снять фокус egui → `end_pass` → `tessellate` → `Renderer::render` (кадр рисуется в `Canvas`, затем `PresentPass` выводит его на поверхность; заливка новых и изменённых текстур, мешей и текстур egui; при `gfx.scene` — `MeshPipeline::draw_scene`: проход мира с `Clear` цвета и глубины на всю поверхность, затем слой `Scene::viewmodel` с `Load` цвета и `Clear` глубины; затем `SpritePipeline::draw` с `Load` после 3D во вьюпорт `viewport`: `letterbox_pixel` при `Config::pixel_perfect`, иначе `letterbox`; потом egui-проход с `LoadOp::Load` на всю поверхность) → после показа — `ctx.shown` (камера и вьюпорт показанного кадра, 3D-камера и размер поверхности) → в конце кадра `request_redraw` следующего. Цикл в `ControlFlow::Wait`; `Poll` не возвращать — на Wayland с vsync он крутит ядро на 100%.
- Граница тестируемости: всё, что не требует окна и GPU (`clock`, `input`, `stats`, `step_frame`, `step_visual`, `camera` (`Camera2D`, `Camera3D`), `ui` (маршрутизация событий окна в `Input`), `assets`, `audio` (декод, микшер и `fill` — заполнение буфера устройства), `text`, `ecs`, `physics`, `physics3d`, `watch`, `batch`, `surface` (чистая логика поверхности для `renderer`), `scene` (раскрытие и свет), `model` (разбор glTF и поза), тик игры), — юнит-тесты в `#[cfg(test)]` того же файла. GPU-путь — офскрин-тесты `SpritePipeline` и `MeshPipeline` на хелперах `gpu_test` и реальном адаптере (пропуск только без адаптера, с `SKIPPED`). `app.rs`, работа с поверхностью и `audio/device.rs` (устройство `cpal`) — тонкая склейка, проверяется запуском игры.
- Runner (`stapel::runner`, `script`, `canvas`, `frame_renderer`, `present`, `gpu`): тот же `step_visual`, что в окне, но вход — сценарий TOML по тикам (`Script`), события идут в `Context` и `RawInput` egui, часы `FixedClock` с одним `update` на тик, цель — `Canvas` (`*Srgb`), из неё PNG и `report.json` (`RunReport`, `Game::report`). `--window` показывает тот же `Canvas` через `PresentPass`; окно и офскрин равны побайтно, перевод событий winit это не доказывает. `run` читает `--script`, `--out`, `--window`, `--no-png` (отчёт без PNG). `Gfx::record_text` (включает runner) копит строки `draw_text` в `Gfx::texts`, в обычном кадре выключено; символ вне ASCII 32..=126 рисуется `?` с `warn` один раз, в `texts` остаётся исходная строка. Детерминизм: случайность только от `ctx.seed` (вне сценария app ставит его от `SystemTime`), без `HashMap`-обхода в логике; нарушение молча ломает равенство прогонов (→ `replay.rs`). `run_script(.., None)` — без PNG и чтения с GPU, для подбора тиков. `Gpu::headless` создаёт экземпляр под замком: параллельные Vulkan-экземпляры роняют загрузчик. `Stalled` — 10 с без прогресса в `--window`, причина не установлена. Формат сценария и смысл тиков — `README.md`; инструкция агенту, пишущему игру, — `docs/agents/AGENTS.md`, журнал его трения — `docs/friction/`.
- Рисование immediate-mode: игра каждый кадр заполняет `ctx.gfx` (`draw_sprite`, `draw_text`), текстуры и шрифты грузит один раз в `init` через `ctx.assets` — получает `Handle<Texture>`/`Handle<Font>`, повторная загрузка того же пути отдаёт тот же хэндл. `Renderer` об играх и ECS не знает.
- 3D: игра в `draw` ставит `gfx.scene = Some(Scene { camera: Camera3D, ambient, fog, sun, lights, meshes, models, viewmodel })`, `step_frame` перед `draw` сбрасывает её в `None`. Солнце — `DirectionalLight`, Ламберт без теней. Модель — `ctx.assets.load_model` (GLB) → `Handle<Model>`; цвет материала (`baseColorFactor`, линейный) умножает текстуру. Поза — `Model::pose(anim, t, looped, &mut Pose)` на CPU, ручная — `Pose::local_mut(node)` и `Model::recompose`; `ModelDraw { model, transform, pose, tint }`, `tint.a < 1` — дизеринг без сортировки; скиннинг — на GPU. Ортокамера — `Camera3D::ortho = Some(высота кадра в мире)`, слой оружия всегда перспективный. Габариты — `Node::bounds`, `Model::bounds`; треугольники меша на CPU — `Assets::mesh_geometry`. Модели горячо не перезагружаются. Оружие в руках — `viewmodel: Some(Viewmodel { fov_y, near, models })`: модели в мировых координатах, камера — сцены со своими `fov_y` и `near`.
- Ввод: `app.rs` переводит `WindowEvent` в `InputEvent`, `Context::apply` применяет его; клавиатура и мышь — снимок `ctx.input` в `update` (`mouse_just_pressed`, `mouse_down`, `cursor`, `mouse_delta`, `wheel` — строки за тик). Обзор мышью: игра ставит `ctx.cursor_grab`, app захватывает и пишет `ctx.grabbed`; `mouse_delta` копится только при фокусе и захвате, `update` его потребляет, `draw` добавляет непотреблённый к камере. Курсор в мир — `ctx.cursor_world()` по камере последнего показанного кадра, не по `gfx.camera`; луч в 3D — `ctx.cursor_ray()` по 3D-камере показанного кадра и всей поверхности. Нажатие кнопки мыши и колесо над панелью egui в `Input` не попадают, отпускание — всегда. Панель кладёт команды в очередь игры, разбирает её `update`.
- Горячая перезагрузка: изменённые на диске текстура или звук подменяются под прежним хэндлом; ошибка декода — прежний ресурс и `warn`. Файлы игры (уровень, трасса) — `stapel::Watcher` в `update`: раз в `WATCH_TICKS` перечитывает файл и отдаёт текст, если он изменился по содержимому.
- Звук: `ctx.audio.play`/`play_music` шлют команды через очередь `rtrb` микшеру аудиопотока; `Context::default` — немой `Audio`. Эффект в пространстве — `play_stereo(sound, Stereo::at(&camera, pos, volume, range))`: файл сводится в моно, панорама по `yaw`. Длящийся голос — `start_loop` → `Option<LoopId>` (`None` — нет слота или очередь полна, повторить позже), `set_loop(id, rate, volume)` на ходу, `stop_loop`; `Stop` и последний `Set` при полной очереди досылаются, app зовёт `Audio::flush` раз в кадр.
- Физика 3D (`stapel::physics3d`) — библиотека: `RigidBody`, `StaticWorld` (треугольники с `tag` игры, `raycast`, односторонние `sphere_contacts`), `solve_contacts`. Подвеска, шины и привод — в игре (`games/racing`).
- ECS (`stapel::ecs`) — библиотека: `World`, компоненты и системы принадлежат игре (`World` в структуре игры, не в `Context`). Удаление и создание во время обхода — через `Commands`, применяются `apply` в конце тика.
- `Color` линейный; поверхность `*Srgb`. Цвета из sRGB — через `Color::from_srgb8`.

## Правила

- Функция появляется в движке только когда она нужна игре текущего этапа (`docs/design.md`, таблица «Этапы»). Изменение решения из дизайна — вместе с правкой `docs/design.md`.
- Исключение из линта — локальный `#[expect(lint, reason = "...")]`, не `allow`; если линт срабатывает по-разному в тестовой сборке (`unwrap_used`, `print_stderr`, `dead_code`) — `#[cfg_attr(not(test), expect(..))]`, иначе `--all-targets` упадёт на `unfulfilled_lint_expectations`.
- Долгий тест — под `#[ignore = "..."]`: его включают `make test` и `make cov`, а `cargo test --workspace` и мутанты — нет.
- Порог покрытия в `make cov` — только вверх; снижение — осознанное решение в коммите.
- Выживший мутант — тест, который его убивает, или исключение в `.cargo/mutants.toml` с причиной.
- Приёмка этапа — `mutants-diff`: задача CI на push ветки (раннер home-server) или локально; полный `make mutants` — если ветка удаляет или ослабляет старые тесты, иначе его гоняет CI по расписанию.
- Новая молчаливая поломка — в свой слой раздела `docs/design.md` с пометкой «→ что её ловит»; правка кода, снимающая защиту, правит и пометку.
- Изменение контракта `Game`, CLI или формата сценария — вместе с `README.md` и `docs/agents/AGENTS.md`; записи трения разбираются по `docs/friction/README.md`.
- Тег `CI_IMAGE` в `.gitlab-ci.yml` поднимается вместе с правкой `ci/Dockerfile`; образ собирается и пушится вручную.
- Ревью веток — `revmux`; профиль проекта для него — `.revmux/profile.md`, архив раундов `.revmux/tasks/` в git не попадает.
