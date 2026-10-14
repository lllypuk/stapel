# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Учебный движок на Rust (`engine/`) и игры-проверки на нём (`games/*`). Платформа — Linux (Wayland, X11), GPU через Vulkan. Решения, их причины и список молчаливых поломок — `docs/design.md`; планы этапов — `docs/plans/` (выполненные — в `completed/`).

## Команды

```sh
cargo test --workspace
cargo test -p engine input::tests::focus_lost_clears_held_keys   # один тест
cargo clippy --workspace --all-targets -- -D warnings
RUST_LOG=info cargo run -p clear-color                            # в логе FPS/TPS раз в секунду
NO_VSYNC=1 RUST_LOG=info cargo run -p clear-color
```

## Архитектура

- Движок — фреймворк: `winit` владеет циклом, `engine::run::<G>(Config)` запускает `app::App`, игра реализует трейт `Game { init, update, draw(alpha) }`.
- Кадр: `RedrawRequested` → `step_frame` (N фиксированных тиков `update` по `FixedClock`, после каждого — `Input::end_tick`, затем один `draw(alpha)`) → `Renderer::render(&ctx.gfx)` → в конце кадра `request_redraw` следующего. Цикл в `ControlFlow::Wait`; `Poll` не возвращать — на Wayland с vsync он крутит ядро на 100%.
- Граница тестируемости: всё, что не требует окна и GPU (`clock`, `input`, `stats`, `step_frame`, чистые функции `renderer`), — юнит-тесты в `#[cfg(test)]` того же файла. `app.rs` и GPU-вызовы — тонкая склейка, проверяется запуском игры.
- Рисование immediate-mode: игра каждый кадр заполняет `ctx.gfx`, `Renderer` об играх и будущем ECS не знает.
- `Color` линейный; поверхность `*Srgb`. Цвета из sRGB — через `Color::from_srgb8`.

## Правила

- Функция появляется в движке только когда она нужна игре текущего этапа (`docs/design.md`, таблица «Этапы»). Изменение решения из дизайна — вместе с правкой `docs/design.md`.
- Ревью веток — `revmux`; профиль проекта для него — `.revmux/profile.md`, архив раундов `.revmux/tasks/` в git не попадает.
