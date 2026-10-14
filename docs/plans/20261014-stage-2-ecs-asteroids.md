# Этап 2: ECS, хэндлы, текст и Asteroids

## Overview
- Движок: ECS на sparse set с поколениями, буфер команд, типизированные хэндлы ресурсов с кэшем путей, поворот и UV у спрайтов, текст через `fontdue`, статистика кадров.
- Игра `games/asteroids`: корабль, пули, астероиды трёх размеров, частицы взрывов, счёт и жизни текстом. Спрайты из PNG.
- Готово, когда: Asteroids играбелен; на этой машине (AMD Renoir, RADV), release, vsync, окно 1280×720 после смерти корабля у центра есть окно лога с минимумом спрайтов > 5 000 и FPS 60 (частицы смерти держат > 5 000 дольше 2 с, поэтому одно окно ≥ 1 с целиком ложится в нагрузку); офскрин-тест GPU проходит, а не пропускается.

## Context (from discovery)
- Этапы 0 и 1 в `main` (`2d29f2c`), ветка `stage-2-ecs`.
- `Sprite { texture, pos, size, color, layer }` — без поворота и UV; `TextureId(pub u32)`; все текстуры `Rgba8UnormSrgb`, один сэмплер `Nearest` (`engine/src/sprite_pipeline.rs:113`).
- FPS — `RateCounter` по моменту начала кадра (`engine/src/app.rs:105`), не связан с нагрузкой кадра.
- Новая зависимость: `fontdue 0.9.4`. Шрифт — JetBrains Mono Regular (OFL) из `~/.local/share/fonts/JetBrainsMono/fonts/ttf/`.
- Решения согласованы с codex (тред `stage2-ecs`, два раунда), план проверен агентом plan-review.

## Development Approach
- **testing approach**: TDD — тест раньше кода для всего, что не требует окна; GPU-путь — офскрин-тестом.
- Следующая задача — только при зелёных `cargo test --workspace` и `cargo clippy --workspace --all-targets -- -D warnings`.
- Решение, меняющее `docs/design.md` или `CLAUDE.md`, правится в той же задаче.
- Изменения скоупа — сразу в этот файл: ➕ новая задача, ⚠️ блокер.

## Testing Strategy
- Юнит-тесты в `#[cfg(test)]` того же файла: ECS, команды, хэндлы, атлас, раскладка текста, статистика кадров, логика Asteroids.
- Офскрин-тест `SpritePipeline` расширяется: повёрнутый неквадратный спрайт с асимметричным UV, атлас покрытия с промежуточным значением и `tint.a < 1`, граница соседних глифов. Пропуск — только без адаптера.
- Окно, нагрузка, ресайз — запуском Asteroids (Task 13).

## Solution Overview
- ECS — библиотека в движке, но мир, компоненты и системы принадлежат игре: `World` живёт в структуре игры, не в `Context`. Рендер об ECS не знает: система спрайтов Asteroids кормит `ctx.gfx`. Стандартные компоненты (`Transform` и т.п.) переедут в движок, когда появится вторая игра на ECS.
- Хранилище на тип — `RefCell<SparseSet<T>>`: из `&World` можно держать `Ref<A>` и `RefMut<B>` одновременно, конфликт на одном типе — паника. Структурные изменения (`spawn`, `insert`, `despawn`, `apply`) требуют `&mut World`, поэтому во время обхода их запрещает компилятор.
- Однократность последствий столкновения — в игре: пара обрабатывается, только если обе сущности живы и не ждут удаления; обе ставятся в очередь вместе.
- Частицы — сущности ECS: нагрузка 5 000+ проверяет и мир, и батчер. Спаун — одна closure на взрыв, а не на частицу.
- Поколения в хэндлах ресурсов откладываются: выгрузки нет, индексы только растут.

## Technical Details

