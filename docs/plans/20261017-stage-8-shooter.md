# Этап 8: шутер в лесу

## Overview
- Новая игра `games/shooter` в духе Doom:
  - один плоский уровень из ASCII-сетки на Kenney Mini Forest: деревья, камни и забор — стены;
  - враги из Kenney Blocky Characters трёх типов, со своими анимациями;
  - два 3D-бластера из Kenney Blaster Kit в руках: мгновенный (hitscan) и со снарядом;
  - ключ, двери, аптечки, патроны, выход; смерть и рестарт уровня. Сохранения нет.
- Движок — только то, что нужно игре:
  - пространственный звук эффектов: громкость левого и правого канала;
  - слой оружия в руках: второй 3D-проход после очистки глубины.
- Готово, когда:
  - бот проходит уровень от старта до выхода: убивает врагов обоими оружиями, тратит патроны, берёт ключ, открывает дверь;
  - смерть игрока рестартит уровень;
  - закрытая дверь останавливает ход игрока и врагов, взгляд врага, луч выстрела и снаряд;
  - звук врага справа громче в правом канале;
  - ствол не уходит в стену, а мир за оружием рисуется как без него;
  - `make check`, `make cov`, `make mutants-diff` зелёные; офскрин-тесты — под `REQUIRE_GPU=1`.

## Context (from discovery)
- **Рендер.**
  - `MeshPipeline::draw` очищает цвет и глубину и пишет `scene_buf`, `instances`, `palettes` через `write_buffer` (`engine/src/mesh_pipeline.rs:258`–`:326`); второй `draw` до `submit` затрёт первый (`:259`). `draw` берёт `&mut self` ради `lights_warned` и `joints_warned`.
  - Uniform сцены — свет, туман и глаз от `scene.camera` (`mesh_pipeline.rs:47`–`:62`).
  - `Renderer::render`: сцена → спрайты с `Load` → egui (`renderer.rs:228`–`:247`). `renderer.rs` вне `make cov` и мутантов.
  - `Scene` — `Default` (`scene.rs:16`). Полные литералы без `..` — `games/maze/src/main.rs:389`, `games/racing/src/main.rs:602`.
- **Звук.**
  - Команды `Play`, `Music`, `LoopStart`, `LoopSet`, `LoopStop` (`engine/src/audio/mixer.rs:21`–`:41`); `VOICES = 32`.
  - `Audio::play(sound, volume)` (`audio/mod.rs:66`). Звук — стерео-интерлив, моно при декоде дублируется в L/R (`decode.rs:21`); `fill` сводит в моно на одноканальном устройстве (`mod.rs:207`). Громкость чистит `gain()`.
- **Модели** (все CC0, проверены: данные внутри GLB, без морфов, анимации LINEAR):
  - Blocky Characters: 18 GLB, текстура `Textures/texture-*.png`. Скелета нет — 8 узлов (`root`, `leg-*`, `torso`, `arm-*`, `head`) с анимациями узлов; 27 клипов, среди них `idle`, `walk`, `sprint`, `die`, `holding-right-shoot`, `attack-melee-right`, `attack-kick-right`. Габарит по Y — −1..8 единиц.
  - Mini Forest: `tree`, `tree-high`, `rocks-high`, `stones`, `fence`, `plant` и др. Двери и ключа нет.
  - Blaster Kit: `blaster-a`…`r`, `bullet-foam`, `clip-*`, `crate-*`.
  - Mini Forest, Blaster Kit и Mini Dungeon (`key.glb`, `gate.glb` из `games/maze/assets`) ссылаются на один URI `Textures/colormap.png`, а файлы разные. `Assets` кэширует текстуру по пути.
  - `Pose::globals` публичны (`engine/src/model.rs:34`): бластер в руке врага — `transform × globals[arm-right]` в игре.
- **Образцы.**
  - `games/maze`: ASCII-карта с ошибкой по строке (`map.rs`), игрок-круг на плане (`player.rs`), ворота твёрдые до конца анимации (`main.rs:298`), тесты сцены, HUD, захвата, рестарта. Код карты и игрока копируется в `shooter` с правками: игры друг от друга не зависят.
  - `games/racing`: чистая функция команд звука `SoundCmd` (`engine_sound.rs`), потому что `Audio` в тестах немой; исключение `update_sound` в `.cargo/mutants.toml`.
  - Makefile собирает цель на каждую игру из `games/*`: `make shooter` появится сам.
