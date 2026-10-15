# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Учебный движок на Rust (`engine/`) и игры-проверки на нём (`games/*`). Платформа — Linux (Wayland, X11), GPU через Vulkan. Решения, их причины и список молчаливых поломок — `docs/design.md`; планы этапов — `docs/plans/` (выполненные — в `completed/`).

## Команды

```sh
cargo test --workspace
cargo test -p engine input::tests::focus_lost_clears_held_keys   # один тест
cargo clippy --workspace --all-targets -- -D warnings
RUST_LOG=info cargo run -p clear-color                            # движок пишет FPS/TPS/worst/sprites раз в секунду
RUST_LOG=info cargo run -p pong
RUST_LOG=info cargo run -p platformer                             # F1 — редактор уровней из games/platformer/assets/levels/
make maze                                                         # 3D-лабиринт: клик — захват мыши, Esc отпускает, F1 — панель
make asteroids                                                    # любая игра из games/, release; make test, make lint
NO_VSYNC=1 RUST_LOG=info cargo run -p clear-color
make check                                                        # fmt, typos, clippy, doc, machete, deny, тесты; deny качает advisory-db — нужна сеть
make cov                                                          # покрытие строк с порогом
make mutants                                                      # мутанты, ~17 мин; исключения — .cargo/mutants.toml
REQUIRE_GPU=1 WGPU_BACKEND=vulkan cargo test -p engine   # офскрин-тесты 2D и 3D без адаптера падают, а не пропускаются; без WGPU_BACKEND найдётся адаптер GL
```

## Архитектура

- Движок — фреймворк: `winit` владеет циклом, `engine::run::<G>(Config)` запускает `app::App`, игра реализует трейт `Game { init, update, draw(alpha), ui }`.
- Кадр: `RedrawRequested` → раз в 0,5 с `Assets::reload_changed` → egui `begin_pass` → `step_frame` (N фиксированных тиков `update` по `FixedClock`, после каждого — `Input::end_tick`, затем один `draw(alpha)`) → захват курсора по `ctx.cursor_grab` → `Game::ui(&egui::Context)` → снять фокус egui → `end_pass` → `Renderer::render` (заливка новых и изменённых текстур, мешей и текстур egui; при `gfx.scene` — 3D-проход `MeshPipeline` с `Clear` цвета и глубины на всю поверхность; затем `SpritePipeline::draw` с `Load` после 3D во вьюпорт `viewport`: `letterbox_pixel` при `Config::pixel_perfect`, иначе `letterbox`; потом egui-проход с `LoadOp::Load` на всю поверхность) → после показа — `ctx.shown` (камера и вьюпорт показанного кадра) → в конце кадра `request_redraw` следующего. Цикл в `ControlFlow::Wait`; `Poll` не возвращать — на Wayland с vsync он крутит ядро на 100%.
- Граница тестируемости: всё, что не требует окна и GPU (`clock`, `input`, `stats`, `step_frame`, `camera` (`Camera2D`, `Camera3D`), `ui` (маршрутизация событий окна в `Input`), `assets`, `audio` (декод, микшер и `fill` — заполнение буфера устройства), `text`, `ecs`, `physics`, `batch`, `surface` (чистая логика поверхности для `renderer`), `scene` (раскрытие и свет), `model` (разбор glTF и поза), тик игры), — юнит-тесты в `#[cfg(test)]` того же файла. GPU-путь — офскрин-тесты `SpritePipeline` и `MeshPipeline` на хелперах `gpu_test` и реальном адаптере (пропуск только без адаптера, с `SKIPPED`). `app.rs`, работа с поверхностью и `audio/device.rs` (устройство `cpal`) — тонкая склейка, проверяется запуском игры.
- Рисование immediate-mode: игра каждый кадр заполняет `ctx.gfx` (`draw_sprite`, `draw_text`), текстуры и шрифты грузит один раз в `init` через `ctx.assets` — получает `Handle<Texture>`/`Handle<Font>`, повторная загрузка того же пути отдаёт тот же хэндл. `Renderer` об играх и ECS не знает.
- 3D: игра в `draw` ставит `gfx.scene = Some(Scene { camera: Camera3D, ambient, fog, lights, meshes, models })`, `step_frame` перед `draw` сбрасывает её в `None`. Модель — `ctx.assets.load_model` (GLB) → `Handle<Model>`; поза — `Model::pose(anim, t, looped, &mut Pose)` на CPU, `ModelDraw { model, transform, pose }`; скиннинг — на GPU. Модели горячо не перезагружаются.
- Ввод: клавиатура и мышь — снимок `ctx.input` в `update` (`mouse_just_pressed`, `mouse_down`, `cursor`, `mouse_delta`). Обзор мышью: игра ставит `ctx.cursor_grab`, app захватывает и пишет `ctx.grabbed`; `mouse_delta` копится только при фокусе и захвате, `update` его потребляет, `draw` добавляет непотреблённый к камере. Курсор в мир — `ctx.cursor_world()` по камере последнего показанного кадра, не по `gfx.camera`. Нажатие кнопки мыши над панелью egui в `Input` не попадает, отпускание — всегда. Панель кладёт команды в очередь игры, разбирает её `update`.
- Горячая перезагрузка: изменённые на диске текстура или звук подменяются под прежним хэндлом; ошибка декода — прежний ресурс и `warn`.
- Звук: `ctx.audio.play`/`play_music` шлют команды через очередь `rtrb` микшеру аудиопотока; `Context::default` — немой `Audio`.
- ECS (`engine::ecs`) — библиотека: `World`, компоненты и системы принадлежат игре (`World` в структуре игры, не в `Context`). Удаление и создание во время обхода — через `Commands`, применяются `apply` в конце тика.
- `Color` линейный; поверхность `*Srgb`. Цвета из sRGB — через `Color::from_srgb8`.

## Правила

- Функция появляется в движке только когда она нужна игре текущего этапа (`docs/design.md`, таблица «Этапы»). Изменение решения из дизайна — вместе с правкой `docs/design.md`.
- Исключение из линта — локальный `#[expect(lint, reason = "...")]`, не `allow`; если линт срабатывает по-разному в тестовой сборке (`unwrap_used`, `print_stderr`, `dead_code`) — `#[cfg_attr(not(test), expect(..))]`, иначе `--all-targets` упадёт на `unfulfilled_lint_expectations`.
- Порог покрытия в `make cov` — только вверх; снижение — осознанное решение в коммите.
- Выживший мутант — тест, который его убивает, или исключение в `.cargo/mutants.toml` с причиной.
- Новая молчаливая поломка — в свой слой раздела `docs/design.md` с пометкой «→ что её ловит»; правка кода, снимающая защиту, правит и пометку.
- Тег `CI_IMAGE` в `.gitlab-ci.yml` поднимается вместе с правкой `ci/Dockerfile`; образ собирается и пушится вручную.
- Ревью веток — `revmux`; профиль проекта для него — `.revmux/profile.md`, архив раундов `.revmux/tasks/` в git не попадает.