### ECS (`engine/src/ecs/`)
- `Entity { index: u32, generation: u32 }` (`gen` — ключевое слово в edition 2024); аллокатор со списком свободных, `despawn` увеличивает поколение; `is_alive(e)` сравнивает поколение.
- `SparseSet<T>`: `sparse: Vec<u32>` (`NONE = u32::MAX`), `dense: Vec<T>`, `entities: Vec<Entity>`. `get`/`get_mut` сравнивают полный `Entity`. Удаление — `swap_remove` с правкой `sparse` перенесённого. `insert`/`remove` — `pub(crate)`: вставка в пустой слот не знает о живости и создала бы призрачную сущность.
- Публично снаружи `World`: `get`, `get_mut`, `iter() -> (Entity, &T)`, `iter_mut`, `len`.
- `World`: `register::<T>()` (в `init`, до обходов), `spawn() -> Entity`, `insert(e, T)` (паника на мёртвую `e` и незарегистрированный тип), `despawn(e)` (no-op на мёртвую), `is_alive`, `storage::<T>() -> Ref<SparseSet<T>>`, `storage_mut::<T>() -> RefMut<SparseSet<T>>` — паника с `type_name` на незарегистрированный тип.
- Хранилища — `HashMap<TypeId, Box<dyn ErasedStorage>>`, `ErasedStorage { remove_entity(&mut self, Entity), as_any }`; `despawn` проходит все через `RefCell::get_mut`.
- `Commands`: `spawn(impl FnOnce(&mut World) + 'static)`, `despawn(e)`, `is_pending(e)` (`HashSet<Entity>` по полному `Entity`, без линейного поиска), `apply(&mut World)` — сначала despawn, потом spawn, затем очистка. Сущность из `spawn` получает id только в `apply`.
- Систем-планировщика и запросов на кортежах нет: обход `iter()` одного хранилища + `get` в другом.

### Хэндлы и ресурсы (`engine/src/assets.rs`)
- `Handle<T> { pub(crate) index: u32, PhantomData<T> }`, маркеры `Texture` и `Font`; `Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash` — ручными impl (derive потребовал бы их от `T`), `Ord` нужен сортировке `batch`; `TextureId` заменяется на `Handle<Texture>`, `Handle::<Texture>::WHITE`. Поколения нет: индексы append-only на всём сроке `Assets` и GPU-таблицы; при первом переиспользовании слота поколение обязательно (запись в `docs/design.md`).
- Кэш путей: `load_texture(path)` дважды — один хэндл и одна загрузка на GPU.
- `PendingTexture.kind: TextureKind { Color, Coverage }` (`TextureKind` экспортируется); покрытие добавляет `pub(crate) Assets::add_coverage(w, h, &[u8]) -> Handle<Texture>`. `Color` — как сейчас: `Rgba8UnormSrgb`, `premultiply`, `Nearest`. `Coverage` — байты `[c, c, c, c]` (линейный премультиплицированный белый) **без** `premultiply`, `Rgba8Unorm`, общий второй сэмплер `Linear`. Через sRGB-текстуру покрытие 128 дало бы RGB ≈ 0,216 при alpha ≈ 0,502 — тёмная кайма.

### Спрайты
- `Sprite` + `rotation: f32` (радианы, против часовой) и `uv: Rect { min: Vec2, max: Vec2 }` (доли текстуры, `(0, 0)` — левый верхний). `Sprite::default()`: `WHITE`, uv `0..1`, **белый** tint (`Color::default` чёрный), `layer 0`.
- `Instance` + `rotation [cos, sin]` (на CPU) и `uv_min`, `uv_max`. Шейдер: геометрия из исходного угла квада, поворот `local × size`, затем `+ pos`; UV — отдельно `mix(uv_min, uv_max, corner)`. Цель рендера остаётся sRGB.

