# game-engine

Учебный движок на Rust: сначала 2D, потом 3D. Функции движка появляются вместе с играми в `games/`, которым они нужны. Решения и структура — в [docs/design.md](docs/design.md).

## Запуск

```sh
cargo test --workspace
RUST_LOG=info cargo run -p clear-color            # Space меняет цвет фона, в логе FPS и TPS
NO_VSYNC=1 RUST_LOG=info cargo run -p clear-color # без vsync
```

Платформа — Linux (Wayland, X11), нужен Vulkan.
