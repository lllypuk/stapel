# Этап 4: инструменты платформера — мышь, egui, горячая перезагрузка, редактор уровней

## Overview
- Движок: мышь в `Input`, перевод курсора в мир, egui поверх кадра (`Game::ui`), горячая перезагрузка текстур и звуков.
- Игра `games/platformer`: несколько уровней из `assets/levels/`, переход к следующему после выхода, перезагрузка уровня при изменении файла, редактор по F1 — палитра, кисть ЛКМ/ластик ПКМ, скролл, размер уровня, Ctrl+Z, Ctrl+S, список уровней и новый уровень.
- Готово, когда: уровень рисуется и сохраняется в редакторе, F1 сразу играет правку; правка PNG/WAV/уровня снаружи видна без перезапуска; `make check`, `make cov`, `make mutants` и CI зелёные.

## Context (from discovery)
- `egui`, `egui-winit`, `egui-wgpu` 0.36.2 требуют `wgpu ^30`, `winit ^0.30.13` — как у нас (`Cargo.toml:10`). AccessKit (MIT OR Apache-2.0) — обязательная зависимость egui. Фича `default_fonts` тянет `epaint_default_fonts` (OFL-1.1, UFL-1.0 — вне `deny.toml:9`); `egui-winit` по умолчанию — буфер обмена и ссылки, `egui-wgpu` — `wgpu/webgl`, `wgpu/metal`.
- `egui_kittest` 0.36.2 без фич — dev-dep для кликов по UI.
- `Input` — только клавиатура; `focus_lost` чистит лишь `down` (`engine/src/input.rs:35`), быстрое нажатие между тиками сохраняет `just_pressed` без `down` (`engine/src/input.rs:88`).
- `Renderer::render` льёт текстуры до проверки поверхности (`engine/src/renderer.rs:118`) и выходит без кадра при недоступной поверхности и `Reconfigure`/`Recreate` (`:120`, `:132`).
- `SpritePipeline::upload` всегда делает `push` с assert строгого порядка (`engine/src/sprite_pipeline.rs:113`, `:150`); тестовый `load_png` удаляет файл после загрузки (`:333`).
- `Assets` — кэш путь→хэндл без версии файла (`engine/src/assets.rs:103`); `Sound { frames: Arc<[f32]> }` (`engine/src/audio/mod.rs:18`), голос держит клон.
- Платформер: уровень вшит `include_str!` (`games/platformer/src/main.rs:67`), битая карта — паника (`:959`); `Level::parse` сворачивает `=`/`H` в платформы. Тесты опираются на содержимое `level1` (`:1112`, `:1163`, `:1196`, стартовые клетки случайного прогона `:2571`). Enter после выхода — `restart` в чистом `tick` (`:728`).
- Покрытие исключает `renderer.rs` и `app.rs` (`Makefile:30`), мутанты — только `Renderer::` (`.cargo/mutants.toml:4`).
- Решения согласованы с codex (тред `stage4-tools`, два раунда), план проверен агентом plan-review.

## Development Approach
- **testing approach**: TDD — тест раньше кода для всего, что не требует окна и GPU; тест сначала прогоняется на мутации.
- Следующая задача — только при зелёном `make check`.
- Решение, меняющее `docs/design.md` или `CLAUDE.md`, и новая молчаливая поломка с пометкой «→ что ловит» — в той же задаче.
- Изменения скоупа — сразу в этот файл: ➕ новая задача, ⚠️ блокер.

## Testing Strategy
- Юнит-тесты в `#[cfg(test)]` того же файла: мышь, экран→мир, маршрутизация событий, перезагрузка в `Assets`, сетка, история, команды и режимы редактора, список и смена уровней, наблюдатель уровня.
- Тесты игры не трогают `assets/levels/`: карта для них — замороженная копия `src/testdata/level01.txt` через `include_str!`, файловые тесты — во временном каталоге (`std::env::temp_dir()` + pid + имя теста). Версия файла в тестах — `File::set_modified`, не пауза: mtime на Linux грубый.
- Панель egui — `egui_kittest`: клик → `EditorCmd`; эффекты команд — отдельными тестами без UI.
- Замена текстуры по хэндлу — офскрин-тест `SpritePipeline` (`REQUIRE_GPU=1`).
- egui-склейка — только в `impl Renderer` и `app.rs` (вне покрытия и мутантов); вид панели и фокус — запуск.

