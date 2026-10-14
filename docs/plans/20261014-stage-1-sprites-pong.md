# Этап 1: спрайты на wgpu и Pong

## Overview
- Движок: загрузка PNG-текстур, батчер спрайтов с instancing, ортокамера с фиксированным миром 640×360, вписанным в окно с полями.
- Игра `games/pong`: две ракетки, мяч (PNG с мягким краем), стенки, счёт 7-сегментными цифрами из прямоугольников.
- Готово, когда: Pong играбелен; любой ресайз, включая крайние пропорции, не меняет поле и не ломает игру; офскрин-тест GPU проходит на этой машине, а не пропускается.

## Context (from discovery)
- Этап 0 в `main`: `engine/src/{app,frame,renderer,clock,input,stats}.rs`, `games/clear-color`. `Renderer` умеет только clear pass, `Gfx { clear_color }`.
- Порядок кадра: `step_frame` (N × `update`, затем всегда один `draw`) → `Renderer::render` (`engine/src/app.rs`).
- Версии (crates.io, 2026-10-14): `glam 0.34.1`, `bytemuck 1.25.2`, `image 0.25.10`.
- Решения и план согласованы с codex (тред `stage1-render`, два раунда).

## Development Approach
- **testing approach**: TDD — тест раньше кода для всего, что не требует окна; GPU-путь — офскрин-тестом.
- Следующая задача — только при зелёных `cargo test --workspace` и `cargo clippy --workspace --all-targets -- -D warnings`.
- Решение, меняющее `docs/design.md` или `CLAUDE.md`, правится в той же задаче, а не в конце.
- Изменения скоупа — сразу в этот файл: ➕ новая задача, ⚠️ блокер.

## Testing Strategy
- Юнит-тесты в `#[cfg(test)]` того же файла: камера, letterbox, премультипликация, загрузка PNG, батчинг, рост буфера, логика и коллизии Pong, сегменты цифр.
- Офскрин-тест `SpritePipeline`: рендер в `Rgba8UnormSrgb`-текстуру на реальном адаптере и readback. Пропуск — только при отсутствии адаптера (`eprintln!("SKIPPED: ...")`); ошибки устройства, шейдера и readback проваливают тест.
- Окно: ресайз, поля, восстановление поверхности — запуском Pong (Task 9).

## Solution Overview
- `Context` остаётся CPU-only: `ctx.assets` декодирует PNG и копит очередь загрузок, `ctx.gfx` копит спрайты. `Renderer::render(&Gfx, &mut Assets)` сначала заливает очередь на GPU (даже при недоступной поверхности), потом рисует.
- `SpritePipeline` — весь GPU-путь спрайтов и **владелец прохода**: `Clear(clear_color)`, `set_viewport`, батчи. `Renderer` только берёт кадр у поверхности, считает вьюпорт и вызывает тот же `draw`, что и офскрин-тест. Так тест проверяет код игры, а не его копию.
- Поворот и произвольные UV у спрайта не делаем: в Pong им нет потребителя.