### Текст (`engine/src/text.rs`)
- `assets.load_font(path, px) -> Handle<Font>`, кэш по `(path, px)`. Растеризация ASCII 32..=126 через `fontdue` при загрузке; полочная упаковка с прозрачным отступом 1 texel вокруг каждого глифа и у края атласа (иначе `Linear` подтянет соседа). Атлас получает обычный `Handle<Texture>` из текстурного аллокатора. Нет файла или битый TTF → `log::warn!`, шрифт-заглушка без атласа: глиф — белый прямоугольник 0,5px × 0,7px на baseline, `advance` 0,6px, строка 1,2px.
- `Gfx::draw_text(&Assets, Handle<Font>, &str, pos, scale, color, layer)`: `pos` — начало baseline, Y вверх. Центр глифа из `xmin`, `ymin`, `width`, `height`; курсор по `advance_width`; `'\n'` — вниз на `new_line_size` из line metrics; пробел без спрайта; символ вне атласа → `'?'`. Без кернинга.
- Шрифт лежит в `assets/fonts/` корня: тесты движка — `concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/fonts/…")`, игра — `"/../../assets/fonts/…"`.
- HUD: `load_font(.., 32.0)`, рисуется со `scale = 0.5` — при окне 1280×720 texel к пикселю 1:1.

### Статистика кадров (`engine/src/stats.rs`, `engine/src/app.rs`)
- `FrameStats::record(Instant::now(), sprites)` — момент **после** `render` с `Ok(true)`, не `now` начала кадра (`engine/src/app.rs:105`); `Ok(false)` сбрасывает предыдущую отметку, иначе первый кадр после сворачивания попадёт в `worst` секундами. Интервал, пересекающий границу окна, уходит в новое окно, как у `RateCounter`; интервал между завершениями успешных render связан с числом спрайтов этого кадра. Окно ≥ 1 с отдаёт `{ fps, worst, min_sprites, max_sprites }`. Заменяет `RateCounter` для FPS, `RateCounter` остаётся для TPS.
- Лог раз в секунду: `fps 60 tps 60 worst 17.4ms sprites 5012..6150`. Собственный лог `clear-color` убирается, иначе строки задвоятся.

### Asteroids (`games/asteroids`, tick 60, мир 640×360)
- Управление: A/D — поворот 4 рад/с, W — тяга 300 ед/с², Space — огонь (кулдаун 0,2 с), Enter — рестарт после GAME OVER. Сопротивление — скорость × 0,5 в секунду, предел 300 ед/с.
- Пули: 480 ед/с + скорость корабля, TTL 1 с, радиус 2. Астероиды: радиусы 32/16/8, скорость 40–80/60–110/90–150 ед/с, вращение; раскол на 2 следующего размера; очки 20/50/100. Корабль: радиус 8.
- Волна: 4 больших + 1 за волну (до 11), появляются не ближе 120 ед от корабля. 3 жизни; после смерти корабль скрыт 2 с, затем в центре с неуязвимостью 2 с (мигает). GAME OVER — текст, частицы доживают. Возрождение — телепорт: `prev = pos`.
- Порядок тика: управление → интегрирование (`prev = pos`) → столкновения по движению до переноса → перенос → время жизни → `cmds.apply`.
- Столкновения — заметание: отрезок относительного перемещения за тик против круга суммы радиусов, ближайшее `t ∈ [0, 1]`, старт внутри → 0. И для пуль, и для корабля: проверка конечных положений пропускает касание при любом пределе скорости.
- Однократность: пуля в порядке обхода берёт ближайшую по `t` цель, у которой `is_alive && !is_pending` для **обеих**; обе — в `despawn`, затем очки, раскол, взрыв; обход пули окончен. Потом корабль против астероидов тем же правилом. Пули раньше корабля.
- Перенос (тор без копий) — для всех сущностей, включая частицы: объект с визуальным радиусом `r` (полудиагональ спрайта) переносится, когда целиком ушёл: `x > 320 + r → x -= 640 + 2r` (и по Y с 360). Невидимость — мгновение. `prev` сдвигается на тот же вектор. Столкновения через шов не считаются — правило игры.
- Интерполяция: `lerp(prev, pos, alpha)`; угол хранится ненормализованным, иначе переход 2π→0 крутит спрайт почти полный оборот назад.
- Частицы: 2×2, скорость 20–150 ед/с, альфа = остаток/TTL. Взрыв астероида: 300/150/75, TTL 0,5–2 с. Смерть корабля: 6 000, TTL 3–4 с — при TTL 0,5–2 с живых > 5 000 было бы только 0,75 с, и окно статистики ≥ 1 с не легло бы в нагрузку.
- Сущность из closure получает `prev = pos` при создании, иначе первый кадр интерполируется из (0, 0). Closure взрыва `'static` и не заимствует ГПСЧ игры: ей передаётся сид из `rng.next()` — копия состояния дала бы одинаковые взрывы в одном тике.
- ГПСЧ — свой xorshift64 в игре: сид из времени в `main`, фиксированный в тестах.
- Спрайты: `ship.png`, `asteroid.png` (один, масштаб по размеру), `bullet.png`, `particle.png` — одноразовым скриптом на stdlib Python, скрипт не коммитится.