## Solution Overview
- **Один потребитель фронтов — `update`.** Кисть, F1, Ctrl+S, Ctrl+Z, скролл — по снимку `Input` в тике. `Game::ui(&mut self, ui: &egui::Context)` (пустая по умолчанию, раз в кадр после `draw`) строит панель и кладёт `EditorCmd` в очередь игры; очередь разбирает следующий `update`.
- **Маршрутизация.** egui получает все события окна. Нажатие кнопки мыши идёт в `Input`, только если `egui_wants_pointer_input` ложно на момент нажатия; отпускание — всегда: без принятого нажатия оно ничего не меняет, с ним снимает `down`. Клавиатура — всегда в `Input`; после прохода egui фокус снимается: у панели нет текстовых полей, а сфокусированная кнопка нажималась бы Space/Enter игры. Текстовое поле — пересмотреть правило.
- **Курсор в мир** — по камере и вьюпорту последнего *показанного* кадра: app публикует их после `render → Ok(true)`, после resize — `None` до следующего показа.
- **egui-проход** — отдельный pass после спрайтов, `LoadOp::Load`, на всю поверхность, в не-sRGB вид поверхности (`view_formats`): egui смешивает в гамме и на `*Srgb` пишет warn при каждом старте. `textures_delta.set` применяется до проверки поверхности, как `sprites.upload`: egui шлёт атлас шрифта один раз. `free` — после submit, на пропущенном кадре — сразу. Шрифт — `default_fonts` выключен, игра ставит `JetBrainsMono` (`set_fonts` при первом `ui`).
- **Перезагрузка ассетов.** `Assets` хранит `Stamp { mtime, len }` текстур и звуков; app раз в 0,5 с (по `Instant`) зовёт `reload_changed`. `Stamp` обновляется и при ошибке декода: warn один раз на версию файла. Текстура — `PendingTexture` с прежним хэндлом, `upload` заменяет bind group при `index < len`. Звук — новый `Arc` в слот, старый `Sound` в `retired` навсегда: иначе голос стал бы последним владельцем и освободил память в колбэке. Играющая музыка остаётся старой. Ошибка — старый ресурс. Шрифты не перезагружаются. Пропуск изменения с тем же `Stamp` принят: дев-инструмент.
- **Уровни** — `assets/levels/levelNN.txt` (две цифры: порядок по имени). Каталог — поле `Platformer`, не константа. Наблюдатель читает файл раз в 30 тиков и сравнивает текст с базой (последний загруженный или сохранённый): свой Ctrl+S и правка с тем же mtime не путаются.
- **Редактор** правит `Grid`, не `Level`: сохранение без потерь, невалидная карта видна. `Grid::parse` не падает: строки дополняются `.` до самой длинной. Уровень, который не разбирается (при старте, переходе, перезагрузке), открывается в редакторе с ошибкой в панели; F1 в игру не уходит, пока `Level::parse` не прошёл.
- **Грязный редактор** (`dirty`) запрещает смену и создание уровня — панель говорит «сохраните или отмените»; иначе правки терялись бы молча.

## Technical Details

### Мышь и курсор (`engine/src/input.rs`, `camera.rs`, `frame.rs`)
- `Input`: `on_button(MouseButton, ElementState)` по схеме `on_key`; `on_cursor(Option<Vec2>)` — физические пиксели окна, `None` — курсор ушёл; `mouse_down`, `mouse_just_pressed`, `cursor`. `focus_lost` чистит `down` кнопок; `pressed` не трогает — как у клавиш.
- `screen_to_world(viewport, camera, pos) -> Option<Vec2>` — обратное к `view_proj` внутри вьюпорта (начало сверху слева, Y мира вверх); вне — `None`.
- `Context::shown: Option<Shown { camera, viewport }>`; `Context::cursor_world()`.

### egui (`engine/src/ui.rs`, `app.rs`, `renderer.rs`)
- `ui::to_input(event, wants_pointer) -> bool` — чистое правило маршрутизации.
- `pub use egui;` в `engine/src/lib.rs`: игра берёт ту же версию.
- Зависимости: `egui` без `default_fonts`; `egui-winit` `default-features = false`, `["wayland", "x11"]`; `egui-wgpu` `default-features = false`.
- `App`: `egui::Context`, `egui_winit::State`; кадр: `take_egui_input → begin_pass → step_frame → game.ui → снять фокус → end_pass → handle_platform_output → tessellate`.
- `Renderer::render(gfx, assets, egui)`: `update_texture` рядом с `sprites.upload`; `update_buffers`/`render` в pass после спрайтов; `ScreenDescriptor` — полный размер поверхности и `pixels_per_point` окна.

