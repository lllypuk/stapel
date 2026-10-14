# game-engine

Учебный движок на Rust: сначала 2D, потом 3D. Функции движка появляются вместе с играми в `games/`, которым они нужны. Решения и структура — в [docs/design.md](docs/design.md).

## Запуск

```sh
cargo test --workspace
make asteroids                                    # Asteroids: A/D — поворот, W — тяга, Space — огонь, Enter — рестарт
RUST_LOG=info cargo run -p pong                   # Pong на двоих: W/S — левая ракетка, O/L — правая
RUST_LOG=info cargo run -p clear-color            # Space меняет цвет фона
NO_VSYNC=1 RUST_LOG=info cargo run -p clear-color # без vsync
```

С `RUST_LOG=info` движок раз в секунду пишет в лог FPS, TPS, худший кадр и число спрайтов. `make <игра>` — release-сборка.

Платформа — Linux (Wayland, X11), нужен Vulkan.