### Молчаливые поломки этапа 2 (в `docs/design.md` по задачам)
- Вставка компонента в слот мёртвой сущности даёт призрачную сущность в обходе.
- Двойные последствия одного столкновения до `apply`: очки, раскол, жизни.
- Кэш шрифта без `px` в ключе отдаёт чужие метрики.
- Атлас покрытия как sRGB или повторная премультипликация — тёмные края глифов.
- Без отступа между глифами `Linear` подмешивает соседа.
- Угол через 2π при lerp; телепорт при переносе без сдвига `prev`.
- FPS за окно не говорит о нагрузке кадра: число спрайтов и длительность должны относиться к одному кадру.

## Implementation Steps

### Task 1: Поворот и UV у спрайтов

**Files:**
- Modify: `engine/src/sprite.rs`, `engine/src/sprite.wgsl`, `engine/src/sprite_pipeline.rs`, `engine/src/frame.rs`, `engine/src/lib.rs`, `games/pong/src/main.rs`, `docs/design.md`

- [x] тесты: `Sprite::default()` — белый tint, uv `0..1`, `WHITE`; `Instance::from` — `cos/sin` поворота, `uv_min/uv_max`; `batch` не меняет поведение
- [x] `Rect`, поля `Sprite`, `Instance`, vertex attributes, `InstanceIn` и шейдер: геометрия от угла, UV отдельно
- [x] офскрин-тест: старые проверки сохранены; неквадратный спрайт, повёрнутый на 90°, с асимметричным UV-фрагментом — цвета в ожидаемых углах
- [x] Pong и литералы тестов — через `..Default::default()`
- [x] `docs/design.md`, «Рендер»: строка «Этап 1: осевой прямоугольник…» — поворот и UV-подпрямоугольник появились
- [x] тесты — зелёные

### Task 2: Хэндлы и кэш путей

**Files:**
- Modify: `engine/src/assets.rs`, `engine/src/sprite.rs`, `engine/src/sprite_pipeline.rs`, `engine/src/lib.rs`, `games/pong/src/main.rs`, `docs/design.md`

- [x] тесты: один путь дважды → один хэндл и одна запись в очереди; разные пути → разные; отсутствующий файл → шахматка, повтор того же пути не грузит её снова; `WHITE` — индекс 0
- [x] `Handle<T>`, маркеры `Texture`, `Font`; `TextureId` → `Handle<Texture>` по всему движку и Pong
- [x] `docs/design.md`, «Ресурсы»: поколения отложены до выгрузки, append-only индексы — инвариант
- [x] тесты — зелёные

### Task 3: Текстуры покрытия

**Files:**
- Modify: `engine/src/assets.rs`, `engine/src/sprite_pipeline.rs`, `docs/design.md`

- [x] тесты: `PendingTexture.kind` — `Color` у PNG, `Coverage` у `add_coverage`; байты покрытия `[c, c, c, c]` не проходят `premultiply`
- [x] `TextureKind`, `add_coverage`; в pipeline — формат по виду и второй сэмплер `Linear`, подставляемый в bind group текстуры покрытия
- [x] офскрин-тест: покрытие 128 с белым tint и `tint.a = 0.5` на **цветном** фоне (как `bg` в существующем тесте) → `0,251 + dst·(1 − 0,251)` с допуском. На чёрном фоне альфа не видна и непрозрачный серый `[c, c, c, 255]` прошёл бы; через sRGB-формат тест тоже падает
- [x] `docs/design.md`, «Молчаливые поломки»: атлас покрытия как sRGB и повторная премультипликация
- [x] тесты — зелёные

### Task 4: Шрифт и атлас