- **Согласование:** архитектуру проверил codex (тред `stage8-shooter`), план — агент plan-review.

## Development Approach
- **testing approach**: TDD, как на этапе 7. Тест пишется раньше кода для всего, что не требует окна и GPU, и сначала прогоняется на мутации.
- Каждая задача закрывается `cargo mutants -f <файлы задачи>` и `make cov`; следующая — только при зелёном `make check`.
- В той же задаче делаются решение, меняющее `docs/design.md` или «Архитектуру» в `CLAUDE.md`, и новая молчаливая поломка с пометкой «→ что ловит».
- Изменения скоупа — сразу в этот файл: ➕ новая задача, ⚠️ блокер.

## Testing Strategy
- **Движок:** юнит-тесты панорамы и микшера; офскрин (`REQUIRE_GPU=1`) — слой оружия через `MeshPipeline::draw_scene`.
- **Игра:** юнит-тесты модулей; сквозные — бот, смерть, закрытая дверь.
- **Вне покрытия:** вид, звук на слух, управление — запуск.

## Solution Overview
- **Одно препятствие для всего.** Ход игрока и врагов, взгляд, выстрел, снаряды и путь идут через твёрдые клетки сетки плюс закрытые двери. `StaticWorld` не нужен: уровень плоский. Дверь вне общего запроса пропустила бы выстрел или взгляд.
- **Бой на плане, как в Doom.** Враг — круг на плане, выстрел и снаряд летят по плану; наклон взгляда только визуальный. Снаряды врагов сквозь врагов пролетают.
- **Слой оружия без своей позиции.** Камера слоя — камера сцены со своими `fov_y` и `near`: свет и туман ложатся на оружие верно, а рассинхрон позиции невыразим.
- **Сведение в моно — по команде, а не по громкостям.** Иначе стерео-файл прямо спереди и чуть сбоку звучал бы по-разному.

## Technical Details
- **Звук** (`engine/src/audio`):
  - `Stereo { left, right }`; `Stereo::at(listener: &Camera3D, source: Vec3, volume, range)`: правый вектор — из `yaw` (наклон головы панораму не меняет), панорама равной мощности (в центре каждый канал — `volume·√½`), затухание `max(1 − d/range, 0)²` как у света;
  - `Audio::play_stereo(sound, Stereo)` → `Command::Play { frames, gains: Stereo, downmix: true }`; `play` → `Stereo { volume, volume }`, `downmix: false` — старое поведение;
  - `gain()` чистит каждый канал. Длящиеся голоса не меняются.
- **Слой оружия** (`scene.rs`, `mesh_pipeline.rs`):
  - `Scene::viewmodel: Option<Viewmodel { fov_y, near, models: Vec<ModelDraw> }>`;
  - `PassBuffers { scene_buf, scene_group, instances, capacity, palettes, palette_group, palette_capacity }`, у `MeshPipeline` — `[PassBuffers; 2]`, проход выбирается индексом. Меши и пайплайны общие;
  - `MeshPipeline::draw_scene(encoder, view, depth, textures, scene, assets, clear)` раскрывает модели мира и слоя и рисует оба прохода: мир — `Some(clear)`, слой — `None` (цвет `Load`); глубина в обоих `Clear(1.0)`. `Renderer::render` зовёт только его.