## Technical Details
- `engine` реэкспортирует `glam::{Vec2, vec2}`: они в публичном API спрайтов и камеры.
- `Camera2D { center: Vec2, size: Vec2 }`, `view_proj()` = `glam::camera::rh::proj::directx::orthographic(l, r, b, t, -1, 1)` (в glam 0.34 `Mat4::orthographic_rh` нет): глубина `z = 0` — 0.5, с запасом от обеих границ wgpu-диапазона `0..1`. Y вверх. `Default` — квадрат 2×2 вокруг нуля, то есть NDC как есть.
- `letterbox(surface: (u32, u32), world_aspect) -> Viewport { x, y, w, h }`: по фактическому `config.width/height` после ограничения лимитом устройства. `LoadOp::Clear` красит весь кадр, а `set_viewport` ограничивает спрайты, поэтому поля получаются цвета `clear_color`.
- `TextureId(u32)` — индекс без поколения: текстуры не выгружаются, индексы не переиспользуются. `TextureId::WHITE = 0` (белая 1×1) резервируется в `Assets::default` и уходит в очередь первой. Загрузка — один раз в `Game::init`, игра хранит `TextureId`.
- `Assets::load_texture(path) -> TextureId`: `image` (только `png`) → RGBA8 → премультипликация **в линейном пространстве**: RGB sRGB → linear → ×α → sRGB, α без гаммы (белый при α = 128 хранится как 188, а не 128). Ошибка чтения или декодирования → пурпурно-чёрная шахматка 8×8 и `log::warn!`. Лимиты устройства на размер текстуры и буфера не проверяются: в Pong одна текстура 10×10 и ~40 спрайтов.
- `Sprite { texture, pos: Vec2 (центр), size: Vec2, color: Color, layer: i32 }`, `Gfx { clear_color, camera, sprites }`, `Gfx::draw_sprite`. `step_frame` очищает `sprites` прямо перед `draw` — иначе спрайты копятся на кадрах без рендера; `clear_color` и `camera` сохраняются.
- `batch(&[Sprite]) -> (Vec<Instance>, Vec<Batch { texture, range }>)`: стабильная сортировка индексов по `(layer, texture)`, вход не меняется — `render` получает `&Gfx`. Внутри слоя порядок разных текстур не гарантирован: для перекрытий нужен разный `layer`.
- `Instance` (`#[repr(C)]`, `bytemuck::Pod`): `pos [f32; 2]`, `size [f32; 2]`, `color [f32; 4]` — линейный, не премультиплицированный. Шейдер: `rgb = t.rgb * tint.rgb * tint.a`, `a = t.a * tint.a`; блендинг `PREMULTIPLIED_ALPHA_BLENDING`.
- Квад — 6 вершин из `vertex_index`, инстансы — `set_vertex_buffer` со `VertexStepMode::Instance`; диапазон батча — через `draw(0..6, range)`, без второго смещения буфера. Буфер инстансов пересоздаётся **до** `write_buffer`, если мал (запись за край — паника валидации): ёмкость — степень двойки в байтах, `VERTEX | COPY_DST`. Пустой кадр — только `Clear`.
- UV: `(0, 0)` — левый верхний texel, совпадает с левым верхним углом спрайта при Y вверх.
- Текстуры: `Rgba8UnormSrgb`, `Queue::write_texture` с `bytes_per_row = 4 * width`; общий layout, своя bind group на текстуру (view + sampler `Nearest`, clamp); камера — отдельная группа.
- `pick_format` больше не берёт не-sRGB формат: при его отсутствии — паника с понятным сообщением. Линейный `Color` на не-sRGB поверхности темнеет молча.
- Pong (`tick_hz = 60` фиксирован): два игрока за одной клавиатурой — `W`/`S` и `↑`/`↓`, без ИИ. Поле 640×360 с центром в нуле; ракетки 8×60 у `x = ±300`, 300 ед/с; мяч 10×10, модуль скорости — старт 240 ед/с, +5% за удар, предел 420 ед/с. Подача под 30° к горизонту, вверх и вниз по очереди. Угол отскока от ракетки — по точке удара: смещение от центра ракетки / её полувысота × 60°, иначе мяч ходит по одной прямой. Коллизия мяча с ракеткой — **заметание**: отрезок движения центра мяча относительно ракетки против её AABB, расширенного на полуразмер мяча (slab-тест, первое `t ∈ [0, 1]`). Проверка только конечных положений пропускает касание угла движущейся ракеткой даже при 7 ед/тик < 8. Отражение только приближающегося мяча, с выталкиванием. Гол → +1, подача из центра к пропустившему, `prev = cur` (иначе интерполяция нарисует перелёт через поле). Интерполяция только мяча: `lerp(prev, cur, alpha)`; ракетки двигаются ступенями по тикам — принятая цена. Счёт 0–9, на 10 матч сбрасывается.

## Implementation Steps

### Task 1: Зависимости и только sRGB-поверхность

**Files:**
- Modify: `Cargo.toml`, `engine/Cargo.toml`, `engine/src/renderer.rs`, `engine/src/lib.rs`

- [x] `glam`, `bytemuck` (`derive`), `image` (`default-features = false`, `png`) в `[workspace.dependencies]` и `engine`
- [x] переписать тест `prefers_srgb_format`: без sRGB-форматов → `None` (сейчас закрепляет `Rgba16Float → Some`)
- [x] `pick_format` — только `is_srgb()`; `Renderer::new` паникует с сообщением
- [x] `pub use glam::{Vec2, vec2}` в `engine`
- [x] тесты и clippy — зелёные

### Task 2: Камера и letterbox

**Files:**
- Create: `engine/src/camera.rs`
- Modify: `engine/src/lib.rs`, `docs/design.md`