### Перезагрузка (`engine/src/assets.rs`, `sprite_pipeline.rs`)
- `pub(crate) Stamp::of(path) -> Option<Stamp>`; `Assets` хранит `(PathBuf, Stamp, Handle)` текстур и звуков; `reload_changed()` без возврата.
- `take_pending` — FIFO: `[0, 1, 1]` создаёт слот 1, потом заменяет.

### Платформер
- Модули: `level.rs` (`Level`, разбор), `levels.rs` (список, наблюдатель), `editor.rs` (`Grid`, `History`, `Editor`, `EditorCmd`, рисование), `panel.rs` (egui), `main.rs` (игра).
- `tick` остаётся чистым: Enter после `X` ставит флаг перехода, `update` читает следующий файл.
- `Grid { size, cells: Vec<char> }`: `parse` (дополнение `.`), `to_text` (каждая строка с `\n`), `get/set(cell, ch)` (второй `P` стирает первый), `add/remove_column` справа, `add/remove_row` сверху, минимум 1×1. Палитра: `. # P E o ^ X ! = H`; неизвестный символ рисуется маркером.
- `History`: в историю кладётся состояние *до* мазка или resize; мазок без изменений снимка не даёт; лимит 100; `undo`; `dirty = grid != saved`; смена уровня и загрузка с диска очищают историю.
- Мазок: начинается по `mouse_just_pressed`, рисует клетку под курсором каждый тик, кончается, когда кнопка не `down` (клик между тиками — одна клетка) или по F1.
- Скролл камеры редактора — WASD/стрелки по обеим осям, в пределах уровня с полем в экран.
- Сохранение: временный файл рядом и `rename`; ошибка — текст в панели, `dirty` остаётся; база наблюдателя := сохранённый текст.
- Внешнее изменение: чистый редактор — новая сетка; грязный — флаг «файл изменён на диске» до «Загрузить с диска» или Ctrl+S. В игре — уровень заново; не разбирается — редактор с ошибкой. Файл пропал — warn один раз.
- Новый уровень: `levelNN.txt` со следующим свободным NN, шаблон 20×10 с полом и `P`, список обновляется, редактор открывает его.
- Пустой или нечитаемый каталог при старте — паника с путём: уровни лежат в репозитории.

## What Goes Where
- **Implementation Steps** — код, тесты, документация в репозитории.
- **Post-Completion** — проверки запуском.

## Implementation Steps

### Task 1: Мышь в `Input`

**Files:**
- Modify: `engine/src/input.rs`, `engine/src/lib.rs`, `engine/src/app.rs`

- [x] тесты: нажатие кнопки — один тик `just_pressed`; клик между тиками — `just_pressed` без `down`; отпускание без нажатия — без изменений; `focus_lost` снимает `down` кнопок; курсор `None` при уходе
- [x] `on_button`, `on_cursor`, запросы; реэкспорт `MouseButton`
- [x] `app.rs`: `MouseInput`, `CursorMoved`, `CursorLeft` → `Input` (пока без egui)
- [x] `make check`

### Task 2: Курсор в мир по показанному кадру

**Files:**
- Modify: `engine/src/camera.rs`, `engine/src/frame.rs`, `engine/src/renderer.rs`, `engine/src/app.rs`

- [x] тесты `screen_to_world`: центр вьюпорта — центр камеры; Y вверх; поля `letterbox` и `letterbox_pixel` — `None`; угол вьюпорта — край камеры
- [x] тесты `cursor_world`: без `shown` или курсора — `None`
- [x] `screen_to_world`, `Shown`, `Context::cursor_world`; `Renderer::render` отдаёт вьюпорт показанного кадра; app ставит `shown` после `Ok(true)`, `None` после resize
- [x] `docs/design.md`: «курсор — по камере показанного кадра, не `gfx.camera`» → тест; порядок в app — ревью
- [x] `make check`

### Task 3: egui поверх кадра и `Game::ui`

**Files:**
- Modify: `Cargo.toml`, `engine/Cargo.toml`, `deny.toml`, `engine/src/lib.rs`, `engine/src/frame.rs`, `engine/src/app.rs`, `engine/src/renderer.rs`
- Create: `engine/src/ui.rs`