- **Ассеты:** подкаталог на набор — `assets/forest/`, `assets/blaster/`, `assets/characters/`, `assets/dungeon/` (ключ и ворота-дверь из `maze`), у каждого свой `Textures/`.
- **Игра** (`games/shooter/src`):
  - `map.rs`: `#` забор, `T` дерево, `R` камень, `.` трава, `P` игрок (взгляд на открытого соседа), `D` дверь, `k` ключ (один открывает все двери), `+` аптечка, `a` патроны, `X` выход, `m` ближний бой, `s` стрелок, `f` быстрый. Ошибки со строкой: неизвестный символ, не один `P` или `X`, рваные строки, двери без ключа. Уровень — `assets/levels/level01.txt`, тесты берут его `include_str!`.
  - `obstacle.rs`: `Obstacles { solid, width, doors }` — `solid(cell)`, `ray(from, dir, max) -> Option<f32>` по клеткам (DDA), `segment(a, b)`, BFS. Дверь проходима только после конца анимации.
  - `player.rs`: круг на плане, обзор мышью, здоровье.
  - `weapon.rs`: `Target { pos: Vec2, radius }`; `Weapon { model, damage, cooldown, ammo, kind: Hitscan | Projectile { speed } }`. Hitscan бьёт ближайшую цель ближе препятствия; снаряд за тик проверяет отрезок против препятствий и целей. 1 и 2 меняют оружие, ЛКМ стреляет; отдача — смещение модели в слое.
  - `enemy.rs`: `Enemy { kind, pos, hp, state: Sleep | Chase | Attack | Dead, t }`, отдаёт свой `Target`:
    - просыпается, если видит игрока лучом или ранен; идёт по BFS, путь пересчитывается раз в N тиков и при смене дверей;
    - ближний бой — `sprint` и `attack-melee-right`, много здоровья, урон вплотную;
    - стрелок — `holding-right-shoot`, бластер в руке, держит дистанцию, стреляет снарядом при прямой видимости;
    - быстрый — `sprint` и `attack-kick-right`, мало здоровья, быстрее игрока;
    - `die` без цикла, тело остаётся; клипы ищутся по имени при загрузке.
  - Вспышка выстрела — `PointLight`, держится по таймеру в состоянии игры и строится в `draw`.
  - Звук — `SoundCmd` из состояния тика, как в `racing`; эффекты — `play_stereo` со `Stereo::at` от камеры.
  - Масштаб: клетка — `CELL` метров; коэффициенты наборов проверяет тест габаритов.

## Implementation Steps

### Task 1: Пространственный звук эффектов
**Files:**
- Modify: `engine/src/audio/mod.rs`, `engine/src/audio/mixer.rs`, `engine/src/lib.rs`, `docs/design.md`, `CLAUDE.md`
- [x] тест `Stereo::at`: справа `right > left`, слева наоборот, спереди поровну по `volume·√½`, в глазу без NaN, за `range` ноль, сумма квадратов постоянна по углу, наклон камеры не меняет каналы
- [x] тест микшера: `play_stereo` со `Stereo { 1, 0 }` звучит только слева; стерео-файл спереди и чуть сбоку даёт непрерывные L и R; NaN в одном канале — тишина только в нём; `play` звучит как раньше
- [x] `Stereo`, `Stereo::at`, `play_stereo`, `Command::Play { gains, downmix }`
- [x] `docs/design.md` «Звук»: решение и молчаливая поломка «сведение по равенству громкостей даёт скачок образа на оси → тест»; `CLAUDE.md` «Архитектура»: `play_stereo`
- [x] `cargo mutants -f engine/src/audio/mod.rs -f engine/src/audio/mixer.rs`, `make cov`, `make check`

### Task 2: Слой оружия — второй 3D-проход
**Files:**
- Modify: `engine/src/scene.rs`, `engine/src/mesh_pipeline.rs`, `engine/src/renderer.rs`, `engine/src/lib.rs`, `games/maze/src/main.rs`, `games/racing/src/main.rs`, `docs/design.md`, `CLAUDE.md`
- [x] офскрин-тест `draw_scene`: модель слоя за стеной в мире рисуется поверх стены
- [x] офскрин-тест: пиксели вне модели слоя совпадают с кадром без слоя
- [x] офскрин-тест: точечный свет сцены освещает модель слоя, слой без моделей ничего не меняет
- [x] `Viewmodel`, `Scene::viewmodel`, `[PassBuffers; 2]`, `draw_scene`; `Renderer::render` — один вызов; литералы `Scene` в maze и racing — `viewmodel: None`
- [x] `docs/design.md` «3D»: слой и буферы проходов, молчаливая поломка «общие буферы проходов затирают мир → тест»; `CLAUDE.md` «Архитектура»: кадр и `Scene::viewmodel`
- [x] `REQUIRE_GPU=1 WGPU_BACKEND=vulkan cargo test -p engine`, `cargo mutants -f engine/src/scene.rs -f engine/src/mesh_pipeline.rs`, `make cov`, `make check`