- [x] тесты: углы мира 640×360 → NDC ±1; `z = 0` → глубина 0.5; Y вверх (верх мира → NDC `+1`); окно шире мира → поля слева/справа, по центру; уже — сверху/снизу; та же пропорция → весь кадр; окно 1×1 — вьюпорт не нулевой
- [x] `Camera2D`, `view_proj`, `Viewport`, `letterbox`
- [x] `docs/design.md`: фиксированный мир + letterbox; «Молчаливые поломки» — глубина в wgpu `0..1`
- [x] тесты — зелёные

### Task 3: Текстуры на CPU — `Assets`

**Files:**
- Create: `engine/src/assets.rs`
- Modify: `engine/src/lib.rs`, `engine/src/frame.rs`, `docs/design.md`

- [x] тесты: `premultiply` — белый α=128 → 188, α=0 → `0,0,0,0`, α=255 не меняется; `Assets::default` — первая в очереди `WHITE` (id 0, 1×1, белая); валидный PNG (кодируется в тесте через `image` во временный файл) → в очереди его размер и премультиплицированные пиксели; отсутствующий путь и битый PNG → шахматка 8×8; id выдаются подряд; `take_pending` очищает очередь
- [x] `TextureId`, `PendingTexture { id, width, height, rgba }`, `Assets { load_texture, take_pending }`, `premultiply`, шахматка
- [x] `Context.assets` (через `Default`; `render(&Gfx)` этапа 0 пока не меняется)
- [x] `docs/design.md`, «Ресурсы»: на этапе 1 `TextureId` без поколения и без кэша путей — поколения и кэш вместе с выгрузкой на этапе 2; «Молчаливые поломки» — премультипликация в линейном пространстве
- [x] тесты — зелёные

### Task 4: Спрайты и батчинг на CPU

**Files:**
- Create: `engine/src/sprite.rs`
- Modify: `engine/src/frame.rs`, `engine/src/lib.rs`, `docs/design.md`

- [x] тесты `batch`: сортировка по слою; внутри слоя — по текстуре; одинаковый ключ сохраняет порядок вызовов; диапазоны режутся на смене текстуры и покрывают все инстансы без пропусков; пустой список → пусто; `Color` f64 → `[f32; 4]` в `Instance`
- [x] тесты `step_frame`: спрайты прошлого кадра исчезают и при 0 тиков; `clear_color` и `camera` сохраняются
- [x] `Sprite`, `Instance`, `Batch`, `batch(&[Sprite])`; `Gfx { clear_color, camera, sprites }`, `draw_sprite`; очистка в `step_frame`
- [x] `docs/design.md`: API `draw_sprite` без поворота и UV, пока нет потребителя; «Молчаливые поломки» — очистка спрайтов перед `draw`, порядок между текстурами только через слой
- [x] тесты — зелёные

### Task 5: `SpritePipeline` и офскрин-тест

Читать: «Learn Wgpu», главы The Pipeline, Buffers and Indices, Textures and bind groups, Instancing.

**Files:**
- Create: `engine/src/sprite_pipeline.rs`, `engine/src/sprite.wgsl`
- Modify: `engine/src/lib.rs` (`pub mod sprite_pipeline` — иначе до Task 6 он мёртвый код для clippy), `docs/design.md`

- [x] тест `grow_capacity(current, needed)`: хватает → `None`; не хватает → степень двойки ≥ needed, кратная 4
- [x] офскрин-тест (пишется до шейдера). Цель 64×64 `Rgba8UnormSrgb`, мир 64×64, фон — непрозрачный не белый. Тестовые текстуры — временные PNG через `Assets::load_texture`, как в Task 3. Все сценарии — одна сцена и один `submit`: `draw` пишет буферы через `write_buffer`, и несколько `draw` до `submit` затёрли бы друг друга. Эталоны считаются в тесте вручную, не через `premultiply` движка; допуск ±2. Контрольные пиксели — внутри спрайтов, не на краях:
  - **UV**: текстура 2×2 с четырьмя разными цветами, спрайт 32×32 — проверяется центр каждого квадранта
  - **двойная премультипликация**: texel с α = 128 и `tint.a = 0.5` поверх фона
  - **порядок слоёв**: два перекрывающихся спрайта подаются в обратном порядке, а `TextureId` выбраны так, что сортировка только по текстуре дала бы противоположный результат; проверяется цвет перекрытия
  - **sRGB-запись**: непрозрачный серый sRGB 128 возвращается как ≈128 (линейная запись дала бы ≈55)
  - **рост буфера**: инстансов больше начальной ёмкости, второй батч начинается не с нуля