- [x] зависимости с фичами из Technical Details в `[workspace.dependencies]`, `pub use egui`; `cargo deny check` — новые лицензии в `deny.toml` осознанно (новых нет, `deny.toml` не менялся)
- [x] тесты `ui::to_input`: нажатие мыши при `wants_pointer` — нет; отпускание — да; клавиши — всегда да (`KeyEvent` вне winit не собрать — прочие события: курсор, уход, фокус)
- [x] `ui::to_input`; `Game::ui` с пустой реализацией; склейка в `app.rs`, снятие фокуса после `ui`
- [x] `Renderer::render(gfx, assets, UiFrame)` — `UiFrame` по значению: неочищенный `TexturesDelta` паникует при drop; не-sRGB `view_formats`, `update_texture` до проверки поверхности, egui-pass с `LoadOp::Load`, `free` после submit или сразу на пропущенном кадре — всё внутри `impl Renderer`
- [x] `docs/design.md`: egui — в «Готовое»; поломки «отпускание всегда в `Input`» → тест; «атлас egui теряется на непоказанном кадре» → ревью; «egui-pass с `Clear` стирает игру» → запуск; «сфокусированная кнопка ловит Space/Enter» → запуск
- [x] `make check`; запуск pong и platformer — кадр не изменился, warn egui о framebuffer нет

### Task 4: Горячая перезагрузка текстур

**Files:**
- Modify: `engine/src/assets.rs`, `engine/src/sprite_pipeline.rs`, `engine/src/app.rs`

- [x] тесты `Assets`: изменённый файл → `PendingTexture` с прежним хэндлом; без изменений — пусто; битый файл — без pending, повторный опрос без нового warn; заглушка по отсутствующему пути заменяется, когда файл появился
- [x] офскрин-тест: пачка `[0, 1, 1]` — слот 1 заменён, рисуется новая (файл живёт до перезагрузки, не `load_png`)
- [x] `Stamp`, `reload_changed`; `upload` с ветками push/замена; app — опрос раз в 0,5 с
- [x] `docs/design.md`: замена ресурса того же пути — не переиспользование слота, поколение не нужно; «замена без push-ветки» → офскрин-тест
- [x] `make check`

### Task 5: Горячая перезагрузка звуков

**Files:**
- Modify: `engine/src/assets.rs`

- [x] тесты: изменённый WAV → новые `frames` под прежним хэндлом; старый `Arc` жив в `retired` (`Arc::strong_count`); ошибка декода — старый звук
- [x] перезагрузка звуков в `reload_changed`, `retired`
- [x] `docs/design.md`: «будущее: выгрузка звуков» → «перезагрузка: старый звук в `retired`» → тест; «играющая музыка остаётся старой»
- [x] `make check`

### Task 6: Платформер — модули, каталог уровней, переход

**Files:**
- Create: `games/platformer/src/level.rs`, `games/platformer/src/levels.rs`, `games/platformer/src/testdata/level01.txt`
- Modify: `games/platformer/src/main.rs`; Move: `assets/level1.txt` → `assets/levels/level01.txt`

- [x] перенос `Level` и тестов в `level.rs` без изменений поведения; тесты игры — на `testdata/level01.txt`
- [x] тесты `levels`: список `*.txt` по имени, прочее пропускается; следующий после последнего — первый
- [x] тесты игры: Enter после `X` ставит флаг, `update` грузит следующий уровень из временного каталога; следующий не разбирается — редактор с ошибкой (до Task 9 — `warn` и текущий уровень заново; ➕ пересмотреть в Task 9); правка тестов Enter и случайного прогона
- [x] `Platformer { levels_dir, .. }`, чтение с диска вместо `include_str!`
- [x] `make check`

### Task 7: Перезагрузка уровня в игре

**Files:**
- Modify: `games/platformer/src/levels.rs`, `games/platformer/src/main.rs`

- [x] тесты наблюдателя: текст не изменился — ничего; изменился — новый текст один раз; тот же текст после своего сохранения — ничего; файл пропал — `warn` один раз
- [x] тесты игры: изменение — уровень заново с новой картой; не разбирается — старый уровень (редактор — в Task 9)
- [x] наблюдатель раз в 30 тиков
- [x] `docs/design.md`: «изменение уровня — по содержимому, не по mtime» → тест
- [x] `make check`

### Task 8: Сетка и история

**Files:**
- Create: `games/platformer/src/editor.rs`