**Files:**
- Create: `engine/src/text.rs`, `assets/fonts/JetBrainsMono-Regular.ttf`, `assets/fonts/OFL.txt`
- Modify: `Cargo.toml`, `engine/Cargo.toml`, `engine/src/assets.rs`, `engine/src/lib.rs`, `docs/design.md`

- [x] тесты чистой упаковки: прямоугольники не перекрываются с учётом отступа 1 texel, не касаются края атласа, все в границах
- [x] тесты `load_font`: глифы ASCII 32..=126 есть; `(path, 32)` дважды → один хэндл; `(path, 16)` → другой с другими метриками; отсутствующий файл → шрифт без атласа и `warn`
- [x] `fontdue` в зависимости; растеризация, упаковка, атлас как `Coverage` через текстурный аллокатор
- [x] `docs/design.md`: кэш шрифта по `(path, px)`, отступ между глифами
- [x] тесты — зелёные

### Task 5: Раскладка и `draw_text`

**Files:**
- Modify: `engine/src/text.rs`, `engine/src/frame.rs`, `engine/src/sprite_pipeline.rs`

- [x] тесты: курсор растёт на `advance × scale`; пробел без спрайта; `'\n'` — вниз на `new_line_size`; символ вне ASCII → `'?'`; центр глифа из `xmin/ymin` при Y вверх; заглушка — прямоугольники 0,5px × 0,7px с шагом 0,6px
- [x] `layout` — чистая функция в `Sprite`; `Gfx::draw_text` поверх неё
- [x] офскрин-тест: синтетический атлас упаковщиком Task 4 — глиф A с покрытием 0 рядом с глифом B с покрытием 255; A рисуется с увеличением ≥ 2 и сдвигом на полtexel'а (при 1:1 центр фрагмента в центре texel'а, и `Linear` соседа не трогает) — у края A альфа ≈ 0
- [x] тесты — зелёные

### Task 6: ECS — сущности и хранилища

**Files:**
- Create: `engine/src/ecs/mod.rs`, `engine/src/ecs/entity.rs`, `engine/src/ecs/sparse_set.rs`, `engine/src/ecs/world.rs`
- Modify: `engine/src/lib.rs`, `docs/design.md`

- [x] тесты: повтор индекса после despawn с новым поколением; устаревшая `Entity` → `None` в `get`; `swap_remove` правит `sparse` перенесённого, удаление последнего; despawn удаляет из всех хранилищ; `insert` в мёртвую — паника; незарегистрированный тип — паника с именем; `Ref<A>` + `RefMut<B>` одновременно — ок; два `storage_mut::<A>` — `#[should_panic]`
- [x] `Entity`, аллокатор, `SparseSet`, `ErasedStorage`, `World`
- [x] `docs/design.md`, «Молчаливые поломки»: вставка в слот мёртвой сущности
- [x] тесты — зелёные

### Task 7: Буфер команд

**Files:**
- Create: `engine/src/ecs/commands.rs`
- Modify: `engine/src/ecs/mod.rs`, `engine/src/lib.rs`

- [x] тесты: `apply` — despawn раньше spawn, порядок spawn сохраняется; двойной `despawn` — no-op; `is_pending` до и после `apply`; despawn мёртвой — no-op; одна closure создаёт N сущностей
- [x] `Commands`
- [x] тесты — зелёные

### Task 8: Статистика кадров

**Files:**
- Modify: `engine/src/stats.rs`, `engine/src/app.rs`, `engine/src/lib.rs`, `games/clear-color/src/main.rs`, `docs/design.md`

- [x] тесты `FrameStats`: окно ≥ 1 с; `worst` — самый длинный интервал окна, интервал через границу — в новом окне; `min/max_sprites` по кадрам окна; после сброса (`Ok(false)`) первый интервал не считается; отдаётся один раз
- [x] `FrameStats`; `app.rs`: `record(Instant::now(), sprites)` после `Ok(true)`, сброс на `Ok(false)`, лог в одну строку, `ctx.time.fps` из `FrameStats`; убрать лог `clear-color`
- [x] `docs/design.md`, «Молчаливые поломки»: FPS окна не говорит о нагрузке кадра
- [x] тесты — зелёные