- [x] `sprite.wgsl`, `SpritePipeline::new(device, queue, format)`, `upload(&mut Assets)`, `draw(encoder, view, viewport, &Gfx)` — сам открывает проход с `Clear` и `set_viewport`
- [x] `docs/design.md`, «Молчаливые поломки»: `tint.a` умножает и RGB в шейдере
- [x] тесты — зелёные, офскрин-тест не пропущен

### Task 6: Спрайты в окне

**Files:**
- Modify: `engine/src/renderer.rs`, `engine/src/app.rs`, `CLAUDE.md`

- [ ] `Renderer` владеет `SpritePipeline`; `render(&Gfx, &mut Assets)`: `upload` до проверки доступности поверхности, затем кадр поверхности → `letterbox` → `SpritePipeline::draw`; собственный clear pass `Renderer` удаляется
- [ ] `App` передаёт `&mut ctx.assets`
- [ ] `CLAUDE.md`: новая сигнатура `render`, `ctx.assets`, офскрин-тест как часть границы тестируемости
- [ ] тестов нет: путь покрыт тестами задач 2–5; `clear-color` по-прежнему запускается и меняет цвет

### Task 7: Логика Pong

**Files:**
- Create: `games/pong/Cargo.toml`, `games/pong/src/main.rs`
- Modify: `docs/design.md`

- [ ] тесты на чистом `Pong::tick(&Input)` без окна: ракетка движется по клавише и не выходит за поле; отскок от верхней и нижней стенки; удар отражает и ускоряет модуль скорости на 5%, не выше 420; угол отскока зависит от точки удара (центр → горизонтально, край → 60°); подачи чередуются вверх/вниз; удаляющийся мяч не отражается; **контрпример codex**: ракетка в (300, 0) движется вниз, мяч в (290, 34) летит вправо на пределе — удар засчитан; мяч на пределе не проходит сквозь неподвижную ракетку; гол → счёт +1, подача к пропустившему, `prev == cur`; 10-й гол сбрасывает матч
- [ ] `sweep(from, delta, aabb) -> Option<f32>` — заметание отрезка против AABB (slab-тест) с отдельными тестами: попадание, промах, старт внутри, движение параллельно грани
- [ ] `Pong`: ракетки, мяч (`prev`, `cur`, скорость), счёт; `main` и `Game` с минимальным `draw` — ракетки и мяч белыми квадами, мяч по `lerp(prev, cur, alpha)` (без чтения `prev` clippy упадёт на dead_code)
- [ ] `docs/design.md`, «Молчаливые поломки»: заметание против движущейся ракетки, `prev = cur` при телепорте
- [ ] тесты — зелёные

### Task 8: Отрисовка Pong

**Files:**
- Modify: `games/pong/src/main.rs`
- Create: `games/pong/assets/ball.png`

- [ ] тест таблицы сегментов: `8` — все 7, `1` — 2 сегмента, у каждой цифры 0–9 свой набор
- [ ] `ball.png` 10×10 с полупрозрачным краем — одноразовым скриптом на stdlib Python, скрипт не коммитится
- [ ] `init`: камера 640×360, `load_texture` по `concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ball.png")` (не относительно cwd), `TextureId` хранится в игре
- [ ] `draw`: пунктир по центру, мяч текстурой `ball.png` слоем выше пунктира, цифры счёта
- [ ] тесты — зелёные

### Task 9: Verify acceptance criteria
- [ ] `cargo test --workspace -- --nocapture`: нет `SKIPPED`; `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `RUST_LOG=info cargo run -p pong` на Wayland и на X11 (`WAYLAND_DISPLAY=`): партия до 10 играбельна
- [ ] ресайз в очень широкое и очень узкое окно, сворачивание: поле не меняется, поля цвета фона, игра не падает
- [ ] `cargo run -p clear-color` работает как прежде

### Task 10: [Final] Update documentation
- [ ] `README.md`: запуск Pong и управление
- [ ] сверить `CLAUDE.md` и `docs/design.md` с итоговым кодом
- [ ] перенести план в `docs/plans/completed/`