### Task 3: Каркас игры и ассеты
**Files:**
- Create: `games/shooter/Cargo.toml`, `games/shooter/src/main.rs`, `games/shooter/assets/CREDITS.md`
- Create: `games/shooter/assets/{forest,blaster,characters,dungeon}/` с моделями и своими `Textures/`
- Modify: `docs/design.md`, `Cargo.toml` (фича `gltf` `KHR_materials_unlit`), `engine/src/model.rs`
- ➕ персонажи Blocky требуют `KHR_materials_unlit` и без фичи `gltf` не грузились: фича включена, расширение не учитывается, тест в `model.rs`
- [x] скопировать модели леса, трёх персонажей, двух бластеров, пули, обоймы, ящика; `key.glb` и `gate.glb` из `games/maze/assets`
- [x] тест: каждая модель грузится непустой, у персонажей есть все клипы по имени
- [x] тест: у моделей разных наборов разные `Handle<Texture>`; молчаливая поломка в «Игры» `docs/design.md`: одинаковый URI атласа у наборов в одном каталоге даёт всем одну текстуру
- [x] тест габаритов: после масштаба дерево и камень помещаются в клетку, персонаж ~1,8 м стоит на полу
- [x] `Shooter: Game` с пустой сценой; `make shooter` запускается
- [x] `cargo mutants -f games/shooter/src/main.rs`, `make cov`, `make check`

### Task 4: Карта уровня
**Files:**
- Create: `games/shooter/src/map.rs`, `games/shooter/assets/levels/level01.txt`
- [x] тесты разбора: все символы легенды, ошибки со строкой, взгляд игрока на открытого соседа
- [x] `Map::parse`
- [x] уровень: стартовая поляна, коридоры из деревьев, дверь в заборе перед выходом, все три типа врагов, патроны и аптечки на пути
- [x] `cargo mutants -f games/shooter/src/map.rs`, `make cov`, `make check`

### Task 5: Препятствия, лучи, путь
**Files:**
- Create: `games/shooter/src/obstacle.rs`
- Modify: `docs/design.md`
- [x] тесты `ray`: стоп на твёрдой клетке и закрытой двери, проход через открытую, луч точно в угол клеток не проскакивает между ними
- [x] тесты: `segment` ловит тонкую стену при большом шаге; BFS обходит закрытую дверь и находит путь после открытия
- [x] `Obstacles`, DDA, BFS
- [x] молчаливая поломка в «Игры» `docs/design.md`: дверь вне общего запроса пропускает выстрел и взгляд → тесты
- [x] `cargo mutants -f games/shooter/src/obstacle.rs`, `make cov`, `make check`

### Task 6: Игрок
**Files:**
- Create: `games/shooter/src/player.rs`
- [x] тесты: ход и скольжение вдоль стены, закрытая дверь держит, мышь потребляется в `update`, остаток — в `draw`, наклон ограничен, урон и смерть
- [x] перенос из `maze` с правками; захват курсора кликом, Esc отпускает
- [x] `cargo mutants -f games/shooter/src/player.rs`, `make cov`, `make check`

### Task 7: Оружие и снаряды
**Files:**
- Create: `games/shooter/src/weapon.rs`
- [x] тесты hitscan: ближайшая `Target` ближе препятствия; за стеной и дверью — промах; скорострельность; патроны кончаются
- [x] тесты снаряда: летит, бьёт цель, гаснет о стену, не проходит тонкую стену; снаряд врага пролетает сквозь врагов
- [x] тест: 1 и 2 меняют оружие, отдача возвращает модель в покой
- [x] `Target`, `Weapon`, `Projectile`
- [x] `cargo mutants -f games/shooter/src/weapon.rs`, `make cov`, `make check`