### Task 9: Asteroids — корабль, пули, перенос

**Files:**
- Create: `games/asteroids/Cargo.toml`, `games/asteroids/src/main.rs`
- Modify: `docs/design.md`

- [x] тесты на чистом тике без окна: поворот и тяга по клавишам, сопротивление и предел скорости; выстрел с кулдауном, скорость пули складывается со скоростью корабля, пуля исчезает по TTL; перенос по `r` и сдвиг `prev`, частицы тоже переносятся; угол через 2π интерполируется коротко; пуля после `apply` имеет `prev == pos`
- [x] компоненты, `World` и `Commands` в игре; xorshift64; `main` и `Game` с минимальным `draw` белыми квадами
- [x] `docs/design.md`, «Молчаливые поломки»: угол при lerp, перенос без сдвига `prev`
- [x] тесты — зелёные

### Task 10: Asteroids — астероиды и столкновения

**Files:**
- Modify: `games/asteroids/src/main.rs`, `docs/design.md`

- [ ] тесты `sweep_circle`: попадание, промах, старт внутри, касание при перемещении меньше суммы радиусов (контрпример codex: R = 10, отрезок (−1; 9,99)→(1; 9,99))
- [ ] тесты: две пули в один астероид за тик — одни очки, один раскол; одна пуля на пути двух — бьёт ближайший; пуля и корабль в один астероид — пуля первой, корабль цел; смерть корабля — −1 жизнь, неуязвимость после возрождения, `prev == pos` после возрождения; осколки после `apply` — `prev == pos`; очищенная волна → следующая на 1 больше и не ближе 120 ед; 0 жизней → GAME OVER, Enter → рестарт
- [ ] волны, раскол, очки, жизни, правило однократности пары
- [ ] `docs/design.md`, «Молчаливые поломки»: двойные последствия столкновения до `apply`
- [ ] тесты — зелёные

### Task 11: Частицы

**Files:**
- Modify: `games/asteroids/src/main.rs`

- [ ] тесты: взрыв большого — 300 частиц одной командой; два взрыва за тик дают разные частицы; альфа убывает с TTL, по истечении частица удалена; с фиксированным сидом после смерти корабля живых частиц > 5 000 не меньше 120 тиков подряд
- [ ] взрыв как одна closure `Commands::spawn` с сидом из `rng.next()`
- [ ] тесты — зелёные

### Task 12: Отрисовка Asteroids

**Files:**
- Modify: `games/asteroids/src/main.rs`
- Create: `games/asteroids/assets/{ship,asteroid,bullet,particle}.png`

- [ ] тест: система спрайтов выдаёт по спрайту на видимую сущность с интерполированными позицией и углом
- [ ] PNG одноразовым скриптом; `load_texture` и `load_font` по `concat!(env!("CARGO_MANIFEST_DIR"), ...)`
- [ ] `draw`: сущности, мигание неуязвимости, HUD — счёт, жизни, FPS; GAME OVER. Число спрайтов — только в логе Task 8
- [ ] тесты — зелёные

### Task 13: Verify acceptance criteria
- [ ] `cargo test --workspace -- --nocapture`: нет `SKIPPED`; clippy
- [ ] `make asteroids`: партия играбельна на Wayland и на X11 (`WAYLAND_DISPLAY=`)
- [ ] смерть корабля у центра: окно лога с `sprites` минимум > 5 000 и `fps 60`; записать `worst` в этот план
- [ ] ресайз и сворачивание: мир не меняется, текст читаем, игра не падает
- [ ] `make pong`, `make clear-color` работают как прежде

### Task 14: [Final] Update documentation
- [ ] `README.md`: запуск и управление Asteroids; строка про лог `clear-color` — лог теперь пишет движок
- [ ] сверить `CLAUDE.md` (ECS в игре, хэндлы, текст) и `docs/design.md` с итоговым кодом
- [ ] перенести план в `docs/plans/completed/`

## Post-Completion
- Проверка глазами: читаемость текста при окнах, не кратных 640×360; отсутствие тёмной каймы у глифов и частиц.
