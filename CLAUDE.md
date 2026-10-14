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
make asteroids                                                    # любая игра из games/, release; make test, make lint
NO_VSYNC=1 RUST_LOG=info cargo run -p clear-color
make check                                                        # fmt, typos, clippy, doc, machete, deny, тесты; deny качает advisory-db — нужна сеть
make cov                                                          # покрытие строк с порогом
make mutants                                                      # мутанты, ~8 мин; исключения — .cargo/mutants.toml
REQUIRE_GPU=1 WGPU_BACKEND=vulkan cargo test -p engine sprite_pipeline   # офскрин-тест без адаптера падает, а не пропускается; без WGPU_BACKEND найдётся адаптер GL
```

## Архитектура

- Движок — фреймворк: `winit` владеет циклом, `engine::run::<G>(Config)` запускает `app::App`, игра реализует трейт `Game { init, update, draw(alpha) }`.
- Кадр: `RedrawRequested` → `step_frame` (N фиксированных тиков `update` по `FixedClock`, после каждого — `Input::end_tick`, затем один `draw(alpha)`) → `Renderer::render(&ctx.gfx, &mut ctx.assets)` (заливка новых текстур, затем `SpritePipeline::draw` во вьюпорт `letterbox`) → в конце кадра `request_redraw` следующего. Цикл в `ControlFlow::Wait`; `Poll` не возвращать — на Wayland с vsync он крутит ядро на 100%.
- Граница тестируемости: всё, что не требует окна и GPU (`clock`, `input`, `stats`, `step_frame`, `camera`, `assets`, `text`, `ecs`, `batch`, `surface` (чистая логика поверхности для `renderer`), тик игры), — юнит-тесты в `#[cfg(test)]` того же файла. GPU-путь спрайтов — офскрин-тест `SpritePipeline` на реальном адаптере (пропуск только без адаптера, с `SKIPPED`). `app.rs` и работа с поверхностью — тонкая склейка, проверяется запуском игры.
- Рисование immediate-mode: игра каждый кадр заполняет `ctx.gfx` (`draw_sprite`, `draw_text`), текстуры и шрифты грузит один раз в `init` через `ctx.assets` — получает `Handle<Texture>`/`Handle<Font>`, повторная загрузка того же пути отдаёт тот же хэндл. `Renderer` об играх и ECS не знает.
- ECS (`engine::ecs`) — библиотека: `World`, компоненты и системы принадлежат игре (`World` в структуре игры, не в `Context`). Удаление и создание во время обхода — через `Commands`, применяются `apply` в конце тика.
- `Color` линейный; поверхность `*Srgb`. Цвета из sRGB — через `Color::from_srgb8`.

## Правила

- Функция появляется в движке только когда она нужна игре текущего этапа (`docs/design.md`, таблица «Этапы»). Изменение решения из дизайна — вместе с правкой `docs/design.md`.
- Исключение из линта — локальный `#[expect(lint, reason = "...")]`, не `allow`; если линт срабатывает по-разному в тестовой сборке (`unwrap_used`, `print_stderr`, `dead_code`) — `#[cfg_attr(not(test), expect(..))]`, иначе `--all-targets` упадёт на `unfulfilled_lint_expectations`.
- Порог покрытия в `make cov` — только вверх; снижение — осознанное решение в коммите.
- Выживший мутант — тест, который его убивает, или исключение в `.cargo/mutants.toml` с причиной.
- Тег `CI_IMAGE` в `.gitlab-ci.yml` поднимается вместе с правкой `ci/Dockerfile`; образ собирается и пушится вручную.
- Ревью веток — `revmux`; профиль проекта для него — `.revmux/profile.md`, архив раундов `.revmux/tasks/` в git не попадает.