- [x] тесты `Grid`: `parse`/`to_text` туда-обратно для `level01`; неровные строки дополняются `.`; второй `P` стирает первый; ± столбец справа и строка сверху; минимум 1×1; вне сетки — без изменений
- [x] тесты `History`: в истории состояние до мазка; мазок без изменений — без снимка; undo мазка и resize; лимит 100; `dirty` после правки, снят undo до сохранённого и сохранением
- [x] `Grid`, `History`
- [x] `make check`

### Task 9: Режим редактора и кисть

**Files:**
- Modify: `games/platformer/src/editor.rs`, `games/platformer/src/main.rs`

- [x] тесты: F1 — мир стоит, камера свободна; ЛКМ рисует символ палитры по клетке `cursor_world`, ПКМ — `.`; клик между тиками — одна клетка и один снимок; мазок кончается по `!down` и по F1; скролл WASD/стрелками с границами
- [x] тесты: F1 с неразбираемой сеткой — остаёмся, ошибка в состоянии редактора; с разбираемой — игра с правкой
- [x] тесты: битый уровень при старте, при переходе и при перезагрузке — редактор с ошибкой (закрывает ➕ из Task 6, 7)
- [x] реализация
- [x] `docs/design.md`: «мазок кончается по `!down`, не по `just_released`» → тест
- [x] `make check`

### Task 10: Сохранение, отмена, внешние изменения

**Files:**
- Modify: `games/platformer/src/editor.rs`, `games/platformer/src/main.rs`

- [x] тесты: Ctrl+S — атомарная запись, `dirty` снят, база наблюдателя обновлена; ошибка записи — текст ошибки, `dirty` остаётся; Ctrl+Z
- [x] тесты: внешнее изменение — чистый редактор перечитывает; грязный ставит флаг, флаг держится до `EditorCmd::LoadFromDisk` или Ctrl+S
- [x] реализация
- [x] `docs/design.md`: «внешнее изменение при `dirty` не теряется» → тест
- [x] `make check`

### Task 11: Рисование редактора

**Files:**
- Modify: `games/platformer/src/editor.rs`

- [x] тесты: символ → спрайт для всей палитры; неизвестный символ — маркер; подсветка клетки под курсором; рамка уровня
- [x] реализация
- [x] `make check`

### Task 12: Эффекты команд редактора

**Files:**
- Modify: `games/platformer/src/editor.rs`, `games/platformer/src/main.rs`

- [x] тесты `EditorCmd`: `Pick(ch)`; `Open(level)` — история очищена, редактор чистый; `New` — `levelNN.txt` следующего NN с шаблоном, список обновлён, открыт; `Resize(..)` — через историю; `Save`; `Undo`; `LoadFromDisk`
- [x] тесты: `Open` и `New` при `dirty` — отказ с текстом «сохраните или отмените»
- [x] очередь в `update`
- [x] `docs/design.md`: «смена уровня при `dirty` запрещена» → тест
- [x] `make check`

### Task 13: Панель на egui

**Files:**
- Create: `games/platformer/src/panel.rs`
- Modify: `games/platformer/Cargo.toml`, `games/platformer/src/main.rs`

- [ ] `egui_kittest` dev-dep; `cargo deny check`
- [ ] тесты kittest: клик по палитре, уровню, «Новый», ±столбец/строка, «Сохранить», «Отменить», «Загрузить с диска» → `EditorCmd`; ошибка разбора, ошибка записи, флаг конфликта и отказ при `dirty` показаны текстом; после клика `egui_wants_keyboard_input` ложно
- [ ] `Platformer::ui` — панель только в редакторе, `set_fonts` при первом вызове
- [ ] `make check`

### Task 14: Приёмка
- [ ] все пункты Overview
- [ ] `make check`, `make cov` (порог не ниже прежнего), `make mutants`
- [ ] CI зелёный; если понадобились системные библиотеки — `ci/Dockerfile`, тег `CI_IMAGE`, образ собран и запушен

### Task 15: Документация
- [ ] `docs/design.md`: строка этапа 4 в «Этапы», прочее — в «5+»
- [ ] `CLAUDE.md`: `Game::ui` и egui-pass в описании кадра, `ui` в «Границе тестируемости», мышь и `cursor_world`, перезагрузка ассетов, F1 в платформере
- [ ] `make check`
- [ ] план — в `docs/plans/completed/`

## Post-Completion
- Запуск: панель читаема; клик по панели не рисует в мире; клавиша, отпущенная над панелью, не залипает; после клика по кнопке Space прыгает, а не жмёт кнопку.
- Правка PNG и WAV при запущенной игре; правка уровня в текстовом редакторе.
- Полный проход уровней подряд.
