# game-engine

Учебный движок на Rust: сначала 2D, потом 3D. Функции движка появляются вместе с играми в `games/`, которым они нужны. Решения и структура — в [docs/design.md](docs/design.md).

## Запуск

```sh
cargo test --workspace
make platformer                                   # платформер: A/D или ←/→ — бег, Space/W — прыжок, Enter — следующий уровень после финиша, F1 — редактор
make racing                                       # гонки на время: WASD или стрелки, Space — ручник, R — на чекпоинт, Tab до старта — машина; TRACK=track02 — другая трасса
make maze                                         # лабиринт от первого лица: WASD — ходьба, мышь — обзор, клик — захват мыши, Esc — отпустить, Enter — заново после выхода, F1 — панель света и тумана
make shooter                                      # шутер в лесу: WASD — ход, мышь — обзор, клик — захват мыши, Esc — отпустить, ЛКМ — огонь, 1/2 — оружие, Enter — заново после смерти или выхода
make trains                                       # поезда в изометрии: W/S или ↑/↓ — вперёд/назад, E — стрелка впереди, ЛКМ — стрелка под курсором, Tab — депо
make asteroids                                    # Asteroids: A/D — поворот, W — тяга, Space — огонь, Enter — рестарт
RUST_LOG=info cargo run -p pong                   # Pong на двоих: W/S — левая ракетка, O/L — правая
RUST_LOG=info cargo run -p clear-color            # Space меняет цвет фона
NO_VSYNC=1 RUST_LOG=info cargo run -p clear-color # без vsync
```

С `RUST_LOG=info` движок раз в секунду пишет в лог FPS, TPS, худший кадр и число спрайтов, а при открытом звуковом устройстве — время аудиоколбэка против бюджета и число отброшенных команд (`audio cb 0.4/10.7ms dropped 0`). `make <игра>` — release-сборка.

Платформа — Linux (Wayland, X11), нужен Vulkan. Для сборки нужны `alsa-lib` и `pkg-config` (в Debian — `libasound2-dev pkg-config`): их требует `cpal`.

Уровни и редактор уровней платформера (F1) — [games/platformer/README.md](games/platformer/README.md); формат карты лабиринта — [games/maze/README.md](games/maze/README.md); трассы и машины гонок — [games/racing/README.md](games/racing/README.md); карта и враги шутера — [games/shooter/README.md](games/shooter/README.md); карта и депо поездов — [games/trains/README.md](games/trains/README.md).

Ассеты платформера — CC0 (Kenney, Juhani Junkala), источники и команды конвертации — в [games/platformer/assets/CREDITS.md](games/platformer/assets/CREDITS.md); ассеты `maze` — CC0 (Kenney Mini Dungeon), [games/maze/assets/CREDITS.md](games/maze/assets/CREDITS.md); ассеты `racing` — CC0 (Kenney Racing Kit, Car Kit), [games/racing/assets/CREDITS.md](games/racing/assets/CREDITS.md); ассеты `shooter` — CC0 (Kenney Mini Forest, Blaster Kit, Blocky Characters, Mini Dungeon, звуки Kenney), [games/shooter/assets/CREDITS.md](games/shooter/assets/CREDITS.md); ассеты `trains` — CC0 (Kenney Train Kit, 3D Road Tiles, City Kit Commercial), [games/trains/assets/CREDITS.md](games/trains/assets/CREDITS.md).

## Проверки

Версия Rust и компоненты закреплены в `rust-toolchain.toml`, rustup поставит их сам. Инструменты — в `~/.cargo/bin` (Makefile добавляет его в `PATH`), версии те же, что в `ci/Dockerfile`:

```sh
cargo install cargo-binstall
cargo binstall --locked cargo-llvm-cov@0.9.1 cargo-mutants@27.1.0 cargo-deny@0.20.2 cargo-machete@0.9.2 typos-cli@1.51.1
make check    # линты, документация, зависимости, тесты
make cov      # покрытие с порогом
make mutants  # мутационное тестирование
```

CI — `.gitlab-ci.yml`: на каждый push линты и `make cov` (он же прогон тестов, офскрин-тесты 2D и 3D — на lavapipe), мутанты — по расписанию раз в неделю.
