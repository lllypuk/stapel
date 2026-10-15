# game-engine

Учебный движок на Rust: сначала 2D, потом 3D. Функции движка появляются вместе с играми в `games/`, которым они нужны. Решения и структура — в [docs/design.md](docs/design.md).

## Запуск

```sh
cargo test --workspace
make platformer                                   # платформер: A/D или ←/→ — бег, Space/W — прыжок, Enter — заново после финиша
make asteroids                                    # Asteroids: A/D — поворот, W — тяга, Space — огонь, Enter — рестарт
RUST_LOG=info cargo run -p pong                   # Pong на двоих: W/S — левая ракетка, O/L — правая
RUST_LOG=info cargo run -p clear-color            # Space меняет цвет фона
NO_VSYNC=1 RUST_LOG=info cargo run -p clear-color # без vsync
```

С `RUST_LOG=info` движок раз в секунду пишет в лог FPS, TPS, худший кадр и число спрайтов, а при открытом звуковом устройстве — время аудиоколбэка против бюджета и число отброшенных команд (`audio cb 0.4/10.7ms dropped 0`). `make <игра>` — release-сборка.

Платформа — Linux (Wayland, X11), нужен Vulkan. Для сборки нужны `alsa-lib` и `pkg-config` (в Debian — `libasound2-dev pkg-config`): их требует `cpal`.

Ассеты платформера — CC0 (Kenney, Juhani Junkala), источники и команды конвертации — в [games/platformer/assets/CREDITS.md](games/platformer/assets/CREDITS.md).

## Проверки

Версия Rust и компоненты закреплены в `rust-toolchain.toml`, rustup поставит их сам. Инструменты — в `~/.cargo/bin` (Makefile добавляет его в `PATH`), версии те же, что в `ci/Dockerfile`:

```sh
cargo install cargo-binstall
cargo binstall --locked cargo-llvm-cov@0.9.1 cargo-mutants@27.1.0 cargo-deny@0.20.2 cargo-machete@0.9.2 typos-cli@1.51.1
make check    # линты, документация, зависимости, тесты
make cov      # покрытие с порогом
make mutants  # мутационное тестирование
```

CI — `.gitlab-ci.yml`: на каждый push линты и `make cov` (он же прогон тестов, офскрин-тест — на lavapipe), мутанты — по расписанию раз в неделю.