### Task 8: Враги
**Files:**
- Create: `games/shooter/src/enemy.rs`
- [ ] тесты автомата: спит, пока не видит игрока или не ранен; закрытая дверь закрывает взгляд и держит ход; погоня по пути; атака на дистанции типа; смерть
- [ ] тесты поведения: ближний бой бьёт вплотную, стрелок держит дистанцию и стреляет, быстрый догоняет игрока
- [ ] тест клипов: состояние → клип, `die` останавливается на последнем ключе, бластер стрелка — по `globals[arm-right]`
- [ ] `Enemy`, три типа, урон игроку
- [ ] `cargo mutants -f games/shooter/src/enemy.rs`, `make cov`, `make check`

### Task 9: Ключ, двери, подбор, выход, рестарт
**Files:**
- Modify: `games/shooter/src/main.rs`
- [ ] тесты: ключ берётся касанием один раз; дверь открывается ключом и твёрдая до конца анимации; аптечка не лечит выше максимума; патроны прибавляются
- [ ] тесты: выход — победа, Enter рестартит; смерть — экран, Enter рестартит с полным уровнем
- [ ] тик игры: игрок, оружие, снаряды, враги, подбор, двери
- [ ] `cargo mutants -f games/shooter/src/main.rs`, `make cov`, `make check`

### Task 10: Сцена, слой оружия, вспышка, HUD
**Files:**
- Modify: `games/shooter/src/main.rs`, `docs/design.md`
- [ ] тесты сцены: модель на каждой клетке карты, земля, враги в позе клипа между тиками, снаряды; `viewmodel` с текущим бластером, отдачей и `fov_y`
- [ ] тест: вспышка держится по таймеру, а не тик; молчаливая поломка «вспышка из одного тика не попадает в кадр → тест»
- [ ] тест HUD: здоровье, патроны, ключ, прицел по центру, текст победы и смерти
- [ ] `cargo mutants -f games/shooter/src/main.rs`, `make cov`, `make check`

### Task 11: Звук игры
**Files:**
- Create: `games/shooter/src/sound.rs`, `games/shooter/assets/sounds/`
- Modify: `games/shooter/assets/CREDITS.md`, `.cargo/mutants.toml`
- [ ] звуки Kenney (CC0): выстрелы, попадание, смерть, подбор, дверь; `decode` их читает; записать в `CREDITS.md`
- [ ] тесты `SoundCmd`: выстрел — одна команда на выстрел, звук врага справа даёт `right > left`, вне `range` ничего не шлёт
- [ ] `update_sound` шлёт команды в `Audio`; исключение `in Shooter::update_sound$` в `.cargo/mutants.toml` с причиной, как у racing
- [ ] `cargo mutants -f games/shooter/src/sound.rs`, `make cov`, `make check`

### Task 12: Бот и сквозные тесты
**Files:**
- Create: `games/shooter/src/bot.rs`
- [ ] бот по BFS: к ключу, двери, выходу; стреляет во врага в прямой видимости, меняет оружие, когда кончаются патроны
- [ ] сквозной тест: бот проходит уровень, все враги убиты, обоими оружиями, патроны тратятся; долгий — под `#[ignore = "..."]`
- [ ] сквозной тест: бездействие рядом с врагом — смерть и рестарт уровня
- [ ] сквозной тест: при закрытой двери враг за ней не просыпается и не проходит, выстрел и снаряд в неё не проходят
- [ ] `cargo mutants -f games/shooter/src/bot.rs`, `make test`, `make cov`, `make check`

### Task 13: Приёмка и документация
- [ ] пункты «Готово, когда»; `make mutants-diff`, `REQUIRE_GPU=1 WGPU_BACKEND=vulkan cargo test -p engine`, `make test` с долгими
- [ ] `docs/design.md`: строка 8 в «Этапы», «8+» → «9+»
- [ ] `CLAUDE.md`: `make shooter` в «Команды»
- [ ] `games/shooter/README.md`: управление, легенда карты
- [ ] план — в `docs/plans/completed/`

## Post-Completion
- Запуск `make shooter`: оружие не дёргается и не уходит в деревья, враги различимы по поведению, звук слышен с правильной стороны в наушниках, вспышка видна.
- На моноустройстве панорамы нет: `fill` сводит каналы.
